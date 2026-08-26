//! End-to-end integration tests for task 21 — Rust core unit/integration tests
//! covering the four pillars called out in the roadmap: **download**, **parse**,
//! **validate** and **resume**.
//!
//! The download / resume / validate legs run fully offline against the
//! in-memory [`MockSource`] (a crate-private test double reused from
//! `download::testing`), so the suite is deterministic in CI and on the host.
//! The parse leg exercises the real Mojang `version_manifest` / `version.json`
//! deserialisers and the `inheritsFrom` merge, and the validate leg checks
//! library rule evaluation and Maven-coordinate resolution exactly as the
//! dependency resolver uses them.

use std::collections::HashMap;
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;

use crate::download::testing::MockSource;
use crate::download::{compute_backoff, plan_chunks};
use crate::download::{hex_eq, md5_bytes, md5_path, sha1_bytes, sha1_path};
use crate::download::{DownloadManager, DownloadOptions, DownloadTask};
use crate::game::library::Library;
use crate::game::manifest::VersionManifest;
use crate::game::platform::{Arch, Features, OsName, Platform};
use crate::game::resolve::{ArtifactKind, DependencyResolver, DownloadPlan};
use crate::game::version::{merge_chain, VersionJson};
use crate::mods::resource_pack::ResourcePackManager;
use crate::mods::shader::ShaderPackManager;
use crate::mods::{ModIssueKind, ModManager};
use crate::net::MirrorProvider;

/// A unique scratch directory for an integration test.
fn scratch() -> PathBuf {
    static CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = CTR.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("rc_itest_{}_{}", std::process::id(), n));
    std::fs::create_dir_all(&p).unwrap();
    p
}

// ───────────────────────────── download + resume + validate ─────────────────

#[tokio::test]
async fn download_then_resume_preserves_checksum_integrity() {
    // (1) full chunked download of a payload that spans several chunks.
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let sha = sha1_bytes(&data);
    let dir = scratch();
    let dest = dir.join("game.jar");
    let mgr = DownloadManager::new(
        Arc::new(MockSource::new(data.clone())),
        DownloadOptions {
            chunk_size: 64 * 1024,
            concurrency: 4,
            ..Default::default()
        },
    );
    let summary = mgr
        .download(
            &DownloadTask::new("http://mock/game.jar", dest.clone())
                .with_sha1(sha.clone())
                .with_size(data.len() as u64),
        )
        .await
        .expect("first download must succeed");
    assert_eq!(summary.size, data.len() as u64);
    assert!(!summary.resumed);
    assert_eq!(std::fs::read(&dest).unwrap(), data);

    // (2) simulate an interrupted run: pre-seed .part + .meta for the first half.
    let dir2 = scratch();
    let dest2 = dir2.join("game2.jar");
    let temp2 = dir2.join("game2.jar.part");
    let meta2 = dir2.join("game2.jar.part.meta");
    let chunk_size = 64 * 1024u64;
    let plan = plan_chunks(data.len() as u64, chunk_size);
    let half = plan.len() / 2;
    {
        let f = std::fs::File::create(&temp2).unwrap();
        f.set_len(data.len() as u64).unwrap();
    }
    let mut completed: Vec<u64> = Vec::new();
    for (i, (s, e)) in plan.iter().enumerate() {
        if i < half {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open(&temp2)
                .unwrap();
            f.seek(SeekFrom::Start(*s)).unwrap();
            f.write_all(&data[*s as usize..=*e as usize]).unwrap();
            completed.push(*s / chunk_size);
        }
    }
    let json = serde_json::json!({
        "total_size": data.len() as u64,
        "chunk_size": chunk_size,
        "completed": completed,
    });
    std::fs::write(&meta2, serde_json::to_vec(&json).unwrap()).unwrap();

    let src2 = Arc::new(MockSource::new(data.clone()));
    let mgr2 = DownloadManager::new(
        src2.clone(),
        DownloadOptions {
            chunk_size,
            concurrency: 4,
            ..Default::default()
        },
    );
    let summary2 = mgr2
        .download(
            &DownloadTask::new("http://mock/game2.jar", dest2.clone())
                .with_sha1(sha1_bytes(&data))
                .with_size(data.len() as u64),
        )
        .await
        .expect("resume must succeed");
    // Only the missing chunks were fetched.
    assert_eq!(
        src2.call_count() as usize,
        plan.len() - half,
        "resume must re-fetch only the missing chunks"
    );
    assert!(summary2.resumed);
    // The fully reassembled file still verifies against the checksum.
    assert_eq!(std::fs::read(&dest2).unwrap(), data);

    // (3) validate: a wrong checksum fails and leaves the .part for a retry.
    let bad = Arc::new(MockSource::new(vec![7u8; 1024]));
    let mgr3 = DownloadManager::new(
        bad,
        DownloadOptions {
            chunk_size: 256,
            concurrency: 2,
            ..Default::default()
        },
    );
    let dir3 = scratch();
    let dest3 = dir3.join("x.bin");
    let res = mgr3
        .download(
            &DownloadTask::new("http://mock/x.bin", dest3.clone())
                .with_sha1("deadbeef")
                .with_size(1024),
        )
        .await;
    assert!(res.is_err(), "checksum mismatch must fail the download");
    assert!(
        dir3.join("x.bin.part").exists(),
        "temp must survive for a later resume/retry"
    );
}

#[test]
fn download_planning_helpers_are_invariant() {
    // plan_chunks: exact, remainder, zero and single-chunk edge cases.
    assert_eq!(plan_chunks(100, 10).len(), 10);
    assert_eq!(plan_chunks(105, 10).len(), 11);
    assert_eq!(plan_chunks(105, 10)[10], (100, 104));
    assert!(plan_chunks(0, 10).is_empty());
    assert!(plan_chunks(100, 0).is_empty());
    assert_eq!(plan_chunks(5, 10), vec![(0, 4)]); // single partial chunk

    // compute_backoff: base * 2^(n-1), capped at max, (jitter 0 here).
    let base = std::time::Duration::from_millis(100);
    let max = std::time::Duration::from_millis(500);
    assert_eq!(compute_backoff(1, base, max, 0.0), base);
    assert_eq!(
        compute_backoff(2, base, max, 0.0),
        std::time::Duration::from_millis(200)
    );
    assert_eq!(
        compute_backoff(3, base, max, 0.0),
        std::time::Duration::from_millis(400)
    );
    assert_eq!(compute_backoff(10, base, max, 0.0), max);
}

// ───────────────────────────── parse ─────────────────────────────

#[test]
fn parse_version_manifest_and_query() {
    let json = r#"{
        "latest": { "release": "1.20.4", "snapshot": "24w03a" },
        "versions": [
            { "id": "1.20.4", "type": "release", "url": "https://x/1.20.4.json", "sha1": "abc" },
            { "id": "24w03a", "type": "snapshot", "url": "https://x/24w03a.json" }
        ]
    }"#;
    let m: VersionManifest = serde_json::from_str(json).unwrap();
    assert_eq!(m.latest.release, "1.20.4");
    assert_eq!(m.find("1.20.4").unwrap().kind, "release");
    assert_eq!(m.latest_release().unwrap().id, "1.20.4");
    assert_eq!(m.latest_snapshot().unwrap().id, "24w03a");
    assert_eq!(m.url_of("24w03a"), Some("https://x/24w03a.json"));
    assert!(m.find("does-not-exist").is_none());
}

#[test]
fn parse_version_json_and_merge_inheritance_chain() {
    let parent = r#"{
        "id": "1.20.4",
        "mainClass": "net.minecraft.client.main.Main",
        "assetIndex": { "id": "1.20", "sha1": "idxsha", "size": 123, "url": "https://x/index.json" },
        "downloads": { "client": { "url": "https://x/client.jar", "sha1": "clientsha", "size": 12345 } },
        "libraries": [ { "name": "com.mojang:patchy:1.1", "downloads": { "artifact": { "sha1": "libsha", "size": 678 } } } ]
    }"#;
    let child = r#"{
        "id": "myforge",
        "inheritsFrom": "1.20.4",
        "mainClass": "net.minecraftforge.fml.common.launcher.FMLTweaker",
        "libraries": [ { "name": "net.minecraftforge:forge:1.20.4-1.0" } ]
    }"#;
    let p: VersionJson = VersionJson::parse(parent).unwrap();
    let c: VersionJson = VersionJson::parse(child).unwrap();
    assert_eq!(
        p.main_class.as_deref(),
        Some("net.minecraft.client.main.Main")
    );
    assert_eq!(p.asset_index.as_ref().unwrap().id, "1.20");

    // child overrides the main class; libraries are de-duplicated by coordinate.
    let merged = merge_chain(&[p, c]);
    assert_eq!(merged.id, "myforge");
    assert_eq!(
        merged.main_class.as_deref(),
        Some("net.minecraftforge.fml.common.launcher.FMLTweaker")
    );
    assert_eq!(
        merged.libraries.len(),
        2,
        "distinct libraries must not collide"
    );
    assert_eq!(merged.asset_index.as_ref().unwrap().id, "1.20");
}

// ───────────────────────────── validate ─────────────────────────────

#[test]
fn library_rule_validation_per_platform() {
    let linux_only: Library = serde_json::from_str(
        r#"{
        "name": "org.lwjgl:lwjgl:3.3.1",
        "rules": [ { "action": "allow", "os": { "name": "linux" } } ]
    }"#,
    )
    .unwrap();
    let android = Platform::android(); // Linux / AArch64
    let windows = Platform {
        os: OsName::Windows,
        arch: Arch::X86_64,
        os_version: String::new(),
    };
    let features: Features = HashMap::new();
    assert!(
        linux_only.is_allowed(&android, &features),
        "a linux-only rule must allow on Android (Linux)"
    );
    assert!(
        !linux_only.is_allowed(&windows, &features),
        "a linux-only rule must deny on Windows"
    );

    // Last-matching-rule wins: allow-all then deny-windows.
    let not_windows: Library = serde_json::from_str(
        r#"{
        "name": "a:b:1",
        "rules": [
            { "action": "allow" },
            { "action": "disallow", "os": { "name": "windows" } }
        ]
    }"#,
    )
    .unwrap();
    assert!(not_windows.is_allowed(&android, &features));
    assert!(!not_windows.is_allowed(&windows, &features));
}

#[test]
fn library_maven_coordinate_parse_and_url() {
    let lib: Library = serde_json::from_str(r#"{ "name": "com.mojang:patchy:1.1" }"#).unwrap();
    let (g, a, v, cls, ext) = lib.parse_maven();
    assert_eq!(
        (g.as_str(), a.as_str(), v.as_str(), cls, ext.as_str()),
        ("com.mojang", "patchy", "1.1", None, "jar")
    );
    assert_eq!(lib.maven_path(None), "com/mojang/patchy/1.1/patchy-1.1.jar");
    assert_eq!(
        lib.artifact_url().unwrap(),
        "https://libraries.minecraft.net/com/mojang/patchy/1.1/patchy-1.1.jar"
    );

    // A classifier-only coordinate has no main jar to download.
    let native: Library =
        serde_json::from_str(r#"{ "name": "ca.weblite:java-objc-bridge:1.0.0:natives-osx" }"#)
            .unwrap();
    let (g2, a2, v2, c2, e2) = native.parse_maven();
    assert_eq!(
        (
            g2.as_str(),
            a2.as_str(),
            v2.as_str(),
            c2.as_deref(),
            e2.as_str()
        ),
        (
            "ca.weblite",
            "java-objc-bridge",
            "1.0.0",
            Some("natives-osx"),
            "jar"
        )
    );
    assert!(
        native.artifact_url().is_none(),
        "classifier-only lib has no main jar"
    );
}

#[tokio::test]
async fn checksum_hash_helpers_roundtrip_and_case_insensitive() {
    let dir = scratch();
    let p = dir.join("f.bin");
    let data: Vec<u8> = (1u8..=200).collect();
    std::fs::write(&p, &data).unwrap();

    assert_eq!(
        sha1_bytes(b"abc"),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
    assert_eq!(md5_bytes(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    assert_eq!(sha1_path(&p).await.unwrap(), sha1_bytes(&data));
    assert_eq!(md5_path(&p).await.unwrap(), md5_bytes(&data));
    assert!(hex_eq("ABCDEF", "abcdef"));
    assert!(!hex_eq("abcdef", "abcdeg"));
}

// ───────────────────────────── integration: parse → plan → validate ─────────

#[test]
fn resolve_version_into_deduplicated_download_plan() {
    let parent = r#"{
        "id": "1.20.4",
        "mainClass": "net.minecraft.client.main.Main",
        "assetIndex": { "id": "1.20", "sha1": "idxsha", "size": 123, "url": "https://x/index.json" },
        "downloads": { "client": { "url": "https://x/client.jar", "sha1": "clientsha", "size": 12345 } },
        "libraries": [ { "name": "com.mojang:patchy:1.1", "downloads": { "artifact": { "sha1": "libsha", "size": 678 } } } ]
    }"#;
    let child = r#"{
        "id": "myforge",
        "inheritsFrom": "1.20.4",
        "libraries": [ { "name": "net.minecraftforge:forge:1.20.4-1.0" } ]
    }"#;
    let resolved = merge_chain(&[
        VersionJson::parse(parent).unwrap(),
        VersionJson::parse(child).unwrap(),
    ]);

    let resolver = DependencyResolver::new(
        Platform::android(),
        Arc::new(MirrorProvider::new_default()),
        scratch(),
    );
    let plan: DownloadPlan = resolver.build_plan(&resolved);

    // client + asset index + two libraries (patchy + forge).
    assert_eq!(plan.count_kind(ArtifactKind::Client), 1);
    assert_eq!(plan.count_kind(ArtifactKind::AssetIndex), 1);
    assert_eq!(plan.count_kind(ArtifactKind::Library), 2);
    // The plan must be deduplicated by destination.
    assert_eq!(plan.len(), 4);
    // Known sizes must aggregate (12345 + 123 + 678 + 0).
    assert_eq!(plan.total_bytes(), 13146);
    // Converting to tasks preserves the structure the manager consumes.
    let tasks = plan.into_tasks();
    assert_eq!(tasks.len(), 4);
    assert!(tasks.iter().any(|t| t.checksum.is_some()));
}

// ───────────────────── mods / resource-pack / shader (task 8) ─────────────────────
//
// End-to-end coverage for the per-instance Mod / resource-pack / shader managers
// (task 8). Mirrors FCL's `ModManager` pre-launch validation: install archives,
// scan them into typed records, resolve dependency / conflict / MC-version issues,
// and toggle enable-disable durably via the `.disabled` name suffix.

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

/// Helper: write a `.jar` / `.zip` archive with the given (name, content) entries.
fn write_archive(path: &std::path::Path, entries: &[(&str, &str)]) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, content) in entries {
        zip.start_file(*name, opts).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn mods_install_scan_resolve_missing_dependency_e2e() {
    let base = scratch();
    let mgr = ModManager::new(base.join("mods"));

    // Fabric mod "sodium" depends on mod "iris" (any version).
    let sodium_src = base.join("sodium.jar");
    write_archive(
        &sodium_src,
        &[(
            "fabric.mod.json",
            r#"{"schemaVersion":1,"id":"sodium","version":"0.5","name":"Sodium","depends":{"minecraft":">=1.16","iris":"*"}}"#,
        )],
    );
    let installed = mgr.install(&sodium_src).unwrap();
    assert_eq!(installed.primary().unwrap().modid, "sodium");
    assert!(installed.is_enabled());

    // First resolve: iris is missing.
    let issues = mgr.resolve(Some("1.18.2")).unwrap();
    assert!(
        issues
            .iter()
            .any(|i| i.kind == ModIssueKind::MissingDependency
                && i.target.as_deref() == Some("iris")),
        "expected a missing-dependency issue for iris, got {issues:?}"
    );

    // Install iris (satisfies the dependency) and re-resolve.
    let iris_src = base.join("iris.jar");
    write_archive(
        &iris_src,
        &[(
            "fabric.mod.json",
            r#"{"schemaVersion":1,"id":"iris","version":"1.6","name":"Iris","depends":{"minecraft":">=1.16"}}"#,
        )],
    );
    let iris = mgr.install(&iris_src).unwrap();
    assert_eq!(iris.primary().unwrap().modid, "iris");

    let issues = mgr.resolve(Some("1.18.2")).unwrap();
    assert!(
        !issues
            .iter()
            .any(|i| i.kind == ModIssueKind::MissingDependency),
        "loadout should be clean after installing iris, got {issues:?}"
    );

    // Disabling sodium removes it from the resolvable set (durable state).
    let disabled = mgr.set_enabled(&installed, false).unwrap();
    assert!(!disabled.is_enabled());
    let disabled_name = disabled.path.file_name().unwrap().to_str().unwrap();
    assert!(disabled_name.ends_with(".disabled"));
    // iris alone declares no hard dep on sodium, so still clean.
    let issues = mgr.resolve(Some("1.18.2")).unwrap();
    assert!(!issues
        .iter()
        .any(|i| i.kind == ModIssueKind::MissingDependency));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn mods_conflict_duplicate_and_version_isolation_e2e() {
    let base = scratch();
    // Two independent instances → version isolation.
    let mgr_a = ModManager::new(base.join("instance_a").join("mods"));
    let mgr_b = ModManager::new(base.join("instance_b").join("mods"));

    // Mod "a" breaks mod "b"; mod "b" conflicts with "a".
    let a_src = base.join("a.jar");
    write_archive(
        &a_src,
        &[(
            "fabric.mod.json",
            r#"{"schemaVersion":1,"id":"a","version":"1","name":"A","breaks":{"b":"*"}}"#,
        )],
    );
    let b_src = base.join("b.jar");
    write_archive(
        &b_src,
        &[(
            "fabric.mod.json",
            r#"{"schemaVersion":1,"id":"b","version":"1","name":"B","conflicts":{"a":"*"}}"#,
        )],
    );
    mgr_a.install(&a_src).unwrap();
    mgr_a.install(&b_src).unwrap();

    // Instance B is empty → version isolation means no issues there.
    let issues_b = mgr_b.resolve(Some("1.18.2")).unwrap();
    assert!(
        issues_b.is_empty(),
        "empty instance must be clean, got {issues_b:?}"
    );

    let issues_a = mgr_a.resolve(Some("1.18.2")).unwrap();
    assert!(
        issues_a.iter().any(|i| i.kind == ModIssueKind::Conflict),
        "expected a conflict between a and b, got {issues_a:?}"
    );

    // Two files declaring the same mod id → DuplicateMod.
    let dup1 = base.join("dup1.jar");
    let dup2 = base.join("dup2.jar");
    write_archive(
        &dup1,
        &[(
            "fabric.mod.json",
            r#"{"schemaVersion":1,"id":"dup","version":"1","name":"Dup1"}"#,
        )],
    );
    write_archive(
        &dup2,
        &[(
            "fabric.mod.json",
            r#"{"schemaVersion":1,"id":"dup","version":"2","name":"Dup2"}"#,
        )],
    );
    mgr_a.install(&dup1).unwrap();
    mgr_a.install(&dup2).unwrap();
    let issues_a = mgr_a.resolve(Some("1.18.2")).unwrap();
    assert!(
        issues_a
            .iter()
            .any(|i| i.kind == ModIssueKind::DuplicateMod),
        "expected a duplicate-mod issue, got {issues_a:?}"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn mods_incompatible_minecraft_version_e2e() {
    let base = scratch();
    let mgr = ModManager::new(base.join("mods"));

    // Mod pinned to MC >= 1.20, but the instance runs 1.16.5.
    let src = base.join("modern.jar");
    write_archive(
        &src,
        &[(
            "fabric.mod.json",
            r#"{"schemaVersion":1,"id":"modern","version":"1","name":"Modern","depends":{"minecraft":">=1.20"}}"#,
        )],
    );
    mgr.install(&src).unwrap();

    let issues = mgr.resolve(Some("1.16.5")).unwrap();
    assert!(
        issues
            .iter()
            .any(|i| i.kind == ModIssueKind::IncompatibleMinecraft),
        "expected incompatible-minecraft issue, got {issues:?}"
    );
    // On a compatible version the same loadout is clean.
    let issues = mgr.resolve(Some("1.20.4")).unwrap();
    assert!(!issues
        .iter()
        .any(|i| i.kind == ModIssueKind::IncompatibleMinecraft));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn resource_pack_install_scan_compat_e2e() {
    let base = scratch();
    let mgr = ResourcePackManager::new(base.join("resourcepacks"));

    let src = base.join("faithful.zip");
    write_archive(
        &src,
        &[(
            "pack.mcmeta",
            r#"{"pack":{"pack_format":8,"description":"Faithful"}}"#,
        )],
    );
    let pack = mgr.install(&src).unwrap();
    assert!(pack.enabled);
    assert_eq!(pack.pack_format, Some(8));
    assert_eq!(pack.description.as_deref(), Some("Faithful"));

    // pack_format 8 loads on MC 1.18+, not on 1.12.2.
    assert!(pack.is_compatible("1.18.2"));
    assert!(!pack.is_compatible("1.12.2"));

    let scanned = mgr.scan().unwrap();
    assert_eq!(scanned.len(), 1);

    let disabled = mgr.set_enabled(&pack, false).unwrap();
    assert!(!disabled.enabled);
    let scanned = mgr.scan().unwrap();
    assert!(scanned.iter().all(|p| !p.enabled));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn shader_pack_install_scan_validate_e2e() {
    let base = scratch();
    let mgr = ShaderPackManager::new(base.join("shaderpacks"));

    let src = base.join("bsl.zip");
    write_archive(&src, &[("shaders/dummy.fsh", "void main(){}\n")]);
    let pack = mgr.install(&src).unwrap();
    assert!(pack.enabled);
    assert!(pack.valid);
    assert_eq!(pack.name, "bsl.zip");

    let scanned = mgr.scan().unwrap();
    assert_eq!(scanned.len(), 1);
    assert!(scanned[0].valid);

    let disabled = mgr.set_enabled(&pack, false).unwrap();
    assert!(!disabled.enabled);

    let _ = std::fs::remove_dir_all(&base);
}

/// Mirror fallback (download/validate pillar): when the primary host is dead,
/// the manager must transparently fall back to a configured mirror and still
/// assemble byte-identical content. This exercises the per-task multi-mirror
/// resilience that task 2 introduced and that task 21's download leg must
/// guarantee end-to-end (a flaky origin degrades to a working mirror instead of
/// failing the whole game download).
#[tokio::test]
async fn mirror_fallback_succeeds_when_primary_host_is_dead() {
    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let sha = sha1_bytes(&data);
    let dir = scratch();
    let dest = dir.join("game.jar");

    // Every request whose URL contains "primary" fails forever; the mirror URL
    // does not contain that substring, so it always answers.
    let src = Arc::new(MockSource::new(data.clone()));
    src.fail_url_containing("primary", usize::MAX);
    let mgr = DownloadManager::new(
        src.clone(),
        DownloadOptions {
            chunk_size: 64 * 1024,
            concurrency: 4,
            // Keep the per-retry backoff tiny so the failing primary attempts
            // don't slow the suite down.
            retry_base: std::time::Duration::from_millis(1),
            retry_max: std::time::Duration::from_millis(4),
            ..Default::default()
        },
    );

    let summary = mgr
        .download(
            &DownloadTask::new("http://primary/game.jar", dest.clone())
                .with_mirror("http://mirror/game.jar")
                .with_sha1(sha.clone())
                .with_size(data.len() as u64),
        )
        .await
        .expect("download must succeed via mirror fallback");

    assert_eq!(summary.size, data.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), data);
    // The dead primary was probed (and rejected) before the mirror answered.
    assert!(src.call_count() > 0);
}

/// Resume robustness (resume pillar): a previous run crashed leaving a `.meta`
/// that claims half the chunks are done, but the `.part` file on disk is
/// truncated (shorter than the expected total). The manager must NOT trust the
/// stale meta — it must reset the completed set and re-fetch every chunk,
/// otherwise the final checksum would see zero-filled/garbage ranges and fail
/// (the `resets_stale_meta_when_temp_truncated` discipline from task 2).
#[tokio::test]
async fn resume_resets_stale_meta_when_part_file_truncated() {
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 197) as u8).collect();
    let sha = sha1_bytes(&data);
    let dir = scratch();
    let dest = dir.join("game.jar");
    let temp = dir.join("game.jar.part");
    let meta = dir.join("game.jar.part.meta");
    let chunk_size = 64 * 1024u64;
    let plan = plan_chunks(data.len() as u64, chunk_size);

    // Truncated temp: only 10 bytes present, but the meta claims half done.
    std::fs::write(&temp, vec![0u8; 10]).unwrap();
    let half = plan.len() / 2;
    let completed: Vec<u64> = (0..half).map(|i| (i as u64) * chunk_size).collect();
    let json = serde_json::json!({
        "total_size": data.len() as u64,
        "chunk_size": chunk_size,
        "completed": completed,
    });
    std::fs::write(&meta, serde_json::to_vec(&json).unwrap()).unwrap();

    let src = Arc::new(MockSource::new(data.clone()));
    let mgr = DownloadManager::new(
        src.clone(),
        DownloadOptions {
            chunk_size,
            concurrency: 4,
            ..Default::default()
        },
    );

    let summary = mgr
        .download(
            &DownloadTask::new("http://mock/game.jar", dest.clone())
                .with_sha1(sha.clone())
                .with_size(data.len() as u64),
        )
        .await
        .expect("resume with stale meta must succeed after reset");

    // Because the temp was truncated, the manager discards the stale completed
    // set and re-fetches every chunk (none of the claimed progress is trusted).
    assert_eq!(
        src.call_count() as usize,
        plan.len(),
        "stale meta must trigger a full re-fetch of all chunks"
    );
    assert!(!summary.resumed);
    assert_eq!(std::fs::read(&dest).unwrap(), data);
}

/// Checksum validation (validate pillar): both supported digest algorithms
/// must accept the correct digest, and a file whose bytes are corrupt (but
/// claims the right size) must be rejected by the verifier. This pins the
/// SHA-1 / MD5 acceptance paths and the negative case so a regression in the
/// integrity gate fails the build rather than shipping a corrupt artifact.
#[tokio::test]
async fn validate_accepts_sha1_and_md5_and_rejects_corruption() {
    let good: Vec<u8> = (0..4096u32).map(|i| (i % 37) as u8).collect();
    let sha = sha1_bytes(&good);
    let md5 = md5_bytes(&good);
    let dir = scratch();

    // SHA-1 acceptance.
    let dest_sha = dir.join("a.bin");
    let mgr_sha = DownloadManager::new(
        Arc::new(MockSource::new(good.clone())),
        DownloadOptions {
            chunk_size: 1024,
            concurrency: 2,
            ..Default::default()
        },
    );
    mgr_sha
        .download(
            &DownloadTask::new("http://mock/a.bin", dest_sha.clone())
                .with_sha1(sha.clone())
                .with_size(good.len() as u64),
        )
        .await
        .expect("sha1 must accept the correct digest");
    assert_eq!(std::fs::read(&dest_sha).unwrap(), good);

    // MD5 acceptance.
    let dest_md5 = dir.join("b.bin");
    let mgr_md5 = DownloadManager::new(
        Arc::new(MockSource::new(good.clone())),
        DownloadOptions {
            chunk_size: 1024,
            concurrency: 2,
            ..Default::default()
        },
    );
    mgr_md5
        .download(
            &DownloadTask::new("http://mock/b.bin", dest_md5.clone())
                .with_md5(md5)
                .with_size(good.len() as u64),
        )
        .await
        .expect("md5 must accept the correct digest");
    assert_eq!(std::fs::read(&dest_md5).unwrap(), good);

    // Corruption: the server returns different bytes than the expected digest,
    // so the verifier must reject (never materialise a corrupt artifact).
    let corrupt: Vec<u8> = (0..4096u32).map(|i| (i % 11) as u8).collect();
    let dest_bad = dir.join("c.bin");
    let mgr_bad = DownloadManager::new(
        Arc::new(MockSource::new(corrupt)),
        DownloadOptions {
            chunk_size: 1024,
            concurrency: 2,
            ..Default::default()
        },
    );
    let res = mgr_bad
        .download(
            &DownloadTask::new("http://mock/c.bin", dest_bad.clone())
                .with_sha1(sha.clone())
                .with_size(good.len() as u64),
        )
        .await;
    assert!(
        res.is_err(),
        "checksum mismatch must reject corrupt content (validate pillar)"
    );
}
