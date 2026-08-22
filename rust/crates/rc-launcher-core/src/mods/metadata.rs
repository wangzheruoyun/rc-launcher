//! Mod manifest parsing (task 8).
//!
//! Each loader stores its metadata in a different embedded file / format. This
//! module turns every supported format into a single [`ModMetadata`] view so
//! the manager (and the UI) can treat all mods uniformly:
//!
//! | loader   | archive entry(s)            | format                |
//! |----------|-----------------------------|-----------------------|
//! | Fabric   | `fabric.mod.json`           | JSON (semver ranges) |
//! | Quilt    | `quilt.mod.json`            | JSON (semver ranges) |
//! | Forge    | `META-INF/mods.toml`        | TOML (Maven ranges)  |
//! | Forge    | `mcmod.info`               | JSON array (old)     |
//! | LiteLoader| `litemod.json`             | JSON                 |
//! | OptiFine | (file name)                | name-detected        |
//!
//! Parsers are deliberately *lenient*: a malformed dependency string degrades
//! to "no constraint" instead of failing the whole parse, matching FCL's
//! "best-effort metadata" philosophy.

use crate::error::{RcError, RcResult};
use crate::mods::constraint::VersionConstraint;
use crate::mods::loader::ModLoader;
use std::collections::BTreeMap;

/// A single dependency / conflict edge on another mod or on Minecraft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModDependency {
    /// Target mod id (`"minecraft"` is the special Minecraft version edge).
    pub modid: String,
    /// Optional version range; `None` means "any version".
    pub range: Option<VersionConstraint>,
    /// `true` for hard dependencies (`depends` / `requiredMods`); `false` for
    /// soft ones (`recommends` / `suggests` / `breaks=false`).
    pub required: bool,
}

impl ModDependency {
    /// Convenience constructor.
    pub fn new(modid: impl Into<String>, range: Option<VersionConstraint>, required: bool) -> Self {
        Self {
            modid: modid.into(),
            range,
            required,
        }
    }
}

/// Normalised, loader-agnostic view of one mod's metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModMetadata {
    /// Canonical mod id (e.g. `sodium`, `examplemod`).
    pub modid: String,
    /// Human-friendly display name.
    pub name: String,
    /// Declared version string (raw, as written in the manifest).
    pub version: Option<String>,
    /// Short description / summary.
    pub description: Option<String>,
    /// Authors / credits.
    pub authors: Vec<String>,
    /// Loader this manifest belongs to.
    pub loader: ModLoader,
    /// Minecraft version constraint this mod requires.
    pub minecraft: Option<VersionConstraint>,
    /// Hard + soft mod dependencies (`depends` / `requiredMods`).
    pub dependencies: Vec<ModDependency>,
    /// Hard conflicts (`breaks` / `conflicts`).
    pub conflicts: Vec<ModDependency>,
    /// Soft recommendations (`recommends` / `suggests`).
    pub recommends: Vec<ModDependency>,
}

impl ModMetadata {
    /// Build an empty metadata skeleton for `loader`.
    pub fn empty(loader: ModLoader) -> Self {
        Self {
            modid: String::new(),
            name: String::new(),
            version: None,
            description: None,
            authors: Vec::new(),
            loader,
            minecraft: None,
            dependencies: Vec::new(),
            conflicts: Vec::new(),
            recommends: Vec::new(),
        }
    }

    /// Parse a manifest for the given loader from raw archive entry text.
    ///
    /// For loaders that can declare multiple mods in one file (Forge
    /// `mods.toml` / `mcmod.info`) this returns one entry per mod.
    pub fn parse_for(loader: ModLoader, content: &str) -> RcResult<Vec<ModMetadata>> {
        match loader {
            ModLoader::Fabric => Ok(vec![parse_fabric(content)?]),
            ModLoader::Quilt => Ok(vec![parse_quilt(content)?]),
            ModLoader::Forge => parse_forge(content),
            ModLoader::LiteLoader => Ok(vec![parse_litemod(content)?]),
            ModLoader::OptiFine => Err(RcError::Other(
                "OptiFine has no embedded manifest; use ModMetadata::optifine_from_name".into(),
            )),
            ModLoader::Vanilla => Err(RcError::Other("vanilla content has no mod manifest".into())),
        }
    }

    /// Construct OptiFine metadata purely from the file name
    /// (`OptiFine_1.16.5_HD_U_G8.jar` → id `optifine`, mc `1.16.5`).
    pub fn optifine_from_name(file_name: &str) -> Self {
        let base = file_name
            .rsplit('/')
            .next()
            .unwrap_or(file_name)
            .trim_end_matches(".jar")
            .trim_end_matches(".disabled")
            .trim_end_matches('!');
        let mut m = ModMetadata::empty(ModLoader::OptiFine);
        m.modid = "optifine".into();
        m.name = "OptiFine".into();
        // OptiFine_1.16.5_HD_U_G8 → version "1.16.5_HD_U_G8", mc "1.16.5".
        if let Some(rest) = base
            .strip_prefix("OptiFine_")
            .or_else(|| base.strip_prefix("optifine_"))
        {
            let (mc, qual) = match rest.split_once('_') {
                Some((mc, qual)) => (mc.to_string(), Some(qual.to_string())),
                None => (rest.to_string(), None),
            };
            m.version = Some(match &qual {
                Some(q) => format!("{mc}_{q}"),
                None => mc.clone(),
            });
            m.minecraft = VersionConstraint::parse(&mc)
                .ok()
                .or_else(|| VersionConstraint::parse(&format!("={mc}")).ok());
        } else {
            m.version = Some(base.to_string());
        }
        m
    }
}

// ---------------------------------------------------------------------------
// Fabric / Quilt
// ---------------------------------------------------------------------------

fn parse_fabric(content: &str) -> RcResult<ModMetadata> {
    let v: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| RcError::Other(format!("invalid fabric.mod.json: {e}")))?;
    let obj = v
        .as_object()
        .ok_or_else(|| RcError::Other("fabric.mod.json is not an object".into()))?;

    let mut m = ModMetadata::empty(ModLoader::Fabric);
    m.modid = string_field(obj, "id").unwrap_or_default();
    m.name = string_field(obj, "name").unwrap_or_else(|| m.modid.clone());
    m.version = string_field(obj, "version");
    m.description = string_field(obj, "description");
    m.authors = string_or_array(obj.get("authors"));

    if let Some(provides) = array_field(obj, "provides") {
        // `provides` is informational; we keep it as soft recommendations of
        // the same id so alternate ids don't trip conflict detection.
        for p in provides {
            if let Some(id) = p.as_str() {
                m.recommends.push(ModDependency::new(id, None, false));
            }
        }
    }

    if let Some(map) = obj.get("depends").and_then(|v| v.as_object()) {
        apply_dep_map(&mut m, map, true);
    }
    if let Some(map) = obj.get("breaks").and_then(|v| v.as_object()) {
        apply_breaks(&mut m, map);
    }
    if let Some(map) = obj.get("conflicts").and_then(|v| v.as_object()) {
        apply_breaks(&mut m, map);
    }
    if let Some(map) = obj.get("recommends").and_then(|v| v.as_object()) {
        apply_dep_map(&mut m, map, false);
    }
    if let Some(map) = obj.get("suggests").and_then(|v| v.as_object()) {
        apply_dep_map(&mut m, map, false);
    }
    Ok(m)
}

fn parse_quilt(content: &str) -> RcResult<ModMetadata> {
    let v: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| RcError::Other(format!("invalid quilt.mod.json: {e}")))?;

    // Quilt may nest everything under `quilt_loader`; fall back to the flat
    // (fabric-like) layout otherwise.
    let obj = if let Some(ql) = v.get("quilt_loader").and_then(|x| x.as_object()) {
        // Merge nested `metadata` into the loader object for uniform access.
        let mut merged = ql.clone();
        if let Some(meta) = ql.get("metadata").and_then(|x| x.as_object()) {
            for (k, val) in meta {
                merged.entry(k.clone()).or_insert(val.clone());
            }
        }
        merged
    } else if let Some(o) = v.as_object() {
        o.clone()
    } else {
        return Err(RcError::Other("quilt.mod.json is not an object".into()));
    };

    let mut m = ModMetadata::empty(ModLoader::Quilt);
    m.modid = string_field(&obj, "id").unwrap_or_default();
    m.name = string_field(&obj, "name").unwrap_or_else(|| m.modid.clone());
    m.version = string_field(&obj, "version");
    m.description = string_field(&obj, "description");
    m.authors = string_or_array(obj.get("authors"));

    if let Some(map) = obj.get("depends").and_then(|v| v.as_object()) {
        apply_dep_map(&mut m, map, true);
    }
    if let Some(map) = obj.get("breaks").and_then(|v| v.as_object()) {
        apply_breaks(&mut m, map);
    }
    if let Some(map) = obj.get("conflicts").and_then(|v| v.as_object()) {
        apply_breaks(&mut m, map);
    }
    if let Some(map) = obj.get("recommends").and_then(|v| v.as_object()) {
        apply_dep_map(&mut m, map, false);
    }
    if let Some(map) = obj.get("suggests").and_then(|v| v.as_object()) {
        apply_dep_map(&mut m, map, false);
    }
    Ok(m)
}

/// Apply a Fabric/Quilt `depends` / `recommends` / `suggests` map onto
/// `m.dependencies` / `m.recommends`. The `minecraft` edge is stored on the
/// metadata directly rather than as a mod dependency.
fn apply_dep_map(
    m: &mut ModMetadata,
    map: &serde_json::Map<String, serde_json::Value>,
    required: bool,
) {
    for (key, val) in map {
        let entries = match val {
            serde_json::Value::String(s) => vec![s.clone()],
            serde_json::Value::Array(a) => a
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect(),
            _ => continue,
        };
        for range_txt in entries {
            let range = VersionConstraint::parse(&range_txt).ok();
            if key == "minecraft" {
                if m.minecraft.is_none() {
                    m.minecraft = range;
                }
                continue;
            }
            m.dependencies
                .push(ModDependency::new(key.clone(), range, required));
        }
    }
}

/// Apply a Fabric/Quilt `breaks` / `conflicts` map onto `m.conflicts`.
fn apply_breaks(m: &mut ModMetadata, map: &serde_json::Map<String, serde_json::Value>) {
    for (key, val) in map {
        let entries = match val {
            serde_json::Value::String(s) => vec![s.clone()],
            serde_json::Value::Array(a) => a
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect(),
            _ => continue,
        };
        for range_txt in entries {
            let range = VersionConstraint::parse(&range_txt).ok();
            m.conflicts
                .push(ModDependency::new(key.clone(), range, true));
        }
    }
}

// ---------------------------------------------------------------------------
// Forge (mods.toml / mcmod.info)
// ---------------------------------------------------------------------------

fn parse_forge(content: &str) -> RcResult<Vec<ModMetadata>> {
    let trimmed = content.trim_start();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        parse_forge_mcmod_info(content)
    } else {
        parse_forge_mods_toml(content)
    }
}

fn parse_forge_mods_toml(content: &str) -> RcResult<Vec<ModMetadata>> {
    let doc: toml::Value =
        toml::from_str(content).map_err(|e| RcError::Other(format!("invalid mods.toml: {e}")))?;

    // table: `dependencies` -> `<modId>` -> array of dependency rows.
    let mut deps: BTreeMap<String, Vec<ModDependency>> = BTreeMap::new();
    if let Some(dt) = doc.get("dependencies").and_then(|v| v.as_table()) {
        for (modid, arr) in dt {
            if let Some(arr) = arr.as_array() {
                for entry in arr {
                    if let Some(dep_modid) = entry.get("modId").and_then(|x| x.as_str()) {
                        let required = entry
                            .get("type")
                            .and_then(|x| x.as_str())
                            .map(|t| t.eq_ignore_ascii_case("required"))
                            .unwrap_or(true);
                        let range = entry
                            .get("versionRange")
                            .and_then(|x| x.as_str())
                            .and_then(|st| VersionConstraint::parse(st).ok());
                        deps.entry(modid.to_string())
                            .or_default()
                            .push(ModDependency::new(dep_modid, range, required));
                    }
                }
            }
        }
    }

    let mods = doc
        .get("mods")
        .and_then(|v| v.as_array())
        .ok_or_else(|| RcError::Other("mods.toml has no [[mods]]".into()))?;

    let mut out = Vec::new();
    for mval in mods {
        let obj = match mval.as_table() {
            Some(o) => o,
            None => continue,
        };
        let modid = toml_string(obj, "modId").unwrap_or_default();
        let mut m = ModMetadata::empty(ModLoader::Forge);
        m.modid = modid.clone();
        m.name = toml_string(obj, "displayName").unwrap_or_else(|| modid.clone());
        m.version = toml_string(obj, "version");
        m.description = toml_string(obj, "description");
        if let Some(a) = toml_string(obj, "authors") {
            m.authors = a.split(',').map(|s| s.trim().to_string()).collect();
        } else if let Some(a) = obj.get("authors").and_then(|x| x.as_array()) {
            m.authors = a
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect();
        }
        if let Some(list) = deps.get(&modid) {
            for d in list {
                if d.modid == "minecraft" {
                    if m.minecraft.is_none() {
                        m.minecraft = d.range.clone();
                    }
                } else if d.modid == "forge" {
                    // Forge-as-dependency; keep as a soft edge so it doesn't
                    // produce false "missing dependency" if we can't resolve it.
                    m.recommends
                        .push(ModDependency::new("forge", d.range.clone(), d.required));
                } else {
                    m.dependencies.push(d.clone());
                }
            }
        }
        out.push(m);
    }
    Ok(out)
}

/// toml string helper.
fn toml_string(obj: &toml::map::Map<String, toml::Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn parse_forge_mcmod_info(content: &str) -> RcResult<Vec<ModMetadata>> {
    let v: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| RcError::Other(format!("invalid mcmod.info: {e}")))?;

    // Accept both `[ {...}, {...} ]` and `{ "modList": [ {...} ] }`.
    let arr = match &v {
        serde_json::Value::Array(a) => a.clone(),
        serde_json::Value::Object(o) => o
            .get("modList")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => {
            return Err(RcError::Other(
                "mcmod.info is neither an array nor {modList:[]}".into(),
            ))
        }
    };

    let mut out = Vec::new();
    for mval in arr {
        let obj = match mval.as_object() {
            Some(o) => o,
            None => continue,
        };
        let modid = string_field(obj, "modid").unwrap_or_default();
        let mut m = ModMetadata::empty(ModLoader::Forge);
        m.modid = modid.clone();
        m.name = string_field(obj, "name").unwrap_or_else(|| modid.clone());
        m.version = string_field(obj, "version");
        m.description = string_field(obj, "description");
        if let Some(authors) = array_field(obj, "authorList") {
            m.authors = authors
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect();
        } else if let Some(a) = string_field(obj, "author") {
            m.authors = vec![a];
        }
        if let Some(mc) = string_field(obj, "mcversion") {
            m.minecraft = VersionConstraint::parse(&mc)
                .ok()
                .or_else(|| VersionConstraint::parse(&format!("={mc}")).ok());
        }
        for key in ["requiredMods", "dependencies"] {
            if let Some(list) = obj.get(key).and_then(|x| x.as_array()) {
                let required = key == "requiredMods";
                for item in list {
                    if let Some(s) = item.as_str() {
                        let (id, range) = split_mod_version(s);
                        m.dependencies.push(ModDependency::new(id, range, required));
                    }
                }
            }
        }
        out.push(m);
    }
    Ok(out)
}

/// Forge's `requiredMods` entries may be `modid` or `modid@version`.
fn split_mod_version(s: &str) -> (String, Option<VersionConstraint>) {
    if let Some((id, ver)) = s.split_once('@') {
        let range = VersionConstraint::parse(ver)
            .ok()
            .or_else(|| VersionConstraint::parse(&format!("={ver}")).ok());
        (id.to_string(), range)
    } else {
        (s.to_string(), None)
    }
}

// ---------------------------------------------------------------------------
// LiteLoader
// ---------------------------------------------------------------------------

fn parse_litemod(content: &str) -> RcResult<ModMetadata> {
    let v: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| RcError::Other(format!("invalid litemod.json: {e}")))?;
    let obj = v
        .as_object()
        .ok_or_else(|| RcError::Other("litemod.json is not an object".into()))?;

    let name = string_field(obj, "name").unwrap_or_default();
    let mut m = ModMetadata::empty(ModLoader::LiteLoader);
    // LiteLoader has no stable modid; derive one from the name.
    m.modid = sanitize_id(&name);
    m.name = name;
    m.version = string_field(obj, "version");
    m.description = string_field(obj, "description");
    if let Some(a) = string_field(obj, "author") {
        m.authors = vec![a];
    }
    if let Some(mc) = string_field(obj, "mcversion") {
        m.minecraft = VersionConstraint::parse(&mc)
            .ok()
            .or_else(|| VersionConstraint::parse(&format!("={mc}")).ok());
    }
    Ok(m)
}

/// Replace any non `[A-Za-z0-9_]` run with `_` so a display name can act as an
/// id for loaders that lack one (LiteLoader).
fn sanitize_id(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_underscore = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
    }
    out.trim_matches('_').to_string()
}

// ---------------------------------------------------------------------------
// small JSON helpers
// ---------------------------------------------------------------------------

fn string_field(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn array_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<Vec<serde_json::Value>> {
    obj.get(key).and_then(|v| v.as_array()).cloned()
}

fn string_or_array(v: Option<&serde_json::Value>) -> Vec<String> {
    match v {
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fabric_parse() {
        let json = r#"{
            "schemaVersion": 1,
            "id": "sodium",
            "version": "0.4.4",
            "name": "Sodium",
            "authors": ["CaffeineMC"],
            "depends": { "minecraft": ">=1.16", "fabricloader": ">=0.10.0" },
            "breaks": { "optifine": "*" }
        }"#;
        let m = ModMetadata::parse_for(ModLoader::Fabric, json).unwrap();
        assert_eq!(m.len(), 1);
        let m = &m[0];
        assert_eq!(m.modid, "sodium");
        assert_eq!(m.loader, ModLoader::Fabric);
        assert!(m.minecraft.as_ref().unwrap().matches("1.18.2"));
        assert!(!m.minecraft.as_ref().unwrap().matches("1.12.2"));
        assert_eq!(m.dependencies.len(), 1); // fabricloader (not minecraft)
        assert_eq!(m.conflicts.len(), 1);
        assert_eq!(m.conflicts[0].modid, "optifine");
    }

    #[test]
    fn quilt_nested_parse() {
        let json = r#"{
            "schemaVersion": 1,
            "quilt_loader": {
                "id": "my-mod",
                "version": "1.0",
                "metadata": { "name": "My Mod" },
                "depends": { "minecraft": ">=1.14", "quilt_loader": ">=0.16.0" }
            }
        }"#;
        let m = ModMetadata::parse_for(ModLoader::Quilt, json).unwrap();
        let m = &m[0];
        assert_eq!(m.modid, "my-mod");
        assert_eq!(m.name, "My Mod");
        assert!(m.minecraft.as_ref().unwrap().matches("1.16.5"));
    }

    #[test]
    fn forge_toml_parse() {
        let toml = r#"
            modLoader = "javafml"
            loaderVersion = "[24,)"

            [[mods]]
            modId = "examplemod"
            version = "1.0.0"
            displayName = "Example Mod"
            authors = "Alice"

            [[dependencies.examplemod]]
            modId = "forge"
            type = "REQUIRED"
            versionRange = "[24,)"
            ordering = "NONE"
            side = "BOTH"

            [[dependencies.examplemod]]
            modId = "minecraft"
            type = "REQUIRED"
            versionRange = "[1.16.5]"
        "#;
        let m = parse_forge(toml).unwrap();
        assert_eq!(m.len(), 1);
        let m = &m[0];
        assert_eq!(m.modid, "examplemod");
        assert_eq!(m.loader, ModLoader::Forge);
        assert_eq!(m.authors, vec!["Alice"]);
        assert!(m.minecraft.as_ref().unwrap().matches("1.16.5"));
        // forge is stored as a soft edge.
        assert!(m.recommends.iter().any(|d| d.modid == "forge"));
    }

    #[test]
    fn forge_mcmod_info_parse() {
        let json = r#"[
            {
                "modid": "oldmod",
                "name": "Old Mod",
                "version": "2.3",
                "mcversion": "1.12.2",
                "authorList": ["Bob"],
                "requiredMods": ["basemod@1.0.0"]
            }
        ]"#;
        let m = parse_forge(json).unwrap();
        let m = &m[0];
        assert_eq!(m.modid, "oldmod");
        assert!(m.minecraft.as_ref().unwrap().matches("1.12.2"));
        assert_eq!(m.dependencies.len(), 1);
        assert_eq!(m.dependencies[0].modid, "basemod");
        assert!(m.dependencies[0].required);
    }

    #[test]
    fn litemod_parse() {
        let json =
            r#"{ "name": "Lite Mod X", "version": "1.0", "mcversion": "1.12.2", "author": "Dev" }"#;
        let m = ModMetadata::parse_for(ModLoader::LiteLoader, json).unwrap();
        let m = &m[0];
        assert_eq!(m.modid, "lite_mod_x");
        assert_eq!(m.name, "Lite Mod X");
        assert!(m.minecraft.as_ref().unwrap().matches("1.12.2"));
    }

    #[test]
    fn optifine_from_name() {
        let m = ModMetadata::optifine_from_name("OptiFine_1.16.5_HD_U_G8.jar");
        assert_eq!(m.modid, "optifine");
        assert!(m.minecraft.as_ref().unwrap().matches("1.16.5"));
        assert!(m.version.as_ref().unwrap().starts_with("1.16.5"));
    }

    #[test]
    fn malformed_dep_does_not_fail_parse() {
        let json =
            r#"{ "id": "x", "version": "1", "depends": { "minecraft": "totally-not-a-version" } }"#;
        let m = ModMetadata::parse_for(ModLoader::Fabric, json).unwrap();
        // mc constraint parse failed → None, but the mod still parsed.
        assert_eq!(m[0].minecraft, None);
    }
}
