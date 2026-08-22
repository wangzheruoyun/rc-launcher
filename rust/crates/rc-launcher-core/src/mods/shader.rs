//! Shader-pack management (task 8).
//!
//! OptiFine / Iris shader packs live in the instance's `shaderpacks/`
//! directory as either a folder or a `.zip` that contains a `shaders/` tree
//! (`shaders.properties` + `.fsh`/`.vsh` programs). Unlike mods/resource-packs
//! they have **no** embedded manifest, so identity is the folder/zip name and
//! validity is "does it contain a `shaders/` tree". Enable/disable again uses
//! the `.disabled` suffix convention for durable, zero-bookkeeping state and
//! clean per-instance **version isolation**.

use crate::error::{RcError, RcResult};
use std::path::{Path, PathBuf};

/// A shader pack discovered in the instance `shaderpacks/` directory.
#[derive(Debug, Clone)]
pub struct ShaderPack {
    /// Absolute path (may end in `.disabled` when disabled).
    pub path: PathBuf,
    /// Base name (without `.disabled`), used as the display id.
    pub name: String,
    /// Whether the pack is enabled (selected by the game).
    pub enabled: bool,
    /// `true` if the archive/folder actually contains a `shaders/` tree.
    pub valid: bool,
}

/// Manages one instance's shader packs (version isolation).
#[derive(Debug, Clone)]
pub struct ShaderPackManager {
    dir: PathBuf,
}

impl ShaderPackManager {
    /// Bind to a per-instance `shaderpacks` directory.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The managed directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Scan for shader packs (folders and `.zip` archives) and validate each.
    pub fn scan(&self) -> RcResult<Vec<ShaderPack>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.dir).map_err(RcError::Io)? {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            let file_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .unwrap_or_default();
            let is_pack = path.is_dir() || ext == "zip";
            if !is_pack {
                continue;
            }
            let enabled = !file_name.to_ascii_lowercase().ends_with(".disabled");
            let base = strip_disabled(&file_name);
            let valid = has_shader_tree(&path);
            out.push(ShaderPack {
                path: path.clone(),
                name: base,
                enabled,
                valid,
            });
        }
        out.sort_by_key(|a| a.name.to_ascii_lowercase());
        Ok(out)
    }

    /// Enable / disable a pack by renaming (`.disabled` suffix toggle).
    pub fn set_enabled(&self, pack: &ShaderPack, enabled: bool) -> RcResult<ShaderPack> {
        let new_name = toggle_disabled(&pack.path, enabled);
        if new_name == pack.path.file_name().unwrap().to_str().unwrap() {
            return Ok(pack.clone());
        }
        let new_path = self.dir.join(new_name);
        std::fs::rename(&pack.path, &new_path).map_err(RcError::Io)?;
        Ok(ShaderPack {
            path: new_path,
            name: pack.name.clone(),
            enabled,
            valid: pack.valid,
        })
    }

    /// Copy a shader pack (folder or zip) into the managed directory.
    pub fn install(&self, src: &Path) -> RcResult<ShaderPack> {
        std::fs::create_dir_all(&self.dir).map_err(RcError::Io)?;
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| RcError::Mod("source has no file name".into()))?
            .to_string();
        let dst = self.dir.join(&name);
        if src.is_dir() {
            copy_dir(src, &dst)?;
        } else {
            std::fs::copy(src, &dst).map_err(RcError::Io)?;
        }
        let valid = has_shader_tree(&dst);
        Ok(ShaderPack {
            path: dst,
            name: strip_disabled(&name),
            enabled: !name.to_ascii_lowercase().ends_with(".disabled"),
            valid,
        })
    }

    /// Delete a shader pack.
    pub fn remove(&self, pack: &ShaderPack) -> RcResult<()> {
        if pack.path.is_dir() {
            std::fs::remove_dir_all(&pack.path).map_err(RcError::Io)
        } else {
            std::fs::remove_file(&pack.path).map_err(RcError::Io)
        }
    }
}

/// Does `path` (dir or zip) contain a `shaders/` tree?
fn has_shader_tree(path: &Path) -> bool {
    if path.is_dir() {
        let shaders = path.join("shaders");
        if shaders.is_dir() {
            return true;
        }
        // Some packs put shaders.properties at the root.
        return path.join("shaders.properties").is_file();
    }
    // zip: any entry under `shaders/`, or a root `shaders.properties`.
    read_zip_any(path, &["shaders/", "shaders.properties"])
}

/// Returns `true` if the zip has an entry starting with `shaders/` or an entry
/// exactly equal to `shaders.properties`.
fn read_zip_any(path: &Path, needles: &[&str]) -> bool {
    use std::fs::File;
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(_) => return false,
    };
    for i in 0..archive.len() {
        let name = match archive.by_index(i).map(|z| z.name().to_string()) {
            Ok(n) => n,
            Err(_) => continue,
        };
        let lower = name.to_ascii_lowercase();
        for n in needles {
            if lower == *n || lower.starts_with(n) {
                return true;
            }
        }
    }
    false
}

fn strip_disabled(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    lower
        .strip_suffix(".disabled")
        .map(|s| s.to_string())
        .unwrap_or_else(|| name.to_string())
}

fn toggle_disabled(path: &Path, enabled: bool) -> String {
    let name = path.file_name().unwrap().to_str().unwrap().to_string();
    let lower = name.to_ascii_lowercase();
    if enabled {
        strip_disabled(&name)
    } else if lower.ends_with(".disabled") {
        name
    } else {
        format!("{name}.disabled")
    }
}

fn copy_dir(src: &Path, dst: &Path) -> RcResult<()> {
    std::fs::create_dir_all(dst).map_err(RcError::Io)?;
    for entry in std::fs::read_dir(src).map_err(RcError::Io)? {
        let entry = entry.map_err(RcError::Io)?;
        let p = entry.path();
        let d = dst.join(entry.file_name());
        if p.is_dir() {
            copy_dir(&p, &d)?;
        } else {
            std::fs::copy(&p, &d).map_err(RcError::Io)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_shader(dir: &Path, name: &str, valid: bool) {
        let p = dir.join(name);
        if p.extension().map(|e| e == "zip").unwrap_or(false) {
            let file = std::fs::File::create(&p).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            if valid {
                zip.start_file("shaders/shaders.properties", opts).unwrap();
                zip.write_all(b"shadow.screen=1\n").unwrap();
            } else {
                zip.start_file("readme.txt", opts).unwrap();
                zip.write_all(b"not a shader pack\n").unwrap();
            }
            zip.finish().unwrap();
        } else {
            std::fs::create_dir_all(&p).unwrap();
            if valid {
                std::fs::create_dir_all(p.join("shaders")).unwrap();
                std::fs::write(p.join("shaders").join("shaders.properties"), "x=1").unwrap();
            } else {
                std::fs::write(p.join("notes.txt"), "nope").unwrap();
            }
        }
    }

    #[test]
    fn scan_and_validate() {
        let tmp = std::env::temp_dir().join(format!("rc_sh_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        write_shader(&tmp, "seus", true);
        write_shader(&tmp, "broken.zip", false);

        let mgr = ShaderPackManager::new(&tmp);
        let packs = mgr.scan().unwrap();
        assert_eq!(packs.len(), 2);
        let seus = packs.iter().find(|p| p.name == "seus").unwrap();
        assert!(seus.valid);
        let broken = packs.iter().find(|p| p.name == "broken.zip").unwrap();
        assert!(!broken.valid);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn shader_enable_disable() {
        let tmp = std::env::temp_dir().join(format!("rc_sh_test2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        write_shader(&tmp, "awesome", true);
        let mgr = ShaderPackManager::new(&tmp);
        let packs = mgr.scan().unwrap();
        let disabled = mgr.set_enabled(&packs[0], false).unwrap();
        assert!(!disabled.enabled);
        assert!(disabled
            .path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with(".disabled"));
        let enabled = mgr.set_enabled(&disabled, true).unwrap();
        assert!(enabled.enabled);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
