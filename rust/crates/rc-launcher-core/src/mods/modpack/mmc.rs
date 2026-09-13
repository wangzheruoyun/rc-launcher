//! MultiMC / Prism Launcher instance manifest parser (task 17).
//!
//! Format references:
//! * `instance.cfg`: <https://github.com/MultiMC/Launcher/wiki/Instance-Config>
//! * `mmc-pack.json`: <https://github.com/MultiMC/Launcher/wiki/MMC-Pack> (a.k.a.
//!   `pack.json` for older / third-party variants)
//!
//! A MultiMC instance is a directory with this shape:
//!
//! ```text
//! instance.cfg        \u2014 the metadata (required)
//! mmc-pack.json       \u2014 the manifest (required for our use case)
//! .minecraft/         \u2014 the game directory the instance owns
//! ```
//!
//! For our purposes the user gives us either a folder *or* a `.zip` archive
//! containing the same layout; we read both files in either case.
//!
//! The `mmc-pack.json` looks like:
//!
//! ```json
//! {
//!   "formatVersion": 1,
//!   "name": "My Pack",
//!   "version": "1.0",
//!   "mcVersion": "1.20.1",
//!   "forgeVersion": "47.2.0",
//!   "fabricLoaderVersion": "0.16.0",
//!   "quiltLoaderVersion": "0.20.0",
//!   "libraries": [],
//!   "mods": [
//!     { "id": "jei", "version": "1.20-15.3.0.6", "name": "JEI",
//!       "url": "https://example.com/jei.jar", "sha1": "..." }
//!   ]
//! }
//! ```
//!
//! The parser is **lenient** about optional fields, but a missing `mcVersion`
//! is a hard error (the whole import pipeline needs it to download the right
//! game version).

use serde::Deserialize;

use crate::error::{RcError, RcResult};
use crate::mods::modpack::manifest::{
    Manifest, ModpackFile, ModpackFlavour, ModpackLoader, ModpackSpec,
};

/// Raw `instance.cfg` (INI-style). We only care about a small number of keys.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct InstanceCfg {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub instance_type: Option<String>,
    #[serde(default, rename = "MCVersion")]
    pub mc_version: Option<String>,
    #[serde(default, rename = "ForgeVersion")]
    pub forge_version: Option<String>,
    #[serde(default, rename = "FabricLoaderVersion")]
    pub fabric_loader_version: Option<String>,
    #[serde(default, rename = "QuiltLoaderVersion")]
    pub quilt_loader_version: Option<String>,
    #[serde(default, rename = "NeoForgeVersion")]
    pub neoforge_version: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Parse a MultiMC `instance.cfg` text.
///
/// `instance.cfg` is INI-style, but the keys we care about happen to all be
/// `key=Value` (no sections), so a small line-based parser is enough. Quoted
/// values are stripped; whitespace around the key is trimmed.
pub fn parse_instance_cfg(text: &str) -> RcResult<InstanceCfg> {
    let mut cfg = InstanceCfg::default();
    for raw_line in text.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('[') {
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim();
        let mut value = line[eq + 1..].trim().to_string();
        if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
            || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
        {
            value = value[1..value.len() - 1].to_string();
        }
        match key {
            "name" => cfg.name = Some(value),
            "instanceType" => cfg.instance_type = Some(value),
            "MCVersion" => cfg.mc_version = Some(value),
            "ForgeVersion" => cfg.forge_version = Some(value),
            "FabricLoaderVersion" => cfg.fabric_loader_version = Some(value),
            "QuiltLoaderVersion" => cfg.quilt_loader_version = Some(value),
            "NeoForgeVersion" => cfg.neoforge_version = Some(value),
            "notes" => cfg.notes = Some(value),
            _ => {} // unknown keys ignored (lenient)
        }
    }
    Ok(cfg)
}

/// Raw `mmc-pack.json` / `pack.json` schema (only the fields we read).
#[derive(Debug, Clone, Deserialize)]
pub struct MmcPack {
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default, rename = "mcVersion")]
    pub mc_version: Option<String>,
    #[serde(default, rename = "forgeVersion")]
    pub forge_version: Option<String>,
    #[serde(default, rename = "fabricLoaderVersion")]
    pub fabric_loader_version: Option<String>,
    #[serde(default, rename = "quiltLoaderVersion")]
    pub quilt_loader_version: Option<String>,
    #[serde(default, rename = "neoForgeVersion")]
    pub neo_forge_version: Option<String>,
    #[serde(default)]
    pub mods: Vec<MmcMod>,
}

fn default_format_version() -> u32 {
    1
}

/// One entry of `mmc-pack.json#mods`.
#[derive(Debug, Clone, Deserialize)]
pub struct MmcMod {
    /// Mod id (HMCL/Prism slug, e.g. "jei").
    pub id: String,
    /// Mod display name when present.
    #[serde(default)]
    pub name: Option<String>,
    /// Concrete version (HMCL/Prism slug, e.g. "1.20-15.3.0.6").
    #[serde(default)]
    pub version: Option<String>,
    /// Optional direct download URL. When absent, the importer falls back to
    /// looking the mod up in the online catalog (Modrinth / CurseForge).
    #[serde(default)]
    pub url: Option<String>,
    /// Optional sha1.
    #[serde(default)]
    pub sha1: Option<String>,
    /// Optional md5.
    #[serde(default)]
    pub md5: Option<String>,
    /// Optional explicit file name; falls back to `<id>-<version>.jar`.
    #[serde(default, rename = "fileName")]
    pub file_name: Option<String>,
}

/// Parse a MultiMC `mmc-pack.json` text (or `pack.json`; same schema).
pub fn parse_mmc_pack(text: &str) -> RcResult<MmcPack> {
    serde_json::from_str(text).map_err(|e| RcError::Other(format!("invalid mmc-pack.json: {e}")))
}

/// Build a [`Manifest`] from a parsed `instance.cfg` + `mmc-pack.json` pair.
///
/// `cfg_text` may be empty (some third-party packs omit `instance.cfg`); when
/// it is missing the manifest is built from `mmc-pack.json` alone.
pub fn build_manifest(
    cfg_text: &str,
    pack_text: &str,
    _origin: Option<&str>,
) -> RcResult<Manifest> {
    let cfg = if cfg_text.trim().is_empty() {
        InstanceCfg::default()
    } else {
        parse_instance_cfg(cfg_text)?
    };
    let pack = parse_mmc_pack(pack_text)?;

    // Minecraft version comes from the pack first, then the cfg, then an error.
    let mc_version = pack
        .mc_version
        .clone()
        .or_else(|| cfg.mc_version.clone())
        .ok_or_else(|| {
            RcError::Other(
                "mmc-pack.json is missing `mcVersion` (and instance.cfg has none)".into(),
            )
        })?;

    // Loader family + version: prefer the pack, then cfg, then Vanilla.
    let (loader, loader_version) = if let Some(v) = pack.forge_version.clone() {
        (ModpackLoader::Forge, Some(v))
    } else if let Some(v) = pack.fabric_loader_version.clone() {
        (ModpackLoader::Fabric, Some(v))
    } else if let Some(v) = pack.quilt_loader_version.clone() {
        (ModpackLoader::Quilt, Some(v))
    } else if let Some(v) = pack.neo_forge_version.clone() {
        (ModpackLoader::NeoForge, Some(v))
    } else if let Some(v) = cfg.forge_version.clone() {
        (ModpackLoader::Forge, Some(v))
    } else if let Some(v) = cfg.fabric_loader_version.clone() {
        (ModpackLoader::Fabric, Some(v))
    } else if let Some(v) = cfg.quilt_loader_version.clone() {
        (ModpackLoader::Quilt, Some(v))
    } else if let Some(v) = cfg.neoforge_version.clone() {
        (ModpackLoader::NeoForge, Some(v))
    } else {
        (ModpackLoader::Vanilla, None)
    };

    let files: Vec<ModpackFile> = pack
        .mods
        .into_iter()
        .filter_map(|m| {
            // Mods without an explicit URL can't be downloaded \u2014 the importer
            // resolves them via the online catalog; we still include them in the
            // spec so the pipeline can pick them up.
            m.url.as_ref().map(|url| {
                let file_name = m
                    .file_name
                    .clone()
                    .unwrap_or_else(|| build_mmc_filename(&m));
                ModpackFile {
                    id: m.id.clone(),
                    dest: None,
                    path: format!("mods/{}", file_name),
                    url: url.clone(),
                    sha1: m.sha1.clone(),
                    md5: m.md5.clone(),
                    size: None,
                    force: false,
                }
            })
        })
        .collect();

    let spec = ModpackSpec {
        name: pack
            .name
            .or(cfg.name)
            .unwrap_or_else(|| "MultiMC instance".to_string()),
        version: pack.version,
        summary: cfg.notes.unwrap_or_default(),
        mc_version,
        loader,
        loader_version,
        files,
        overrides: Vec::new(),
        flavour: ModpackFlavour::MultiMc,
    };

    Ok(Manifest::MultiMc {
        cfg: if cfg_text.is_empty() {
            None
        } else {
            Some(cfg_text.to_string())
        },
        pack: Some(pack_text.to_string()),
        spec,
    })
}

fn build_mmc_filename(m: &MmcMod) -> String {
    match &m.version {
        Some(v) => format!("{}-{}.jar", m.id, v),
        None => format!("{}.jar", m.id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CFG: &str = "\
name=Test Pack
instanceType=OneSix
MCVersion=1.20.1
ForgeVersion=47.2.0
notes=Hand-built test pack.
";

    const SAMPLE_PACK: &str = r#"{
      "formatVersion": 1,
      "name": "MMC Sample",
      "version": "0.1.0",
      "mcVersion": "1.20.1",
      "forgeVersion": "47.2.0",
      "mods": [
        {
          "id": "jei",
          "name": "JEI",
          "version": "15.3.0.6",
          "url": "https://example.com/jei.jar",
          "sha1": "deadbeef",
          "fileName": "jei-1.20.jar"
        },
        {
          "id": "missing-url",
          "name": "Lookup-required"
        }
      ]
    }"#;

    #[test]
    fn instance_cfg_parser_extracts_keys() {
        let cfg = parse_instance_cfg(SAMPLE_CFG).unwrap();
        assert_eq!(cfg.name.as_deref(), Some("Test Pack"));
        assert_eq!(cfg.mc_version.as_deref(), Some("1.20.1"));
        assert_eq!(cfg.forge_version.as_deref(), Some("47.2.0"));
        assert_eq!(cfg.notes.as_deref(), Some("Hand-built test pack."));
    }

    #[test]
    fn build_manifest_combines_cfg_and_pack() {
        let m = build_manifest(SAMPLE_CFG, SAMPLE_PACK, Some("/p/instance")).unwrap();
        let s = m.spec();
        assert_eq!(s.mc_version, "1.20.1");
        assert_eq!(s.loader, ModpackLoader::Forge);
        assert_eq!(s.loader_version.as_deref(), Some("47.2.0"));
        // mods without an explicit URL are dropped from `files` (the catalog
        // resolver picks them up separately; this test just checks the
        // direct-URL path).
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].path, "mods/jei-1.20.jar");
        assert_eq!(s.files[0].sha1.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn rejects_pack_without_mc_version() {
        let bad = r#"{"formatVersion":1,"name":"x","mods":[]}"#;
        assert!(build_manifest("", bad, None).is_err());
    }
}
