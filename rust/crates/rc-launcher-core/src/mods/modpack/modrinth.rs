//! Modrinth modpack manifest parser (task 17).
//!
//! Format reference: <https://docs.modrinth.com/docs/modpacks/format/>.
//!
//! A Modrinth modpack is a `.mrpack` zip archive containing a top-level
//! `modrinth.index.json` and an `overrides/` directory. The JSON looks like:
//!
//! ```json
//! {
//!   "formatVersion": 1,
//!   "game": "minecraft",
//!   "versionId": "1.0.0",
//!   "name": "My Modpack",
//!   "summary": "...",
//!   "files": [
//!     {
//!       "path": "mods/example-1.0.jar",
//!       "hashes": { "sha1": "..." },
//!       "downloads": ["https://cdn.modrinth.com/.../example-1.0.jar"],
//!       "fileSize": 12345,
//!       "env": { "client": "required", "server": "optional" }
//!     }
//!   ],
//!   "dependencies": {
//!     "minecraft": "1.20.1",
//!     "fabric-loader": "0.16.0"
//!   }
//! }
//! ```
//!
//! The parser is **lenient**: missing fields degrade to defaults instead of
//! failing the whole import, matching the philosophy used by every other
//! metadata parser in the core.

use std::collections::HashMap;

use serde::Deserialize;

use crate::error::{RcError, RcResult};
use crate::mods::modpack::manifest::{
    Manifest, ModpackFile, ModpackFlavour, ModpackLoader, ModpackSpec,
};

/// Raw `modrinth.index.json` schema (only the fields we read).
#[derive(Debug, Clone, Deserialize)]
pub struct ModrinthIndex {
    #[serde(rename = "formatVersion", default = "default_format_version")]
    pub format_version: u32,
    #[serde(default)]
    pub game: Option<String>,
    #[serde(default, rename = "versionId")]
    pub version_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub files: Vec<ModrinthFile>,
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
}

fn default_format_version() -> u32 {
    1
}

/// One file entry inside `modrinth.index.json#files`.
#[derive(Debug, Clone, Deserialize)]
pub struct ModrinthFile {
    pub path: String,
    #[serde(default)]
    pub hashes: HashMap<String, String>,
    pub downloads: Vec<String>,
    #[serde(default, rename = "fileSize")]
    pub file_size: Option<u64>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Parse a Modrinth `modrinth.index.json` text.
///
/// `origin` is the URL or local path the text was loaded from; it's stored
/// on the resulting [`Manifest`] so the UI can show "loaded from ..." in
/// error / success messages.
pub fn parse_modrinth_index(text: &str, origin: Option<&str>) -> RcResult<Manifest> {
    let idx: ModrinthIndex = serde_json::from_str(text)
        .map_err(|e| RcError::Other(format!("invalid modrinth.index.json: {e}")))?;

    // Validate formatVersion: Modrinth currently ships v1 only.
    if idx.format_version != 1 {
        return Err(RcError::Other(format!(
            "unsupported modrinth formatVersion: {} (only v1 is supported)",
            idx.format_version
        )));
    }
    if idx.game.as_deref().unwrap_or("minecraft") != "minecraft" {
        return Err(RcError::Other(format!(
            "modpack targets a non-minecraft game: {:?}",
            idx.game
        )));
    }

    // Pick the loader + Minecraft version out of `dependencies`.
    let mut mc_version = String::new();
    let mut loader = ModpackLoader::Vanilla;
    let mut loader_version: Option<String> = None;
    for (k, v) in &idx.dependencies {
        match k.as_str() {
            "minecraft" => mc_version = v.clone(),
            "fabric-loader" => {
                loader = ModpackLoader::Fabric;
                loader_version = Some(v.clone());
            }
            "quilt-loader" => {
                loader = ModpackLoader::Quilt;
                loader_version = Some(v.clone());
            }
            "forge" => {
                loader = ModpackLoader::Forge;
                loader_version = Some(v.clone());
            }
            "neoforge" => {
                loader = ModpackLoader::NeoForge;
                loader_version = Some(v.clone());
            }
            // Other keys (e.g. "java") are tolerated \u2014 stored nowhere, but not
            // an error: the import still proceeds.
            _ => {}
        }
    }
    if mc_version.is_empty() {
        return Err(RcError::Other(
            "modrinth.index.json is missing the `minecraft` dependency".into(),
        ));
    }

    // Map files. We keep only those whose env allows the client (server-only
    // files are excluded; we launch the client).
    let files: Vec<ModpackFile> = idx
        .files
        .into_iter()
        .filter(|f| is_client_allowed(&f.env))
        .map(|f| convert_file(f))
        .collect();

    let spec = ModpackSpec {
        name: idx.name,
        version: idx.version_id,
        summary: idx.summary.unwrap_or_default(),
        mc_version,
        loader,
        loader_version,
        files,
        overrides: Vec::new(), // Modrinth packs ship overrides as files alongside the archive; handled by the pipeline, not here.
        flavour: ModpackFlavour::Modrinth,
    };

    Ok(Manifest::Modrinth {
        raw: origin.map(|s| s.to_string()),
        spec,
    })
}

/// True when the file should be downloaded for a *client* install. Modrinth
/// marks server-only files with `env.client = "unsupported"`. Everything
/// else (`required` / `optional` / no env block) is kept.
fn is_client_allowed(env: &HashMap<String, String>) -> bool {
    match env.get("client").map(|s| s.as_str()) {
        None => true,
        Some("unsupported") => false,
        // "required" / "optional" / anything else -> keep
        Some(_) => true,
    }
}

fn convert_file(f: ModrinthFile) -> ModpackFile {
    let sha1 = f.hashes.get("sha1").cloned();
    let url = f.downloads.first().cloned().unwrap_or_default();
    ModpackFile {
        id: f.path.clone(),
        dest: None,
        path: normalise_path(&f.path),
        url,
        sha1,
        md5: None,
        size: f.file_size,
        // sha256 isn't directly consumed by the download manager today;
        // surface it as a separate field would require a new manager option,
        // so we drop it (lenient parse). The UI / verifier can read it from
        // the raw index text if needed.
        force: false,
    }
}

/// Normalise a Modrinth file path: strip a leading `./` and trim slashes.
fn normalise_path(p: &str) -> String {
    let p = p.trim();
    let p = p.strip_prefix("./").unwrap_or(p);
    p.trim_start_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "formatVersion": 1,
      "game": "minecraft",
      "versionId": "0.1.0",
      "name": "Sample Pack",
      "summary": "demo",
      "files": [
        {
          "path": "mods/fabric-api.jar",
          "hashes": {"sha1":"deadbeef","sha512":""},
          "downloads": ["https://cdn.modrinth.com/data/P7dR8mSH/versions/abc/fabric-api-0.92.0.jar"],
          "fileSize": 1500000,
          "env": {"client": "required", "server": "required"}
        },
        {
          "path": "server-only.jar",
          "hashes": {},
          "downloads": [],
          "fileSize": 10,
          "env": {"client": "unsupported", "server": "required"}
        }
      ],
      "dependencies": {"minecraft": "1.20.1", "fabric-loader": "0.16.0"}
    }"#;

    #[test]
    fn parses_basic_modrinth_index() {
        let m = parse_modrinth_index(SAMPLE, Some("https://cdn/x.mrpack")).unwrap();
        let s = m.spec();
        assert_eq!(s.name, "Sample Pack");
        assert_eq!(s.mc_version, "1.20.1");
        assert_eq!(s.loader, ModpackLoader::Fabric);
        assert_eq!(s.loader_version.as_deref(), Some("0.16.0"));
        assert_eq!(s.flavour, ModpackFlavour::Modrinth);
        // server-only file dropped
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].path, "mods/fabric-api.jar");
        assert_eq!(s.files[0].sha1.as_deref(), Some("deadbeef"));
        assert_eq!(s.files[0].size, Some(1500000));
    }

    #[test]
    fn rejects_bad_format_version() {
        let bad =
            r#"{"formatVersion":99,"name":"x","dependencies":{"minecraft":"1.20.1"},"files":[]}"#;
        assert!(parse_modrinth_index(bad, None).is_err());
    }

    #[test]
    fn rejects_non_minecraft_game() {
        let bad = r#"{"formatVersion":1,"game":"terraria","name":"x","dependencies":{"minecraft":"1"},"files":[]}"#;
        assert!(parse_modrinth_index(bad, None).is_err());
    }

    #[test]
    fn rejects_missing_minecraft_dependency() {
        let bad = r#"{"formatVersion":1,"name":"x","dependencies":{},"files":[]}"#;
        assert!(parse_modrinth_index(bad, None).is_err());
    }
}
