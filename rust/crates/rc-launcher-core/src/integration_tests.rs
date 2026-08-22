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
                .with_sha1(sha)
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
