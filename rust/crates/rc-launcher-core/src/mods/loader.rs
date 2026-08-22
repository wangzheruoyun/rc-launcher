//! Mod loader taxonomy (task 8).
//!
//! Mirrors FCL's `fclcore/mod/ModLoaderType` and `modinfo/*` parsers. A loader
//! is the framework a mod is built against (Forge / Fabric / Quilt / LiteLoader
//! / OptiFine), while `Vanilla` is the sentinel used for unversioned content
//! such as plain resource packs.

use std::fmt;
use std::str::FromStr;

/// The modding framework a [`super::metadata::ModMetadata`] belongs to.
///
/// `OptiFine` is kept separate from `Forge` even though OptiFine historically
/// shipped as a Forge coremod, because its metadata model is unique (detected
/// by file name, not by an embedded manifest) and it participates in shader
/// management as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ModLoader {
    /// No mod loader — used for pure resource / shader packs.
    #[default]
    Vanilla,
    /// Minecraft Forge (both `mcmod.info` / `mods.toml` flavours).
    Forge,
    /// Fabric Loader (`fabric.mod.json`).
    Fabric,
    /// Quilt Loader (`quilt.mod.json`).
    Quilt,
    /// LiteLoader (`.litemod` → `litemod.json`).
    LiteLoader,
    /// OptiFine (`OptiFine_*.jar`, name-detected).
    OptiFine,
}

impl ModLoader {
    /// Stable, human-readable identifier used in JSON / FFI boundaries.
    pub fn as_str(self) -> &'static str {
        match self {
            ModLoader::Vanilla => "vanilla",
            ModLoader::Forge => "forge",
            ModLoader::Fabric => "fabric",
            ModLoader::Quilt => "quilt",
            ModLoader::LiteLoader => "liteloader",
            ModLoader::OptiFine => "optifine",
        }
    }

    /// `true` for any loader that actually transforms game code (i.e. not
    /// `Vanilla`). Used by the manager to decide whether two mods can live in
    /// the same instance at all.
    pub fn is_mod_loader(self) -> bool {
        !matches!(self, ModLoader::Vanilla)
    }

    /// The canonical metadata entry name(s) read out of the mod archive for
    /// this loader, in priority order. `None` means the loader is detected
    /// out-of-band (e.g. OptiFine by file name).
    pub fn metadata_entries(self) -> &'static [&'static str] {
        match self {
            ModLoader::Fabric => &["fabric.mod.json"],
            ModLoader::Quilt => &["quilt.mod.json", "fabric.mod.json"],
            ModLoader::Forge => &["META-INF/mods.toml", "mcmod.info"],
            ModLoader::LiteLoader => &["litemod.json"],
            ModLoader::OptiFine => &[],
            ModLoader::Vanilla => &[],
        }
    }
}

impl fmt::Display for ModLoader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parses a loader from common spellings. Falls back to `Vanilla` for anything
/// unrecognised so callers can use `?`-free parsing.
impl FromStr for ModLoader {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let v = match s.trim().to_ascii_lowercase().as_str() {
            "vanilla" | "" => ModLoader::Vanilla,
            "forge" | "forge-old" | "forge-new" | "javafml" => ModLoader::Forge,
            "fabric" | "fabricloader" => ModLoader::Fabric,
            "quilt" | "quiltloader" => ModLoader::Quilt,
            "liteloader" | "lite" | "lite_mod" => ModLoader::LiteLoader,
            "optifine" | "optifinehd" => ModLoader::OptiFine,
            _ => ModLoader::Vanilla,
        };
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn str_roundtrip() {
        for l in [
            ModLoader::Vanilla,
            ModLoader::Forge,
            ModLoader::Fabric,
            ModLoader::Quilt,
            ModLoader::LiteLoader,
            ModLoader::OptiFine,
        ] {
            assert_eq!(l, ModLoader::from_str(l.as_str()).unwrap());
        }
    }

    #[test]
    fn forgiving_parse() {
        assert_eq!(ModLoader::from_str("JavaFML").unwrap(), ModLoader::Forge);
        assert_eq!(
            ModLoader::from_str("fabricloader").unwrap(),
            ModLoader::Fabric
        );
        assert_eq!(ModLoader::from_str("???").unwrap(), ModLoader::Vanilla);
        assert!(!ModLoader::Vanilla.is_mod_loader());
        assert!(ModLoader::Fabric.is_mod_loader());
    }

    #[test]
    fn metadata_entries_known() {
        assert_eq!(ModLoader::Fabric.metadata_entries(), &["fabric.mod.json"]);
        assert!(ModLoader::Forge.metadata_entries().len() == 2);
        assert!(ModLoader::OptiFine.metadata_entries().is_empty());
    }
}
