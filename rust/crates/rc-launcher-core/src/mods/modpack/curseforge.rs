//! CurseForge modpack manifest parser (task 17).
//!
//! Format reference: <https://docs.curseforge.com/#managing-minecraft-modpacks>.
//!
//! A CurseForge modpack is a `.zip` archive with this shape:
//!
//! ```text
//! manifest.json        \u2014 the manifest (required)
//! modlist.html         \u2014 optional pretty-printed mod list (ignored)
//! overrides/           \u2014 files copied verbatim over the instance root
//! ```
//!
//! The manifest JSON has this schema (only the fields we read):
//!
//! ```json
//! {
//!   "manifestType": "minecraftModpack",
//!   "manifestVersion": 1,
//!   "name": "All the Mods 9",
//!   "version": "0.2.7",
//!   "author": "ATMTeam",
//!   "files": [
//!     {
//!       "projectID": 12345,
//!       "fileID": 67890,
//!       "required": true,
//!       "displayName": "...",
//!       "fileName": "jei-1.20.jar"
//!     }
//!   ],
//!   "overrides": "overrides",
//!   "minecraft": {
//!     "version": "1.20.1",
//!     "modLoaders": [
//!       { "id": "forge-47.2.0", "primary": true },
//!       { "id": "fabric-0.16.0" }
//!     ]
//!   }
//! }
//! ```
//!
//! CurseForge files only carry `projectID` + `fileID` \u2014 no direct download
//! URL. We construct the canonical CurseForge CDN URL
//! (`https://edge.forgecdn.net/files/<fileID4>/<fileID>/<filename>`) the way
//! HMCL / FCL do, and let the download manager apply China-mainland mirror
//! fallback on top.

use serde::Deserialize;

use crate::error::{RcError, RcResult};
use crate::mods::modpack::manifest::{
    Manifest, ModpackFile, ModpackFlavour, ModpackLoader, ModpackSpec,
};

/// Raw `manifest.json` schema (only the fields we read).
#[derive(Debug, Clone, Deserialize)]
pub struct CurseManifest {
    #[serde(rename = "manifestType", default)]
    pub manifest_type: Option<String>,
    #[serde(rename = "manifestVersion", default = "default_manifest_version")]
    pub manifest_version: u32,
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub files: Vec<CurseFile>,
    /// Sub-folder name (relative to the archive root) whose contents are
    /// copied over the instance root on import.
    #[serde(default)]
    pub overrides: Option<String>,
    pub minecraft: CurseMinecraft,
}

fn default_manifest_version() -> u32 {
    1
}

/// One file entry inside `manifest.json#files`.
#[derive(Debug, Clone, Deserialize)]
pub struct CurseFile {
    #[serde(rename = "projectID")]
    pub project_id: u64,
    #[serde(rename = "fileID")]
    pub file_id: u64,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(rename = "displayName", default)]
    pub display_name: Option<String>,
    /// File name when known (used to build the CDN URL).
    #[serde(rename = "fileName", default)]
    pub file_name: Option<String>,
}

fn default_true() -> bool {
    true
}

/// The `minecraft` block.
#[derive(Debug, Clone, Deserialize)]
pub struct CurseMinecraft {
    pub version: String,
    #[serde(default, rename = "modLoaders")]
    pub mod_loaders: Vec<CurseModLoader>,
}

/// One entry of `minecraft.modLoaders`. `id` is `forge-47.2.0`,
/// `fabric-0.16.0`, `quilt-0.20.0` or `neoforge-47.2.0`.
#[derive(Debug, Clone, Deserialize)]
pub struct CurseModLoader {
    pub id: String,
    #[serde(default)]
    pub primary: bool,
}

/// Parse a CurseForge `manifest.json` text.
pub fn parse_curse_manifest(text: &str, origin: Option<&str>) -> RcResult<Manifest> {
    let m: CurseManifest = serde_json::from_str(text)
        .map_err(|e| RcError::Other(format!("invalid CurseForge manifest.json: {e}")))?;

    if m.manifest_type.as_deref().unwrap_or("minecraftModpack") != "minecraftModpack" {
        return Err(RcError::Other(format!(
            "unsupported manifestType: {:?} (only minecraftModpack is supported)",
            m.manifest_type
        )));
    }

    // Pick the *primary* loader entry if any, fall back to the first.
    let primary_loader = m
        .minecraft
        .mod_loaders
        .iter()
        .find(|l| l.primary)
        .cloned()
        .or_else(|| m.minecraft.mod_loaders.first().cloned());

    let (loader, loader_version) = match primary_loader {
        None => (ModpackLoader::Vanilla, None),
        Some(ml) => split_loader_id(&ml.id),
    };

    let files: Vec<ModpackFile> = m
        .files
        .into_iter()
        .filter(|f| f.required) // optional files are dropped (CF semantics)
        .map(|f| {
            let file_name = f
                .file_name
                .clone()
                .unwrap_or_else(|| format!("{}-{}.jar", f.project_id, f.file_id));
            let path = format!("mods/{}", file_name);
            let url = build_cdn_url(f.project_id, f.file_id, &file_name);
            ModpackFile {
                id: format!("{}-{}", f.project_id, f.file_id),
                dest: None,
                path,
                url,
                sha1: None, // CF doesn't carry sha1 in the manifest
                md5: None,
                size: None,
                force: false,
            }
        })
        .collect();

    let spec = ModpackSpec {
        name: m.name,
        version: m.version,
        summary: m.author.unwrap_or_default(),
        mc_version: m.minecraft.version,
        loader,
        loader_version,
        files,
        overrides: Vec::new(), // handled by the importer at extraction time
        flavour: ModpackFlavour::CurseForge,
    };

    Ok(Manifest::CurseForge {
        raw: origin.map(|s| s.to_string()),
        spec,
    })
}

/// Split `forge-47.2.0` / `fabric-0.16.0` / `neoforge-47.2.0` into
/// `(loader, version)`. Returns `(Vanilla, None)` when the prefix is empty.
fn split_loader_id(id: &str) -> (ModpackLoader, Option<String>) {
    let id = id.trim();
    if id.is_empty() {
        return (ModpackLoader::Vanilla, None);
    }
    let (prefix, rest) = match id.find('-') {
        Some(idx) => (&id[..idx], &id[idx + 1..]),
        None => (id, ""),
    };
    let loader = match prefix.to_ascii_lowercase().as_str() {
        "forge" => ModpackLoader::Forge,
        "fabric" => ModpackLoader::Fabric,
        "quilt" => ModpackLoader::Quilt,
        "neoforge" => ModpackLoader::NeoForge,
        _ => ModpackLoader::Vanilla,
    };
    let version = if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    };
    (loader, version)
}

/// Build the canonical CurseForge CDN URL for `file_id`.
///
/// Format: `https://edge.forgecdn.net/files/<first4>/<fileid>/<filename>`.
/// `first4` is the first four digits of `file_id`, left-padded with zeros
/// (HMCL/FCL parity).
fn build_cdn_url(_project_id: u64, file_id: u64, file_name: &str) -> String {
    let s = format!("{:0>4}", file_id.to_string());
    let first4 = if s.len() >= 4 { &s[..4] } else { &s[..] };
    format!(
        "https://edge.forgecdn.net/files/{}/{}/{}",
        first4, file_id, file_name
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "manifestType": "minecraftModpack",
      "manifestVersion": 1,
      "name": "ATM 9",
      "version": "0.2.7",
      "author": "ATMTeam",
      "files": [
        {
          "projectID": 238222,
          "fileID": 4470662,
          "required": true,
          "displayName": "Just Enough Items",
          "fileName": "jei-1.20.1-forge-15.3.0.6.jar"
        },
        {
          "projectID": 1,
          "fileID": 2,
          "required": false,
          "fileName": "optional-mod.jar"
        }
      ],
      "overrides": "overrides",
      "minecraft": {
        "version": "1.20.1",
        "modLoaders": [
          {"id": "neoforge-47.2.0", "primary": true}
        ]
      }
    }"#;

    #[test]
    fn parses_basic_curseforge_manifest() {
        let m = parse_curse_manifest(SAMPLE, Some("/tmp/atm9.zip")).unwrap();
        let s = m.spec();
        assert_eq!(s.name, "ATM 9");
        assert_eq!(s.mc_version, "1.20.1");
        assert_eq!(s.loader, ModpackLoader::NeoForge);
        assert_eq!(s.loader_version.as_deref(), Some("47.2.0"));
        assert_eq!(s.flavour, ModpackFlavour::CurseForge);
        // optional file dropped
        assert_eq!(s.files.len(), 1);
        assert!(s.files[0]
            .url
            .contains("edge.forgecdn.net/files/4470/4470662/"));
        assert_eq!(s.files[0].path, "mods/jei-1.20.1-forge-15.3.0.6.jar");
    }

    #[test]
    fn rejects_unsupported_manifest_type() {
        let bad = r#"{"manifestType":"otherPack","manifestVersion":1,"name":"x","minecraft":{"version":"1","modLoaders":[]},"files":[]}"#;
        assert!(parse_curse_manifest(bad, None).is_err());
    }

    #[test]
    fn split_loader_id_handles_all_flavours() {
        assert_eq!(
            split_loader_id("forge-47.2.0"),
            (ModpackLoader::Forge, Some("47.2.0".into()))
        );
        assert_eq!(
            split_loader_id("fabric-0.16.0"),
            (ModpackLoader::Fabric, Some("0.16.0".into()))
        );
        assert_eq!(
            split_loader_id("quilt-0.20.0"),
            (ModpackLoader::Quilt, Some("0.20.0".into()))
        );
        assert_eq!(
            split_loader_id("neoforge-47.2.0"),
            (ModpackLoader::NeoForge, Some("47.2.0".into()))
        );
        assert_eq!(split_loader_id(""), (ModpackLoader::Vanilla, None));
        assert_eq!(
            split_loader_id("unknown-1.0"),
            (ModpackLoader::Vanilla, Some("1.0".into()))
        );
    }
}
