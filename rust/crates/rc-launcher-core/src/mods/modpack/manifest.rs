//! Modpack manifest models (task 17 \u2014 modpack import pipeline).
//!
//! This module defines the **common typed model** the import pipeline builds
//! after parsing any of the three supported archive flavours:
//!
//! | flavour        | detection                  | manifest files parsed                          |
//! |----------------|----------------------------|------------------------------------------------|
//! | **Modrinth**   | `modrinth.index.json`       | `modrinth.index.json`                          |
//! | **CurseForge** | `manifest.json`            | `manifest.json` (+ optional `modlist.html`)    |
//! | **MMC**        | top-level `instance.cfg`    | `instance.cfg` + `mmc-pack.json`/`pack.json`   |
//!
//! Parsing functions for each flavour live in their own files (see the
//! [`super`] module); the resulting [`Manifest`] enum is what the
//! [`super::Importer`] consumes, so the rest of the pipeline never touches
//! the original format directly.
//!
//! Common types:
//!
//! * [`Manifest`] \u2014 the tagged union of the three flavours.
//! * [`ModpackSpec`] \u2014 the normalised, flavour-agnostic description of a
//!   modpack: name, version, summary, Minecraft version, loader family +
//!   version, the file list (download URL + relative destination + hash) and
//!   the override marker (which files replace vanilla assets).
//! * [`ModpackFile`] \u2014 one downloadable entry (URL, path, sha1, size).
//! * [`ModpackOrigin`] \u2014 where a manifest came from (URL / local file path).
//!
//! All types are `serde` (de)serialisable so the FFI layer can ship them to
//! the Compose UI as JSON without manual marshalling.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{RcError, RcResult};
use crate::mods::loader::ModLoader;

/// The three supported modpack flavours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModpackFlavour {
    Modrinth,
    CurseForge,
    MultiMc,
}

impl ModpackFlavour {
    /// Stable, human-readable identifier used in JSON / FFI boundaries.
    pub fn as_str(self) -> &'static str {
        match self {
            ModpackFlavour::Modrinth => "modrinth",
            ModpackFlavour::CurseForge => "curseforge",
            ModpackFlavour::MultiMc => "mmc",
        }
    }
}

/// The loader family the modpack targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModpackLoader {
    /// Plain Minecraft (no mod loader).
    Vanilla,
    /// Fabric Loader.
    Fabric,
    /// Quilt Loader.
    Quilt,
    /// Minecraft Forge.
    Forge,
    /// NeoForge.
    NeoForge,
}

impl ModpackLoader {
    /// Map to the existing [`ModLoader`] taxonomy (used by the mod manager).
    pub fn as_mod_loader(self) -> ModLoader {
        match self {
            ModpackLoader::Vanilla => ModLoader::Vanilla,
            ModpackLoader::Fabric => ModLoader::Fabric,
            ModpackLoader::Quilt => ModLoader::Quilt,
            ModpackLoader::Forge => ModLoader::Forge,
            ModpackLoader::NeoForge => ModLoader::Forge, // reuses Forge metadata model
        }
    }

    /// Stable id (matches [`ModLoader::as_str`]).
    pub fn as_str(self) -> &'static str {
        match self {
            ModpackLoader::Vanilla => "vanilla",
            ModpackLoader::Fabric => "fabric",
            ModpackLoader::Quilt => "quilt",
            ModpackLoader::Forge => "forge",
            ModpackLoader::NeoForge => "neoforge",
        }
    }
}

impl std::str::FromStr for ModpackLoader {
    type Err = RcError;

    fn from_str(s: &str) -> RcResult<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "vanilla" | "none" | "minecraft" => Ok(ModpackLoader::Vanilla),
            "fabric" | "fabricloader" => Ok(ModpackLoader::Fabric),
            "quilt" | "quiltloader" => Ok(ModpackLoader::Quilt),
            "forge" => Ok(ModpackLoader::Forge),
            "neoforge" => Ok(ModpackLoader::NeoForge),
            other => Err(RcError::Other(format!("unknown modpack loader: {other}"))),
        }
    }
}

/// A single downloadable entry inside a modpack.
///
/// `path` is relative to the instance root (e.g. `mods/foo.jar`,
/// `overrides/config/foo.yml`). URLs are the original (canonical) ones the
/// source flavour provides; the download manager applies mirror fallback
/// automatically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModpackFile {
    /// Stable id (slug / project id when available, falls back to `path`).
    pub id: String,
    /// Absolute destination (instance_root + path), computed at plan time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest: Option<PathBuf>,
    /// Relative path inside the instance (always set; the absolute `dest` is
    /// derived from it + the chosen instance root).
    pub path: String,
    /// Canonical download URL.
    pub url: String,
    /// Optional SHA-1 used for verification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    /// Optional MD5 used for verification (CurseForge).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,
    /// Optional content size (bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// True for files that should be written even when the destination
    /// already exists (CurseForge: `override/...`).
    #[serde(default)]
    pub force: bool,
}

/// Normalised, flavour-agnostic description of a modpack. Produced by every
/// parser (Modrinth / CurseForge / MMC); consumed by the import pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModpackSpec {
    /// Display name of the modpack (e.g. "All the Mods 9").
    pub name: String,
    /// Optional version label inside the modpack (e.g. "1.0.2").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Short description (best-effort from the manifest, may be empty).
    #[serde(default)]
    pub summary: String,
    /// Minecraft version this modpack targets (e.g. "1.20.1").
    pub mc_version: String,
    /// Loader family + version (when provided by the flavour).
    pub loader: ModpackLoader,
    /// Concrete loader build ("0.16.0" for Fabric, "47.2.0" for Forge ...),
    /// `None` when the flavour doesn't carry it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader_version: Option<String>,
    /// Downloadable entries (mods + resource packs + overrides).
    pub files: Vec<ModpackFile>,
    /// Files that should overwrite the destination even when it already
    /// exists (CurseForge `overrides/`).
    #[serde(default)]
    pub overrides: Vec<ModpackFile>,
    /// Origin flavour (set by the parser, not user input).
    pub flavour: ModpackFlavour,
}

impl ModpackSpec {
    /// Create an empty spec; used by tests and as the seed for the
    /// parser-specific builder helpers.
    pub fn new(name: impl Into<String>, mc_version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: None,
            summary: String::new(),
            mc_version: mc_version.into(),
            loader: ModpackLoader::Vanilla,
            loader_version: None,
            files: Vec::new(),
            overrides: Vec::new(),
            flavour: ModpackFlavour::Modrinth,
        }
    }

    /// Total number of files (mods + overrides).
    pub fn total_files(&self) -> usize {
        self.files.len() + self.overrides.len()
    }

    /// Total expected download size when every file declares one.
    pub fn total_bytes(&self) -> u64 {
        self.files
            .iter()
            .filter_map(|f| f.size)
            .sum::<u64>()
            .saturating_add(self.overrides.iter().filter_map(|f| f.size).sum::<u64>())
    }

    /// Resolve every `ModpackFile::dest` against `instance_root`. Idempotent:
    /// re-running it just overwrites the absolute `dest` field.
    pub fn with_dest_resolved(&self, instance_root: &Path) -> Self {
        let mut out = self.clone();
        for f in &mut out.files {
            f.dest = Some(instance_root.join(&f.path));
        }
        for f in &mut out.overrides {
            f.dest = Some(instance_root.join(&f.path));
        }
        out
    }
}

/// Tagged union of the three manifest shapes.
///
/// Parsers always return this; downstream code only ever sees [`Spec`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "flavour", rename_all = "lowercase")]
pub enum Manifest {
    Modrinth {
        /// Raw `modrinth.index.json` text (so the UI can show it for debugging).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw: Option<String>,
        spec: ModpackSpec,
    },
    CurseForge {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw: Option<String>,
        spec: ModpackSpec,
    },
    MultiMc {
        /// Raw `instance.cfg` text.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cfg: Option<String>,
        /// Raw `mmc-pack.json` / `pack.json` text (when present).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pack: Option<String>,
        spec: ModpackSpec,
    },
}

impl Manifest {
    /// Borrow the inner normalised spec regardless of flavour.
    pub fn spec(&self) -> &ModpackSpec {
        match self {
            Manifest::Modrinth { spec, .. }
            | Manifest::CurseForge { spec, .. }
            | Manifest::MultiMc { spec, .. } => spec,
        }
    }

    /// Consume the manifest and return the inner spec.
    pub fn into_spec(self) -> ModpackSpec {
        match self {
            Manifest::Modrinth { spec, .. }
            | Manifest::CurseForge { spec, .. }
            | Manifest::MultiMc { spec, .. } => spec,
        }
    }

    /// The flavour of this manifest.
    pub fn flavour(&self) -> ModpackFlavour {
        self.spec().flavour
    }

    /// Where did this manifest come from? (`Url` / `Local` / `Unknown`).
    pub fn origin(&self) -> ModpackOrigin {
        match self {
            Manifest::Modrinth { raw, .. } => ModpackOrigin::from_optional(raw.as_deref()),
            Manifest::CurseForge { raw, .. } => ModpackOrigin::from_optional(raw.as_deref()),
            Manifest::MultiMc { cfg, .. } => ModpackOrigin::from_optional(cfg.as_deref()),
        }
    }
}

/// Where a manifest text came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModpackOrigin {
    /// A URL the manifest was fetched from (e.g. `https://cdn.modrinth.com/...`).
    Url(String),
    /// A local file path the manifest was read from.
    Local(PathBuf),
    /// No origin recorded (synthesised in-memory; rare).
    Unknown,
}

impl ModpackOrigin {
    fn from_optional(s: Option<&str>) -> Self {
        match s {
            None => ModpackOrigin::Unknown,
            Some(s) if s.starts_with("http://") || s.starts_with("https://") => {
                ModpackOrigin::Url(s.to_string())
            }
            Some(s) if !s.is_empty() => ModpackOrigin::Local(PathBuf::from(s)),
            _ => ModpackOrigin::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flavour_strings_round_trip() {
        for f in [
            ModpackFlavour::Modrinth,
            ModpackFlavour::CurseForge,
            ModpackFlavour::MultiMc,
        ] {
            let j = serde_json::to_string(&f).unwrap();
            let back: ModpackFlavour = serde_json::from_str(&j).unwrap();
            assert_eq!(back, f);
        }
    }

    #[test]
    fn loader_from_str_accepts_common_aliases() {
        assert_eq!(
            "fabric".parse::<ModpackLoader>().unwrap(),
            ModpackLoader::Fabric
        );
        assert_eq!(
            "Forge".parse::<ModpackLoader>().unwrap(),
            ModpackLoader::Forge
        );
        assert_eq!(
            "neoforge".parse::<ModpackLoader>().unwrap(),
            ModpackLoader::NeoForge
        );
        assert_eq!(
            "vanilla".parse::<ModpackLoader>().unwrap(),
            ModpackLoader::Vanilla
        );
        assert_eq!("".parse::<ModpackLoader>().unwrap(), ModpackLoader::Vanilla);
        assert!("bogus".parse::<ModpackLoader>().is_err());
    }

    #[test]
    fn total_bytes_handles_missing_sizes() {
        let mut s = ModpackSpec::new("X", "1.20.1");
        s.files.push(ModpackFile {
            id: "a".into(),
            dest: None,
            path: "mods/a.jar".into(),
            url: "https://e/a".into(),
            sha1: None,
            md5: None,
            size: Some(100),
            force: false,
        });
        s.files.push(ModpackFile {
            id: "b".into(),
            dest: None,
            path: "mods/b.jar".into(),
            url: "https://e/b".into(),
            sha1: None,
            md5: None,
            size: None,
            force: false,
        });
        assert_eq!(s.total_bytes(), 100);
        assert_eq!(s.total_files(), 2);
    }

    #[test]
    fn with_dest_resolved_writes_absolute_paths() {
        let mut s = ModpackSpec::new("X", "1.20.1");
        s.files.push(ModpackFile {
            id: "a".into(),
            dest: None,
            path: "mods/a.jar".into(),
            url: "u".into(),
            sha1: None,
            md5: None,
            size: None,
            force: false,
        });
        let resolved = s.with_dest_resolved(Path::new("/data/instance"));
        assert_eq!(
            resolved.files[0].dest.as_deref(),
            Some(Path::new("/data/instance/mods/a.jar"))
        );
    }
}
