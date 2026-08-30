//! Built-in game-version aliases and unlisted-version database (task 7).
//!
//! FCL ships `assets/game/{version-alias.csv, unlisted-versions.json,
//! versions.txt}` so the version picker can offer lab / preview / special builds
//! that are absent from Mojang's official `version_manifest.json`. RC previously
//! only parsed the official manifest, so the picker silently dropped every
//! version that was not in it.
//!
//! This module restores parity by reusing FCL's **exact** static data (extracted
//! byte-for-byte from the FCL APK and compiled in via `include_str!`), plus a
//! small RC-only enhancement (short major.minor aliases). The data files live in
//! `version_data/`:
//!
//! * `version-alias.csv` — FCL's multi-column alias table. Each row is
//!   `canonical_id, alias1, alias2, ...`: the first column is the real version
//!   id used to fetch the `version.json`; every later column is a display / search
//!   alias that maps *to* that canonical id (e.g. `1.14_combat-212796,1.14.3 -
//!   Combat Test` and `2.0,2.0_red,2.0_blue,2.0_purple`).
//! * `versions.txt` — FCL's complete list of unlisted version ids (~936 ids:
//!   old weekly snapshots, combat tests, April-Fools editions, ...). One id per
//!   line.
//! * `unlisted-versions.json` — FCL's `{"versions":[{id,type,url,time,
//!   releaseTime}]}` metadata for the ~205 unlisted builds that need an explicit,
//!   verified `version.json` URL (real piston-meta packages and the community
//!   zkitefly mirror).
//!
//! [`VersionAliasDb`] resolves a typed alias to the canonical id;
//! [`UnlistedVersionDb`] is the union of `versions.txt` (with best-effort derived
//! URLs) and `unlisted-versions.json` (with explicit URLs), each entry expressed
//! as a [`VersionEntry`] so it drops straight into a [`VersionManifest`].
//!
//! [`VersionManifest`] gains [`VersionManifest::resolve_alias`] and
//! [`VersionManifest::with_builtin_unlisted`], so the "complete and
//! auto-updating version list" covers everything the official manifest misses.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::game::manifest::{VersionEntry, VersionManifest};

/// Canonical Mojang meta host used for per-version `version.json` URLs so the
/// mirror system (task 3) can rewrite them onto BMCLAPI / MCBBS / Aliyun.
const PISTON_META_HOST: &str = "https://piston-meta.mojang.com";
/// Path template for a per-version `version.json` on the Mojang meta CDN. The
/// `id` is embedded twice (package id + file name), matching Mojang's layout.
/// Used only for `versions.txt` ids that have no explicit URL in
/// `unlisted-versions.json`; the mirror fallback + graceful error handling in
/// task 4 absorb the case where a specific old build's JSON is not hosted.
const VERSION_JSON_TEMPLATE: &str = "{host}/v1/packages/{id}/{id}.json";

/// FCL's compiled-in alias CSV (`canonical_id,alias,...`), byte-identical to FCL.
const VERSION_ALIAS_CSV: &str = include_str!("version_data/version-alias.csv");
/// RC-only enhancement: short major.minor -> latest patch release, HMCL/FCL style.
/// Written in the SAME `canonical_id,alias` order as FCL, so `1.20` resolves to
/// `1.20.4`.
const RC_VERSION_ALIASES: &str = "\
1.0.0,1.0
1.1.0,1.1
1.2.5,1.2
1.3.2,1.3
1.4.7,1.4
1.5.2,1.5
1.6.4,1.6
1.7.10,1.7
1.8.9,1.8
1.9.4,1.9
1.10.2,1.10
1.11.2,1.11
1.12.2,1.12
1.13.2,1.13
1.14.4,1.14
1.15.2,1.15
1.16.5,1.16
1.17.1,1.17
1.18.2,1.18
1.19.4,1.19
1.20.4,1.20
1.21.4,1.21
";
/// FCL's compiled-in list of unlisted version ids (one per line).
const EXTRA_VERSIONS_TXT: &str = include_str!("version_data/versions.txt");
/// FCL's compiled-in unlisted-version metadata (`{"versions":[...]}`).
const UNLISTED_VERSIONS_JSON: &str = include_str!("version_data/unlisted-versions.json");

/// One metadata record from `unlisted-versions.json`.
///
/// Every field except `id`/`url` is optional so a partial mirror of the upstream
/// data file still maps the ids, just with less detail (robustness).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct UnlistedMetaEntry {
    pub id: String,
    #[serde(default = "default_special_kind")]
    pub r#type: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default, rename = "releaseTime")]
    pub release_time: Option<String>,
}

/// The `{"versions":[...]}` envelope of `unlisted-versions.json`.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct UnlistedMetaFile {
    #[serde(default)]
    pub versions: Vec<UnlistedMetaEntry>,
}

fn default_special_kind() -> String {
    "special".to_string()
}

/// A resolved alias map: `alias -> canonical id`, plus a reverse `canonical ->
/// display name` table for UI labelling.
///
/// Resolution follows alias chains to a fixed point and guards against cycles,
/// so a malformed upstream file can never cause an infinite loop.
#[derive(Debug, Clone, Default)]
pub struct VersionAliasDb {
    /// `alias -> canonical id`.
    map: HashMap<String, String>,
    /// `canonical id -> first display/alias name` (reverse lookup for UI).
    display: HashMap<String, String>,
}

impl VersionAliasDb {
    /// Build from the compiled-in FCL CSV + the RC short-version enhancement.
    pub fn from_builtin() -> Self {
        let mut db = Self::default();
        db.merge_csv(VERSION_ALIAS_CSV);
        db.merge_csv(RC_VERSION_ALIASES);
        db
    }

    /// Build from explicit CSV text (used by tests and by APK-asset overrides).
    pub fn from_csv(csv: &str) -> Self {
        let mut db = Self::default();
        db.merge_csv(csv);
        db
    }

    /// Merge CSV rows of the form `canonical_id,alias1,alias2,...` into the maps.
    fn merge_csv(&mut self, csv: &str) {
        for line in csv.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() < 2 {
                continue; // need at least canonical + one alias
            }
            let canonical = parts[0].trim();
            if canonical.is_empty() {
                continue;
            }
            // First alias (if any) doubles as the canonical's display name.
            if let Some(first) = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                self.display
                    .entry(canonical.to_string())
                    .or_insert_with(|| first.to_string());
            }
            for alias in &parts[1..] {
                let alias = alias.trim();
                if !alias.is_empty() {
                    // Later rows do not overwrite an earlier canonical mapping.
                    self.map
                        .entry(alias.to_string())
                        .or_insert_with(|| canonical.to_string());
                }
            }
        }
    }

    /// Resolve `id` to its canonical version id, following alias chains until a
    /// fixed point. Returns `None` when `id` is not a known alias.
    pub fn resolve(&self, id: &str) -> Option<&str> {
        let mut current: &str = id;
        let mut first: Option<&str> = None;
        let mut last: Option<&str> = None;
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
        loop {
            match self.map.get(current) {
                Some(next) => {
                    if first.is_none() {
                        first = Some(next);
                    }
                    if !visited.insert(current.to_string()) {
                        // Cycle detected: report the first resolved alias rather
                        // than looping forever (robust against a broken DB).
                        return first;
                    }
                    last = Some(next);
                    current = next;
                }
                None => return last,
            }
        }
    }

    /// Direct lookup of an alias (no chain following).
    pub fn get(&self, alias: &str) -> Option<&str> {
        self.map.get(alias).map(|s| s.as_str())
    }

    /// The display name for a canonical id (reverse lookup), if known.
    pub fn display_name(&self, canonical_id: &str) -> Option<&str> {
        self.display.get(canonical_id).map(|s| s.as_str())
    }

    /// Whether `id` is registered as an alias key.
    pub fn is_alias(&self, id: &str) -> bool {
        self.map.contains_key(id)
    }

    /// Total number of alias rules.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the database is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// The unlisted-version database: every build not in the official manifest.
#[derive(Debug, Clone, Default)]
pub struct UnlistedVersionDb {
    entries: Vec<VersionEntry>,
    index: HashMap<String, usize>,
}

impl UnlistedVersionDb {
    /// Build from the compiled-in `versions.txt` + `unlisted-versions.json`.
    pub fn from_builtin() -> Self {
        Self::from_texts(EXTRA_VERSIONS_TXT, UNLISTED_VERSIONS_JSON)
    }

    /// Build from explicit id-list + metadata texts (APK-asset overrides).
    pub fn from_texts(ids_txt: &str, meta_json: &str) -> Self {
        let file: UnlistedMetaFile = serde_json::from_str(meta_json).unwrap_or_default();
        // id -> explicit metadata (json wins for url/type when ids overlap).
        let meta: HashMap<String, UnlistedMetaEntry> = file
            .versions
            .iter()
            .map(|e| (e.id.clone(), e.clone()))
            .collect();

        let mut db = UnlistedVersionDb::default();

        // 1) versions.txt order: explicit metadata when present, else derive.
        for line in ids_txt.lines() {
            let id = line.trim();
            if id.is_empty() || id.starts_with('#') {
                continue;
            }
            if let Some(m) = meta.get(id) {
                db.add(VersionEntry {
                    id: m.id.clone(),
                    kind: m.r#type.clone(),
                    url: m.url.clone(),
                    sha1: m.sha1.clone(),
                    time: m.time.clone(),
                    release_time: m.release_time.clone(),
                });
            } else {
                db.add(VersionEntry {
                    id: id.to_string(),
                    kind: infer_type(id),
                    url: version_json_url(id),
                    sha1: None,
                    time: None,
                    release_time: None,
                });
            }
        }

        // 2) json-only entries (ids present in unlisted-versions.json but not in
        //    versions.txt, e.g. the `2.0_*` colour variants) — keep their URLs.
        for e in &file.versions {
            if db.index.contains_key(&e.id) {
                continue;
            }
            db.add(VersionEntry {
                id: e.id.clone(),
                kind: e.r#type.clone(),
                url: e.url.clone(),
                sha1: e.sha1.clone(),
                time: e.time.clone(),
                release_time: e.release_time.clone(),
            });
        }

        db
    }

    /// Append an entry, de-duplicating by id (first occurrence wins).
    fn add(&mut self, e: VersionEntry) {
        if self.index.contains_key(&e.id) {
            return;
        }
        self.index.insert(e.id.clone(), self.entries.len());
        self.entries.push(e);
    }

    /// All unlisted version entries (versions.txt order, json-only appended).
    pub fn entries(&self) -> &[VersionEntry] {
        &self.entries
    }

    /// Find an unlisted entry by exact id.
    pub fn find(&self, id: &str) -> Option<&VersionEntry> {
        self.index.get(id).map(|&i| &self.entries[i])
    }

    /// Whether `id` is an unlisted (non-manifest) version.
    pub fn contains(&self, id: &str) -> bool {
        self.index.contains_key(id)
    }

    /// Number of unlisted entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the database is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Merge every unlisted entry whose id is NOT already present in `manifest`
    /// into `manifest.versions`. Returns the number of entries appended.
    ///
    /// Existing manifest ids always win, so a real Mojang release can never be
    /// shadowed by a built-in stub.
    pub fn merge_into(&self, manifest: &mut VersionManifest) -> usize {
        // Owned id set so the immutable borrow of `manifest.versions` ends before
        // we push into it.
        let existing: std::collections::HashSet<String> =
            manifest.versions.iter().map(|v| v.id.clone()).collect();
        let mut added = 0;
        for e in &self.entries {
            if !existing.contains(&e.id) {
                manifest.versions.push(e.clone());
                added += 1;
            }
        }
        added
    }
}

/// Infer a display `kind` for a `versions.txt`-only id that has no explicit
/// metadata. Best-effort so the version list can be grouped; falls back to
/// `"special"` for anything unrecognised.
fn infer_type(id: &str) -> String {
    let bytes = id.as_bytes();
    // Weekly snapshots, e.g. `11w47a`, `12w01a`.
    if bytes.len() >= 5
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2] == b'w'
        && bytes[3].is_ascii_digit()
        && bytes[4].is_ascii_digit()
    {
        return "snapshot".to_string();
    }
    let second_is_digit = id
        .chars()
        .nth(1)
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false);
    if id.starts_with('a') && second_is_digit {
        return "old_alpha".to_string();
    }
    if id.starts_with('b') && second_is_digit {
        return "old_beta".to_string();
    }
    if id.starts_with("c0.") {
        return "old_alpha".to_string();
    }
    if id.contains("-pre") || id.contains("-rc") || id.contains("Pre") || id.contains("RC") {
        return "snapshot".to_string();
    }
    "special".to_string()
}

/// Build the canonical per-version `version.json` URL for an unlisted id so the
/// mirror system can rewrite it onto a China-mainland CDN.
fn version_json_url(id: &str) -> String {
    VERSION_JSON_TEMPLATE
        .replace("{host}", PISTON_META_HOST)
        .replace("{id}", id)
}

// ---- process-wide singletons, parsed once (lazily) ----

pub(crate) fn alias_db() -> &'static VersionAliasDb {
    static DB: OnceLock<VersionAliasDb> = OnceLock::new();
    DB.get_or_init(VersionAliasDb::from_builtin)
}

pub(crate) fn unlisted_db() -> &'static UnlistedVersionDb {
    static DB: OnceLock<UnlistedVersionDb> = OnceLock::new();
    DB.get_or_init(UnlistedVersionDb::from_builtin)
}

/// Resolve a user-typed alias to a canonical version id (free function, no
/// manifest needed). Returns `None` when `id` is not a known alias.
pub fn resolve_version_alias(id: &str) -> Option<&'static str> {
    alias_db().resolve(id)
}

/// All built-in unlisted version entries (FCL parity data).
pub fn builtin_unlisted_versions() -> &'static [VersionEntry] {
    unlisted_db().entries()
}

/// Whether `id` is a built-in unlisted (non-manifest) version.
pub fn is_unlisted_version(id: &str) -> bool {
    unlisted_db().contains(id)
}

/// The display name for a canonical version id, if the alias DB knows one.
pub fn display_name_for(id: &str) -> Option<&'static str> {
    alias_db().display_name(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CSV: &str = "\
# comment
1.20.4,1.20
1.21.4,1.21
1.21.4,combat
";

    const SAMPLE_IDS: &str = "\
# ids
1.RV-Pre1
3D Shareware 1.34
1.20.4 - Combat Test 99
";

    const SAMPLE_JSON: &str = r#"{
        "versions": [
            {"id": "1.RV-Pre1", "type": "special", "url": "https://piston-meta.mojang.com/v1/packages/x/1.RV-Pre1.json"},
            {"id": "3D Shareware 1.34", "type": "special", "url": "https://example.test/custom.json"}
        ]
    }"#;

    #[test]
    fn parse_alias_csv_multi_column() {
        // FCL's real format: canonical_id,alias1,alias2,...
        let db = VersionAliasDb::from_csv(
            "1.14_combat-212796,1.14.3 - Combat Test\n2.0,2.0_red,2.0_blue,2.0_purple\n",
        );
        assert_eq!(db.get("1.14.3 - Combat Test"), Some("1.14_combat-212796"));
        assert_eq!(db.get("2.0_red"), Some("2.0"));
        assert_eq!(db.get("2.0_blue"), Some("2.0"));
        assert_eq!(db.get("2.0_purple"), Some("2.0"));
        // reverse display lookup
        assert_eq!(
            db.display_name("1.14_combat-212796"),
            Some("1.14.3 - Combat Test")
        );
    }

    #[test]
    fn parse_alias_csv_sample() {
        let db = VersionAliasDb::from_csv(SAMPLE_CSV);
        // canonical-first convention: column 0 is the real id, column 1 is the
        // alias that resolves *to* it.
        assert_eq!(db.get("1.20"), Some("1.20.4"));
        assert_eq!(db.get("1.21"), Some("1.21.4"));
        assert_eq!(db.get("combat"), Some("1.21.4"));
        assert!(db.is_alias("1.20"));
        assert!(!db.is_alias("1.20.4"));
    }

    #[test]
    fn resolve_alias_chain_and_noop() {
        let db = VersionAliasDb::from_csv("1.21.4,combat\n1.21.4,1.21\n");
        assert_eq!(db.resolve("combat"), Some("1.21.4"));
        assert_eq!(db.resolve("1.21"), Some("1.21.4"));
        assert_eq!(db.resolve("1.21.4"), None);
    }

    #[test]
    fn resolve_alias_cycle_is_safe() {
        // A deliberately broken alias file with a cycle must not loop forever.
        let db = VersionAliasDb::from_csv("a,b\nb,a\n");
        assert_eq!(db.resolve("a"), Some("b"));
        assert_eq!(db.resolve("b"), Some("a"));
    }

    #[test]
    fn builtin_alias_resolves_fcl_and_rc() {
        // RC short-version enhancement.
        assert_eq!(resolve_version_alias("1.20"), Some("1.20.4"));
        assert_eq!(resolve_version_alias("1.7"), Some("1.7.10"));
        assert_eq!(resolve_version_alias("1.21"), Some("1.21.4"));
        // FCL combat/experimental aliases.
        assert_eq!(
            resolve_version_alias("1.14.3 - Combat Test"),
            Some("1.14_combat-212796")
        );
        // A real version id is not an alias.
        assert_eq!(resolve_version_alias("1.20.4"), None);
    }

    #[test]
    fn parse_unlisted_db_array_format() {
        let db = UnlistedVersionDb::from_texts(SAMPLE_IDS, SAMPLE_JSON);
        // 2 from json + 1 from versions.txt (not in json) = 3
        assert_eq!(db.len(), 3);
        // json entry keeps its explicit url + type
        let rv = db.find("1.RV-Pre1").unwrap();
        assert_eq!(rv.kind, "special");
        assert_eq!(
            rv.url,
            "https://piston-meta.mojang.com/v1/packages/x/1.RV-Pre1.json"
        );
        // json entry with custom url wins over template
        let share = db.find("3D Shareware 1.34").unwrap();
        assert_eq!(share.url, "https://example.test/custom.json");
        // versions.txt-only id gets a derived url + inferred type
        let ct = db.find("1.20.4 - Combat Test 99").unwrap();
        assert_eq!(ct.kind, "special");
        assert_eq!(
            ct.url,
            "https://piston-meta.mojang.com/v1/packages/1.20.4 - Combat Test 99/1.20.4 - Combat Test 99.json"
        );
    }

    #[test]
    fn unlisted_dedup_and_union() {
        // versions.txt id also present in json -> json metadata wins (no dup).
        let ids = "alpha\nbeta\n";
        let json = r#"{"versions":[{"id":"beta","type":"old_beta","url":"https://x/beta.json"}]}"#;
        let db = UnlistedVersionDb::from_texts(ids, json);
        assert_eq!(db.len(), 2);
        assert_eq!(db.find("beta").unwrap().kind, "old_beta");
        assert_eq!(db.find("beta").unwrap().url, "https://x/beta.json");
        // json-only id appended
        assert!(db.contains("beta"));
    }

    #[test]
    fn infer_type_classifies() {
        assert_eq!(infer_type("11w47a"), "snapshot");
        assert_eq!(infer_type("a1.0.10"), "old_alpha");
        assert_eq!(infer_type("b1.8.1"), "old_beta");
        assert_eq!(infer_type("c0.0.23a_01"), "old_alpha");
        assert_eq!(infer_type("1.10-pre1"), "snapshot");
        assert_eq!(infer_type("mystery-build"), "special");
    }

    #[test]
    fn merge_into_manifest_skips_existing() {
        let mut manifest = VersionManifest {
            latest: crate::game::manifest::Latest {
                release: "1.20.4".into(),
                snapshot: "24w03a".into(),
            },
            versions: vec![VersionEntry {
                id: "1.20.4".into(),
                kind: "release".into(),
                url: "https://piston-meta.mojang.com/v1/packages/x/1.20.4.json".into(),
                sha1: None,
                time: None,
                release_time: None,
            }],
        };
        // 1.20.4 is itself in FCL's versions.txt, so it must be skipped once.
        let expected = unlisted_db().len() - unlisted_db().contains("1.20.4") as usize;
        let before = manifest.versions.len();
        let added = unlisted_db().merge_into(&mut manifest);
        assert_eq!(added, expected);
        assert_eq!(manifest.versions.len(), before + added);
        // the real release is untouched (still "release" kind)
        assert_eq!(manifest.find("1.20.4").unwrap().kind, "release");
        // an unlisted version (FCL combat build) is now discoverable in the
        // manifest, carrying whatever type FCL assigned it (e.g. "pending").
        let u = manifest
            .find("1.14_combat-212796")
            .expect("unlisted id present");
        assert!(!u.url.is_empty());
    }

    #[test]
    fn manifest_with_builtin_unlisted_and_resolve_alias() {
        let base = VersionManifest {
            latest: crate::game::manifest::Latest {
                release: "1.20.4".into(),
                snapshot: "24w03a".into(),
            },
            versions: vec![VersionEntry {
                id: "1.20.4".into(),
                kind: "release".into(),
                url: "https://piston-meta.mojang.com/v1/packages/x/1.20.4.json".into(),
                sha1: None,
                time: None,
                release_time: None,
            }],
        };
        // with_builtin_unlisted must not mutate the original (clone semantics).
        let augmented = base.with_builtin_unlisted();
        assert_eq!(base.versions.len(), 1);
        assert!(augmented.versions.len() > base.versions.len());
        // A real FCL unlisted id is now in the manifest.
        assert!(augmented.find("25w14craftmine").is_some());
        assert!(augmented.find("1.21.11_unobfuscated").is_some());
        // resolve_alias delegates to the built-in alias DB (RC + FCL).
        assert_eq!(base.resolve_alias("1.20"), Some("1.20.4".to_string()));
        assert_eq!(base.resolve_alias("1.8"), Some("1.8.9".to_string()));
        assert_eq!(
            base.resolve_alias("1.14.3 - Combat Test"),
            Some("1.14_combat-212796".to_string())
        );
    }

    #[test]
    fn builtin_db_not_empty_and_large() {
        assert!(!alias_db().is_empty(), "alias DB should be populated");
        assert!(!unlisted_db().is_empty(), "unlisted DB should be populated");
        // FCL ships ~936 ids in versions.txt + ~205 in unlisted-versions.json.
        assert!(builtin_unlisted_versions().len() >= 900);
    }
}
