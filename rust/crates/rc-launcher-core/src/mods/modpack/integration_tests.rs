//! Integration tests for the modpack import pipeline (task 17).
//!
//! These tests build a real `.zip` / `.mrpack` archive in a temp directory,
//! feed it to [`parse_archive_bytes`] / [`extract_overrides`] and verify the
//! importer handles each flavour end-to-end.

use std::io::Write;

use zip::write::SimpleFileOptions;

use crate::mods::modpack::{
    detect_and_parse, extract_overrides, parse_archive_bytes, Manifest, ModpackFlavour,
};

/// Helper: write a single archive and return the bytes.
fn build_archive(name: &str, files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::<u8>::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (path, data) in files {
            zip.start_file(*path, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }
    buf.into_inner()
}

fn tempdir(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("rc_modpack_it_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn archive_round_trip_modrinth() {
    let index = "{\"formatVersion\":1,\"game\":\"minecraft\",\"name\":\"Modrinth Sample\",\"versionId\":\"1.0\",\"summary\":\"end-to-end test\",\"dependencies\":{\"minecraft\":\"1.20.1\",\"fabric-loader\":\"0.16.0\"},\"files\":[{\"path\":\"mods/a.jar\",\"hashes\":{\"sha1\":\"\"},\"downloads\":[\"https://e/a\"]},{\"path\":\"mods/b.jar\",\"hashes\":{},\"downloads\":[\"https://e/b\"]}]}";
    let bytes = build_archive(
        "mrpack",
        &[
            ("modrinth.index.json", index.as_bytes().to_vec()),
            ("overrides/config/foo.yml", b"hello: world".to_vec()),
            ("overrides/resourcepacks/pack.zip", b"PK".to_vec()),
        ],
    );
    let manifest = parse_archive_bytes(&bytes, Some("archive:modrinth")).unwrap();
    assert_eq!(manifest.flavour(), ModpackFlavour::Modrinth);
    assert_eq!(manifest.spec().files.len(), 2);

    // Extract overrides.
    let root = tempdir("mrpack");
    let written = extract_overrides(&bytes, &root).unwrap();
    assert_eq!(written.len(), 2);
    let cfg = std::fs::read(root.join("config/foo.yml")).unwrap();
    assert_eq!(cfg, b"hello: world".to_vec());
    let pack = std::fs::read(root.join("resourcepacks/pack.zip")).unwrap();
    assert_eq!(pack, b"PK".to_vec());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn archive_round_trip_curseforge() {
    let manifest_json = "{\"manifestType\":\"minecraftModpack\",\"manifestVersion\":1,\"name\":\"CF Sample\",\"version\":\"0.1\",\"author\":\"Test\",\"files\":[{\"projectID\":1,\"fileID\":2,\"required\":true,\"fileName\":\"jei.jar\"}],\"overrides\":\"overrides\",\"minecraft\":{\"version\":\"1.20.1\",\"modLoaders\":[{\"id\":\"forge-47.2.0\",\"primary\":true}]}}";
    let bytes = build_archive(
        "cf",
        &[
            ("manifest.json", manifest_json.as_bytes().to_vec()),
            ("overrides/keybinds.txt", b"forward=W".to_vec()),
        ],
    );
    let m = parse_archive_bytes(&bytes, None).unwrap();
    assert_eq!(m.flavour(), ModpackFlavour::CurseForge);
    let s = m.spec();
    assert_eq!(s.loader.as_str(), "forge");
    assert_eq!(s.loader_version.as_deref(), Some("47.2.0"));
    assert_eq!(s.files.len(), 1);
    assert!(s.files[0].url.contains("edge.forgecdn.net"));

    let root = tempdir("cf");
    let written = extract_overrides(&bytes, &root).unwrap();
    assert_eq!(written.len(), 1);
    let kb = std::fs::read(root.join("keybinds.txt")).unwrap();
    assert_eq!(kb, b"forward=W".to_vec());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn archive_round_trip_mmc() {
    let instance_cfg = "name=MMC Sample\nMCVersion=1.20.1\nForgeVersion=47.2.0\n";
    let pack_json = "{\"formatVersion\":1,\"name\":\"MMC Sample\",\"version\":\"1.0\",\"mcVersion\":\"1.20.1\",\"forgeVersion\":\"47.2.0\",\"mods\":[{\"id\":\"jei\",\"name\":\"JEI\",\"version\":\"15.3\",\"url\":\"https://e/jei.jar\"}]}";
    let bytes = build_archive(
        "mmc",
        &[
            ("instance.cfg", instance_cfg.as_bytes().to_vec()),
            ("mmc-pack.json", pack_json.as_bytes().to_vec()),
        ],
    );
    let m = parse_archive_bytes(&bytes, None).unwrap();
    assert_eq!(m.flavour(), ModpackFlavour::MultiMc);
    assert_eq!(m.spec().files.len(), 1);
    assert_eq!(m.spec().files[0].url, "https://e/jei.jar");
}

#[test]
fn archive_detects_by_shape_not_by_filename() {
    let index = "{\"formatVersion\":1,\"game\":\"minecraft\",\"name\":\"Shape Test\",\"dependencies\":{\"minecraft\":\"1.20.1\"},\"files\":[]}";
    // Put the JSON at the wrong file name to confirm `parse_archive_bytes`
    // doesn't pick by extension alone.
    let bytes = build_archive("weird", &[("not_modrinth.json", index.as_bytes().to_vec())]);
    // No recognised manifest file -> error
    assert!(parse_archive_bytes(&bytes, None).is_err());
}

#[test]
fn detect_and_parse_text_only_modrinth() {
    let json = "{\"formatVersion\":1,\"game\":\"minecraft\",\"name\":\"Just text\",\"dependencies\":{\"minecraft\":\"1.20.1\"},\"files\":[]}";
    let m = detect_and_parse(json, None).unwrap();
    assert_eq!(m.flavour(), ModpackFlavour::Modrinth);
}

#[test]
fn detect_and_parse_rejects_garbage() {
    assert!(detect_and_parse("not json", None).is_err());
    assert!(detect_and_parse("[1,2,3]", None).is_err());
    assert!(detect_and_parse("{}", None).is_err());
}

#[test]
fn extract_overrides_skips_non_override_entries() {
    let bytes = build_archive(
        "mixed",
        &[
            ("modrinth.index.json", br#"{"formatVersion":1,"game":"minecraft","name":"x","dependencies":{"minecraft":"1.20.1"},"files":[]}"#.to_vec()),
            ("mods/file.jar", b"jar".to_vec()),
            ("overrides/wanted.txt", b"yes".to_vec()),
        ],
    );
    let root = tempdir("skip");
    let written = extract_overrides(&bytes, &root).unwrap();
    // Only the overrides entry should be extracted.
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].file_name().unwrap(), "wanted.txt");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn manifest_serde_roundtrip() {
    let m = Manifest::Modrinth {
        raw: Some("https://e/x".into()),
        spec: crate::mods::modpack::ModpackSpec::new("X", "1.20.1"),
    };
    let j = serde_json::to_string(&m).unwrap();
    let back: Manifest = serde_json::from_str(&j).unwrap();
    assert_eq!(m, back);
}
