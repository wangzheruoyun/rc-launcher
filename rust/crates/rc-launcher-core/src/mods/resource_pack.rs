//! Resource-pack management (task 8).
//!
//! A resource pack is a folder or `.zip` placed in the instance's
//! `resourcepacks/` directory and containing a `pack.mcmeta` (which carries the
//! numeric `pack_format` + a description). Like mods, packs are enabled/disabled
//! by toggling a `.disabled` suffix on the file/dir name, which keeps state
//! durable with zero extra bookkeeping and gives us clean **version isolation**
//! (one `resourcepacks/` directory per instance).
//!
//! We also expose [`pack_format_compatible`] so the UI can warn when a pack's
//! `pack_format` can't load on the selected Minecraft version — a frequent
//! source of "black textures" support tickets.

use crate::error::{RcError, RcResult};
use crate::mods::constraint::coerce_mc_version;
use std::path::{Path, PathBuf};

/// A resource pack discovered in the instance `resourcepacks/` directory.
#[derive(Debug, Clone)]
pub struct ResourcePack {
    /// Absolute path (may end in `.disabled` when disabled).
    pub path: PathBuf,
    /// Base name (without `.disabled`), used as the display id.
    pub name: String,
    /// Whether the pack is enabled (loaded by the game).
    pub enabled: bool,
    /// `pack_format` from `pack.mcmeta`, if present/parseable.
    pub pack_format: Option<u32>,
    /// Description from `pack.mcmeta`.
    pub description: Option<String>,
}

impl ResourcePack {
    /// Is this pack loadable by `game_version`? `None` formats are treated as
    /// compatible (unknown → don't block the user).
    pub fn is_compatible(&self, game_version: &str) -> bool {
        match (self.pack_format, coerce_mc_version(game_version)) {
            (Some(fmt), Some(mc)) => pack_format_compatible(fmt, &mc),
            _ => true,
        }
    }
}

/// Manages one instance's resource packs (version isolation).
#[derive(Debug, Clone)]
pub struct ResourcePackManager {
    dir: PathBuf,
}

impl ResourcePackManager {
    /// Bind to a per-instance `resourcepacks` directory.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The managed directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Scan for packs (folders and `.zip` archives), parsing `pack.mcmeta`.
    pub fn scan(&self) -> RcResult<Vec<ResourcePack>> {
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
            // Skip anything already disabled-stripped we still want to read.
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
            let (pack_format, description) = read_pack_meta(&path)?;
            out.push(ResourcePack {
                path: path.clone(),
                name: base,
                enabled,
                pack_format,
                description,
            });
        }
        out.sort_by_key(|a| a.name.to_ascii_lowercase());
        Ok(out)
    }

    /// Enable / disable a pack by renaming (`.disabled` suffix toggle).
    pub fn set_enabled(&self, pack: &ResourcePack, enabled: bool) -> RcResult<ResourcePack> {
        let new_name = toggle_disabled(&pack.path, enabled);
        if new_name == pack.path.file_name().unwrap().to_str().unwrap() {
            return Ok(pack.clone());
        }
        let new_path = self.dir.join(new_name);
        std::fs::rename(&pack.path, &new_path).map_err(RcError::Io)?;
        let (pf, desc) = read_pack_meta(&new_path)?;
        Ok(ResourcePack {
            path: new_path,
            name: pack.name.clone(),
            enabled,
            pack_format: pf,
            description: desc,
        })
    }

    /// Copy a pack archive/dir into the managed directory.
    pub fn install(&self, src: &Path) -> RcResult<ResourcePack> {
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
        let (pf, desc) = read_pack_meta(&dst)?;
        Ok(ResourcePack {
            path: dst,
            name: strip_disabled(&name),
            enabled: !name.to_ascii_lowercase().ends_with(".disabled"),
            pack_format: pf,
            description: desc,
        })
    }

    /// Delete a pack.
    pub fn remove(&self, pack: &ResourcePack) -> RcResult<()> {
        if pack.path.is_dir() {
            std::fs::remove_dir_all(&pack.path).map_err(RcError::Io)
        } else {
            std::fs::remove_file(&pack.path).map_err(RcError::Io)
        }
    }
}

/// Read `pack.mcmeta` from a directory or zip archive.
fn read_pack_meta(path: &Path) -> RcResult<(Option<u32>, Option<String>)> {
    let content = if path.is_dir() {
        let p = path.join("pack.mcmeta");
        std::fs::read_to_string(&p).ok()
    } else {
        read_zip_entry(path, "pack.mcmeta")?
    };
    let content = match content {
        Some(c) => c,
        None => return Ok((None, None)),
    };
    let v: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok((None, None)),
    };
    let pack = v.get("pack").and_then(|p| p.as_object());
    let format = pack
        .and_then(|p| p.get("pack_format"))
        .and_then(|f| f.as_u64())
        .map(|f| f as u32);
    let desc = pack
        .and_then(|p| p.get("description"))
        .and_then(|d| d.as_str())
        .map(|s| s.to_string());
    Ok((format, desc))
}

/// Read a (case-insensitive) entry from a zip archive as text.
fn read_zip_entry(path: &Path, entry_name: &str) -> RcResult<Option<String>> {
    use std::fs::File;
    use std::io::Read;
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
        if zf.read_to_string(&mut buf).is_ok() {
            return Ok(Some(buf));
        }
        return Ok(None);
    }
    Ok(None)
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

/// Map a Minecraft `pack_format` number to the inclusive range of game
/// versions that understand it: `(min_major, min_minor, min_patch, max_major,
/// max_minor, max_patch)`. Returns `None` for unknown formats.
///
/// Source: the Java `pack_format` version table used by the launcher.
pub fn pack_format_range(format: u32) -> Option<(u32, u32, u32, u32, u32, u32)> {
    let (amin, bmin, cmin, amax, bmax, cmax) = match format {
        1 => (1, 6, 0, 1, 8, 9),    // 1.6.1 – 1.8.9
        2 => (1, 9, 0, 1, 10, 2),   // 1.9 – 1.10.2
        3 => (1, 11, 0, 1, 12, 2),  // 1.11 – 1.12.2
        4 => (1, 13, 0, 1, 14, 4),  // 1.13 – 1.14.4
        5 => (1, 15, 0, 1, 16, 1),  // 1.15 – 1.16.1
        6 => (1, 16, 2, 1, 16, 5),  // 1.16.2 – 1.16.5
        7 => (1, 17, 0, 1, 17, 1),  // 1.17 – 1.17.1
        8 => (1, 18, 0, 1, 18, 2),  // 1.18 – 1.18.2
        9 => (1, 19, 0, 1, 19, 2),  // 1.19 – 1.19.2
        11 => (1, 19, 3, 1, 19, 4), // 1.19.3 – 1.19.4 (10 skipped)
        12 => (1, 20, 0, 1, 20, 1), // 1.20 – 1.20.1
        13 => (1, 20, 2, 1, 20, 4), // 1.20.2 – 1.20.4
        14 => (1, 20, 5, 1, 20, 6), // 1.20.5 – 1.20.6
        15 => (1, 21, 0, 1, 21, 1), // 1.21 – 1.21.1
        16 => (1, 21, 2, 1, 21, 4), // 1.21.2 – 1.21.4
        17 => (1, 21, 5, 1, 21, 6), // 1.21.5 – 1.21.6
        18 => (1, 21, 7, 1, 21, 9), // 1.21.7 – 1.21.9
        _ => return None,
    };
    Some((amin, bmin, cmin, amax, bmax, cmax))
}

/// Is `pack_format` loadable by Minecraft `mc` (a concrete version)?
pub fn pack_format_compatible(format: u32, mc: &semver::Version) -> bool {
    match pack_format_range(format) {
        Some((amin, bmin, cmin, amax, bmax, cmax)) => {
            let min = semver::Version::new(amin as u64, bmin as u64, cmin as u64);
            let max = semver::Version::new(amax as u64, bmax as u64, cmax as u64);
            *mc >= min && *mc <= max
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_pack(dir: &Path, name: &str, format: u32) {
        let p = dir.join(name);
        if p.extension().map(|e| e == "zip").unwrap_or(false) {
            let file = std::fs::File::create(&p).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("pack.mcmeta", opts).unwrap();
            zip.write_all(
                format!("{{\"pack\":{{\"pack_format\":{format},\"description\":\"t\"}}}}")
                    .as_bytes(),
            )
            .unwrap();
            zip.finish().unwrap();
        } else {
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(
                p.join("pack.mcmeta"),
                format!("{{\"pack\":{{\"pack_format\":{format},\"description\":\"t\"}}}}"),
            )
            .unwrap();
        }
    }

    #[test]
    fn scan_packs_and_compat() {
        let tmp = std::env::temp_dir().join(format!("rc_rp_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        write_pack(&tmp, "faithful", 8); // 1.18 compatible
        write_pack(&tmp, "ancient.zip", 1); // 1.6 compatible

        let mgr = ResourcePackManager::new(&tmp);
        let packs = mgr.scan().unwrap();
        assert_eq!(packs.len(), 2);
        let faithful = packs.iter().find(|p| p.name == "faithful").unwrap();
        assert_eq!(faithful.pack_format, Some(8));
        let mc = coerce_mc_version("1.18.2").unwrap();
        assert!(faithful.is_compatible("1.18.2"));
        assert!(pack_format_compatible(8, &mc));
        assert!(!pack_format_compatible(1, &mc));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn pack_enable_disable() {
        let tmp = std::env::temp_dir().join(format!("rc_rp_test2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        write_pack(&tmp, "p", 15);
        let mgr = ResourcePackManager::new(&tmp);
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

    #[test]
    fn pack_format_table_sanity() {
        assert_eq!(pack_format_range(15), Some((1, 21, 0, 1, 21, 1)));
        assert_eq!(pack_format_range(99), None);
        let v121 = coerce_mc_version("1.21.0").unwrap();
        assert!(pack_format_compatible(15, &v121));
    }
}
