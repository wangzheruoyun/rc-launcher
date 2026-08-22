//! Mod / resource-pack / shader management (task 8).
//!
//! This is the Rust analogue of FCL's `fclcore/mod` subsystem. It turns a
//! *per-instance* folder of mod archives into a typed, queryable, and
//! *validatable* view:
//!
//! * **[`loader`]** — the loader taxonomy (`Forge` / `Fabric` / `Quilt` /
//!   `LiteLoader` / `OptiFine` / `Vanilla`).
//! * **[`constraint`]** — Minecraft version-range parsing (Fabric semver &
//!   Forge Maven notation) + matching.
//! * **[`metadata`]** — per-loader manifest parsers → a uniform
//!   [`metadata::ModMetadata`] (id, version, deps, conflicts, …).
//! * **[`conflict`]** — dependency / conflict resolution against the instance's
//!   Minecraft version.
//! * **[`resource_pack`]** — `pack.mcmeta` resource packs (enable/disable +
//!   pack-format aware).
//! * **[`shader`]** — OptiFine / Iris shader packs (directory or zip
//!   containing a `shaders/` tree).
//!
//! # Version isolation
//!
//! Every manager is constructed **per instance** (one `mods_dir`, one
//! `resourcepacks_dir`, one `shaderpacks_dir`). Two instances therefore never
//! share mod state — that is the version-isolation boundary the task asks for.
//! Scans never recurse into other instances.
//!
//! # Enable / disable
//!
//! A mod file is *disabled* when its name ends with `.disabled`
//! (FCL/HMCL convention). Toggling renames the file on disk, so the state is
//! durable and survives a launcher restart with zero extra bookkeeping.

pub mod conflict;
pub mod constraint;
pub mod loader;
pub mod metadata;
pub mod resource_pack;
pub mod shader;

pub use conflict::{resolve_issues, ModIssue, ModIssueKind, ModView};
pub use constraint::VersionConstraint;
pub use loader::ModLoader;
pub use metadata::{ModDependency, ModMetadata};

use crate::error::{RcError, RcResult};
use std::path::{Path, PathBuf};

/// Archive extensions we treat as mod containers.
const MOD_ARCHIVE_EXTS: &[&str] = &["jar", "litemod", "zip"];

/// One physical mod file on disk (`.jar` / `.litemod` / `.zip`), with its
/// parsed metadata. A single Forge archive may declare several mods
/// (`[[mods]]`), so `metadata` is a list; [`LocalModFile::primary`] returns the
/// first / display one.
#[derive(Debug, Clone)]
pub struct LocalModFile {
    /// Absolute path to the (possibly renamed) file.
    pub path: PathBuf,
    /// File name (mirrors `path`, kept for cheap display + rename math).
    pub file_name: String,
    /// Loader detected for this archive.
    pub loader: ModLoader,
    /// Parsed manifests (usually one; Forge can have several).
    pub metadata: Vec<ModMetadata>,
}

impl LocalModFile {
    /// Is this file currently disabled (`.disabled` suffix)?
    pub fn is_enabled(&self) -> bool {
        !self.file_name.to_ascii_lowercase().ends_with(".disabled")
    }

    /// The first metadata entry, used as the mod's display identity.
    pub fn primary(&self) -> Option<&ModMetadata> {
        self.metadata.first()
    }

    /// All mod ids declared by this file.
    pub fn modids(&self) -> Vec<&str> {
        self.metadata.iter().map(|m| m.modid.as_str()).collect()
    }

    /// File name with the `.disabled` suffix toggled.
    fn toggled_name(&self, enabled: bool) -> String {
        let lower = self.file_name.to_ascii_lowercase();
        if enabled {
            // strip a trailing `.disabled`
            lower
                .strip_suffix(".disabled")
                .map(|s| s.to_string())
                .unwrap_or_else(|| self.file_name.clone())
        } else {
            if lower.ends_with(".disabled") {
                self.file_name.clone()
            } else {
                format!("{}.disabled", self.file_name)
            }
        }
    }
}

/// A logical mod = one or more physical [`LocalModFile`]s with the same id
/// (e.g. a main jar plus an embedded jar). Used by the UI to present one row
/// per mod rather than one row per file.
#[derive(Debug, Clone)]
pub struct LocalMod {
    /// Canonical mod id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Loader.
    pub loader: ModLoader,
    /// Physical files backing this mod.
    pub files: Vec<LocalModFile>,
}

impl LocalMod {
    /// `true` if at least one backing file is enabled.
    pub fn is_enabled(&self) -> bool {
        self.files.iter().any(|f| f.is_enabled())
    }

    /// Toggle every backing file. Returns the refreshed files.
    pub fn set_enabled(&self, manager: &ModManager, enabled: bool) -> RcResult<Vec<LocalModFile>> {
        let mut out = Vec::with_capacity(self.files.len());
        for f in &self.files {
            out.push(manager.set_enabled(f, enabled)?);
        }
        Ok(out)
    }
}

impl ModView for LocalModFile {
    fn meta(&self) -> &ModMetadata {
        // `scan` / `parse_file` guarantee at least one (possibly placeholder)
        // metadata entry, so this is always `Some`.
        self.metadata
            .first()
            .expect("LocalModFile always carries >=1 metadata entry")
    }
    fn is_enabled(&self) -> bool {
        LocalModFile::is_enabled(self)
    }
}

/// Manages the mods of **one instance** (version isolation).
#[derive(Debug, Clone)]
pub struct ModManager {
    mods_dir: PathBuf,
}

impl ModManager {
    /// Bind the manager to a per-instance `mods` directory. The directory is
    /// created on [`ModManager::scan`]/[`ModManager::install`] if missing.
    pub fn new(mods_dir: impl Into<PathBuf>) -> Self {
        Self {
            mods_dir: mods_dir.into(),
        }
    }

    /// The managed directory.
    pub fn mods_dir(&self) -> &Path {
        &self.mods_dir
    }

    /// Scan the directory, parsing every mod archive into a [`LocalModFile`].
    ///
    /// Only files whose extension marks them as mod containers (`.jar` /
    /// `.litemod` / `.zip`) or whose name matches OptiFine are considered.
    /// Each parsed file is guaranteed to carry at least one [`ModMetadata`]
    /// entry (a `Vanilla` placeholder when no manifest is found), and a file
    /// that fails to parse is kept with a placeholder so the user can see and
    /// remove it rather than the whole scan aborting (robustness).
    pub fn scan(&self) -> RcResult<Vec<LocalModFile>> {
        if !self.mods_dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.mods_dir).map_err(RcError::Io)? {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let file_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .unwrap_or_default();
            let is_mod = MOD_ARCHIVE_EXTS.contains(&ext.as_str()) || is_optifine_name(&file_name);
            if !is_mod {
                continue;
            }
            match Self::parse_file(&path, &file_name) {
                Ok(mut f) => {
                    f.metadata.retain(|m| !m.modid.is_empty());
                    if f.metadata.is_empty() {
                        f.metadata.push(ModMetadata::empty(ModLoader::Vanilla));
                    }
                    out.push(f);
                }
                Err(_) => {
                    // Keep unparseable archives visible (placeholder metadata).
                    out.push(LocalModFile {
                        path,
                        file_name,
                        loader: ModLoader::Vanilla,
                        metadata: vec![ModMetadata::empty(ModLoader::Vanilla)],
                    });
                }
            }
        }
        out.sort_by_key(|a| a.file_name.to_ascii_lowercase());
        Ok(out)
    }

    /// Parse a single mod file on disk into a [`LocalModFile`] (without
    /// touching the directory).
    pub fn parse_file(path: &Path, file_name: &str) -> RcResult<LocalModFile> {
        // A disabled file keeps a `.disabled` suffix; strip it for any
        // name/extension-based detection so the archive is still recognised.
        let base_name = {
            let lower = file_name.to_ascii_lowercase();
            lower
                .strip_suffix(".disabled")
                .map(|s| s.to_string())
                .unwrap_or_else(|| file_name.to_string())
        };
        // OptiFine is name-detected, no embedded manifest.
        if is_optifine_name(&base_name) {
            return Ok(LocalModFile {
                path: path.to_path_buf(),
                file_name: file_name.to_string(),
                loader: ModLoader::OptiFine,
                metadata: vec![ModMetadata::optifine_from_name(file_name)],
            });
        }

        let ext = base_name
            .rsplit('.')
            .next()
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if !MOD_ARCHIVE_EXTS.contains(&ext.as_str()) {
            return Err(RcError::Mod(format!("not a mod archive: {file_name}")));
        }

        // Detect the loader by which manifest entry is present.
        let loader = detect_loader(path)?;
        let mut metadata = Vec::new();
        match loader {
            ModLoader::Forge => {
                // mods.toml takes precedence over mcmod.info.
                if let Some(toml) = read_entry(path, "META-INF/mods.toml")? {
                    metadata.extend(metadata::ModMetadata::parse_for(ModLoader::Forge, &toml)?);
                } else if let Some(info) = read_entry(path, "mcmod.info")? {
                    metadata.extend(metadata::ModMetadata::parse_for(ModLoader::Forge, &info)?);
                }
            }
            ModLoader::Fabric => {
                if let Some(json) = read_entry(path, "fabric.mod.json")? {
                    metadata.extend(metadata::ModMetadata::parse_for(ModLoader::Fabric, &json)?);
                }
            }
            ModLoader::Quilt => {
                if let Some(json) = read_entry(path, "quilt.mod.json")? {
                    metadata.extend(metadata::ModMetadata::parse_for(ModLoader::Quilt, &json)?);
                } else if let Some(json) = read_entry(path, "fabric.mod.json")? {
                    // Quilt mods sometimes ship a fabric manifest.
                    metadata.extend(metadata::ModMetadata::parse_for(ModLoader::Fabric, &json)?);
                }
            }
            ModLoader::LiteLoader => {
                if let Some(json) = read_entry(path, "litemod.json")? {
                    metadata.extend(metadata::ModMetadata::parse_for(
                        ModLoader::LiteLoader,
                        &json,
                    )?);
                }
            }
            _ => {}
        }
        if metadata.is_empty() {
            metadata.push(ModMetadata::empty(loader));
        }
        Ok(LocalModFile {
            path: path.to_path_buf(),
            file_name: file_name.to_string(),
            loader,
            metadata,
        })
    }

    /// Copy `src` into the managed directory and parse it. The destination name
    /// is the source's file name. This is the "install a mod" entry point.
    pub fn install(&self, src: &Path) -> RcResult<LocalModFile> {
        std::fs::create_dir_all(&self.mods_dir).map_err(RcError::Io)?;
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| RcError::Mod("source has no file name".into()))?
            .to_string();
        let dst = self.mods_dir.join(&name);
        std::fs::copy(src, &dst).map_err(RcError::Io)?;
        Self::parse_file(&dst, &name)
    }

    /// Enable or disable a mod file by renaming it on disk. Returns the
    /// refreshed (re-parsed) [`LocalModFile`] with the new path.
    pub fn set_enabled(&self, file: &LocalModFile, enabled: bool) -> RcResult<LocalModFile> {
        let new_name = file.toggled_name(enabled);
        if new_name == file.file_name {
            return Ok(file.clone());
        }
        let new_path = self.mods_dir.join(&new_name);
        std::fs::rename(&file.path, &new_path).map_err(RcError::Io)?;
        Self::parse_file(&new_path, &new_name)
    }

    /// Delete a mod file from disk.
    pub fn remove(&self, file: &LocalModFile) -> RcResult<()> {
        std::fs::remove_file(&file.path).map_err(RcError::Io)
    }

    /// Group scanned [`LocalModFile`]s by mod id into [`LocalMod`]s for UI
    /// presentation. Files with no parsed id fall back to their file name as id.
    pub fn group_by_id(&self, files: &[LocalModFile]) -> Vec<LocalMod> {
        let mut groups: std::collections::BTreeMap<String, LocalMod> =
            std::collections::BTreeMap::new();
        for f in files {
            let id = f
                .primary()
                .map(|m| m.modid.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| f.file_name.clone());
            let name = f
                .primary()
                .map(|m| m.name.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| id.clone());
            let loader = f.loader;
            groups
                .entry(id.clone())
                .or_insert_with(|| LocalMod {
                    id: id.clone(),
                    name,
                    loader,
                    files: Vec::new(),
                })
                .files
                .push(f.clone());
        }
        groups.into_values().collect()
    }

    /// Scan + validate the enabled mods against `game_version`, returning the
    /// concrete list of [`ModIssue`]s (missing deps, conflicts, broken MC
    /// version, duplicates).
    pub fn resolve(&self, game_version: Option<&str>) -> RcResult<Vec<ModIssue>> {
        let files = self.scan()?;
        Ok(resolve_issues(&files, game_version))
    }
}

/// True for `OptiFine_*.jar` / `optifine*.jar` (case-insensitive).
fn is_optifine_name(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    (lower.starts_with("optifine_") || lower.starts_with("optifine")) && lower.ends_with(".jar")
}

/// Detect a loader by probing for its manifest entry.
fn detect_loader(path: &Path) -> RcResult<ModLoader> {
    if read_entry(path, "META-INF/mods.toml")?.is_some() {
        return Ok(ModLoader::Forge);
    }
    if read_entry(path, "mcmod.info")?.is_some() {
        return Ok(ModLoader::Forge);
    }
    if read_entry(path, "quilt.mod.json")?.is_some() {
        return Ok(ModLoader::Quilt);
    }
    if read_entry(path, "fabric.mod.json")?.is_some() {
        return Ok(ModLoader::Fabric);
    }
    if read_entry(path, "litemod.json")?.is_some() {
        return Ok(ModLoader::LiteLoader);
    }
    Ok(ModLoader::Vanilla)
}

/// Read a (case-insensitive) entry out of a zip/litemod/zip archive as text.
fn read_entry(path: &Path, entry_name: &str) -> RcResult<Option<String>> {
    use std::fs::File;
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(_) => return Ok(None),
    };
    let target = entry_name.to_ascii_lowercase();
    for i in 0..archive.len() {
        let mut zf = match archive.by_index(i) {
            Ok(z) => z,
            Err(_) => continue,
        };
        if zf.name().to_ascii_lowercase() != target {
            continue;
        }
        let mut buf = String::new();
        use std::io::Read;
        if zf.read_to_string(&mut buf).is_ok() {
            return Ok(Some(buf));
        }
        return Ok(None);
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_jar(dir: &Path, name: &str, entries: &[(&str, &str)]) -> PathBuf {
        let path = dir.join(name);
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (ename, content) in entries {
            zip.start_file(*ename, opts).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    #[test]
    fn scan_parses_fabric_and_forge() {
        let tmp = std::env::temp_dir().join(format!("rc_mod_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        write_jar(
            &tmp,
            "sodium.jar",
            &[(
                "fabric.mod.json",
                r#"{"schemaVersion":1,"id":"sodium","version":"0.4","name":"Sodium","depends":{"minecraft":">=1.16"}}"#,
            )],
        );
        write_jar(
            &tmp,
            "example.jar",
            &[(
                "META-INF/mods.toml",
                r#"
modLoader = "javafml"
loaderVersion = "[24,)"
[[mods]]
modId = "example"
version = "1.0"
displayName = "Example"
[[dependencies.example]]
modId = "minecraft"
type = "REQUIRED"
versionRange = "[1.16.5]"
"#,
            )],
        );

        let mgr = ModManager::new(&tmp);
        let files = mgr.scan().unwrap();
        assert_eq!(files.len(), 2);
        let sodium = files
            .iter()
            .find(|f| f.primary().unwrap().modid == "sodium")
            .unwrap();
        assert_eq!(sodium.loader, ModLoader::Fabric);
        let example = files
            .iter()
            .find(|f| f.primary().unwrap().modid == "example")
            .unwrap();
        assert_eq!(example.loader, ModLoader::Forge);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn install_disable_enable_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("rc_mod_test2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let src = write_jar(
            &tmp,
            "src_mod.jar",
            &[(
                "fabric.mod.json",
                r#"{"schemaVersion":1,"id":"m","version":"1","name":"M"}"#,
            )],
        );
        let mgr = ModManager::new(tmp.join("mods"));
        let installed = mgr.install(&src).unwrap();
        assert!(installed.is_enabled());
        assert_eq!(installed.primary().unwrap().modid, "m");

        let disabled = mgr.set_enabled(&installed, false).unwrap();
        assert!(!disabled.is_enabled());
        assert!(disabled
            .path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with(".disabled"));

        let enabled = mgr.set_enabled(&disabled, true).unwrap();
        assert!(enabled.is_enabled());
        assert!(!enabled
            .path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with(".disabled"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn optifine_detected_without_manifest() {
        let tmp = std::env::temp_dir().join(format!("rc_mod_test3_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let p = write_jar(&tmp, "OptiFine_1.16.5_HD_U_G8.jar", &[]);
        let f = ModManager::parse_file(&p, "OptiFine_1.16.5_HD_U_G8.jar").unwrap();
        assert_eq!(f.loader, ModLoader::OptiFine);
        assert_eq!(f.primary().unwrap().modid, "optifine");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_reports_missing_deps() {
        let tmp = std::env::temp_dir().join(format!("rc_mod_test4_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        write_jar(
            &tmp,
            "needsb.jar",
            &[(
                "fabric.mod.json",
                r#"{"schemaVersion":1,"id":"needsb","version":"1","name":"NeedsB","depends":{"b":"*"}}"#,
            )],
        );
        let mgr = ModManager::new(&tmp);
        let issues = mgr.resolve(Some("1.18.2")).unwrap();
        assert!(issues
            .iter()
            .any(|i| i.kind == ModIssueKind::MissingDependency));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
