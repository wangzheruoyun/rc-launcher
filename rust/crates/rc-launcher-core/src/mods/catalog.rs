//! Mod / modpack **metadata** and **source integration** (task 6).
//!
//! FCL ships `assets/mod_data.txt` and `assets/modpack_data.txt` as an offline
//! fallback so the Modrinth / CurseForge browsers keep working without a network.
//! RC's [`super`] module already parses per-instance loader / metadata /
//! conflicts, but it had no *data source* to browse against. This module closes
//! that gap:
//!
//! * It **embeds** FCL's offline dictionaries (`mod_data.txt` / `modpack_data.txt`,
//!   compiled in via [`include_str!`] and also shipped as APK assets for FCL
//!   parity). These are `slug;id;aliases;cn_name;en_name;abbr` records that map
//!   any mod id to a human-readable name — the offline fallback for name
//!   resolution, exposed as [`ModDictionary`].
//! * It **embeds a curated, rich overlay** (`featured_mods.tsv` /
//!   `featured_modpacks.tsv`) carrying categories, tags, supported loaders,
//!   game-version constraints and dependency edges for the most popular modern
//!   projects.
//! * [`Catalog`] merges both into one queryable set and provides **search**,
//!   **version / loader / category / environment filtering** and **dependency
//!   resolution**.
//! * [`ModrinthCatalogClient`] pulls **live catalogs** from Modrinth through the
//!   China-mainland-aware [`NetworkClient`](crate::net::NetworkClient): mirror
//!   fallback, DoH and proxy all apply automatically, so browsing stays usable on
//!   weak mainland networks. [`Catalog::augment_with_online`] merges results in.
//!
//! All types are `serde` (de)serialisable so the Compose / FFI layer can bridge
//! them to the UI as JSON.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::str::FromStr;

use serde::de::Deserializer;
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use crate::error::RcResult;
use crate::mods::constraint::VersionConstraint;
use crate::mods::loader::ModLoader;
use crate::net::NetworkClient;

/// Is this catalog entry a mod or a modpack?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectKind {
    Mod,
    Modpack,
}

/// Where a catalog entry originates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CatalogSource {
    /// Curated, compiled-in offline data (the FCL `mod_data.txt` analogue).
    Builtin,
    /// Fetched live from Modrinth.
    Modrinth,
    /// Fetched live from CurseForge (mirror).
    CurseForge,
}

/// Which side(s) a mod runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModEnvironment {
    Client,
    Server,
    Both,
}

/// Well-known Modrinth-style categories. `Other` keeps unknown slugs instead of
/// dropping them, so the taxonomy stays forward-compatible.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ModCategory {
    Technology,
    Magic,
    Adventure,
    Decoration,
    Utility,
    Library,
    Storage,
    Food,
    Combat,
    Worldgen,
    Redstone,
    Optimization,
    Cursed,
    Equipment,
    GameMechanics,
    Social,
    Economy,
    Map,
    Other(String),
}

impl ModCategory {
    /// Stable slug used in catalogs, the FCL data files and the Modrinth API.
    pub fn slug(&self) -> &str {
        match self {
            ModCategory::Technology => "technology",
            ModCategory::Magic => "magic",
            ModCategory::Adventure => "adventure",
            ModCategory::Decoration => "decoration",
            ModCategory::Utility => "utility",
            ModCategory::Library => "library",
            ModCategory::Storage => "storage",
            ModCategory::Food => "food",
            ModCategory::Combat => "combat",
            ModCategory::Worldgen => "worldgen",
            ModCategory::Redstone => "redstone",
            ModCategory::Optimization => "optimization",
            ModCategory::Cursed => "cursed",
            ModCategory::Equipment => "equipment",
            ModCategory::GameMechanics => "game-mechanics",
            ModCategory::Social => "social",
            ModCategory::Economy => "economy",
            ModCategory::Map => "map",
            ModCategory::Other(s) => s.as_str(),
        }
    }

    /// Parse a slug into a category, defaulting to [`ModCategory::Other`].
    pub fn from_slug(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "technology" => ModCategory::Technology,
            "magic" => ModCategory::Magic,
            "adventure" => ModCategory::Adventure,
            "decoration" => ModCategory::Decoration,
            "utility" => ModCategory::Utility,
            "library" => ModCategory::Library,
            "storage" => ModCategory::Storage,
            "food" => ModCategory::Food,
            "combat" => ModCategory::Combat,
            "worldgen" => ModCategory::Worldgen,
            "redstone" => ModCategory::Redstone,
            "optimization" | "performance" => ModCategory::Optimization,
            "cursed" => ModCategory::Cursed,
            "equipment" => ModCategory::Equipment,
            "game-mechanics" | "gamemechanics" => ModCategory::GameMechanics,
            "social" => ModCategory::Social,
            "economy" => ModCategory::Economy,
            "map" => ModCategory::Map,
            other => ModCategory::Other(other.to_string()),
        }
    }

    /// The full, stable list (used by the UI to render filter chips).
    pub fn all() -> Vec<ModCategory> {
        vec![
            ModCategory::Technology,
            ModCategory::Magic,
            ModCategory::Adventure,
            ModCategory::Decoration,
            ModCategory::Utility,
            ModCategory::Library,
            ModCategory::Storage,
            ModCategory::Food,
            ModCategory::Combat,
            ModCategory::Worldgen,
            ModCategory::Redstone,
            ModCategory::Optimization,
            ModCategory::Cursed,
            ModCategory::Equipment,
            ModCategory::GameMechanics,
            ModCategory::Social,
            ModCategory::Economy,
            ModCategory::Map,
        ]
    }
}

impl Serialize for ModCategory {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.slug())
    }
}

impl<'de> Deserialize<'de> for ModCategory {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Ok(ModCategory::from_slug(&raw))
    }
}

/// One browsable mod or modpack with enough metadata to search, filter and
/// resolve dependencies against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogMod {
    /// Canonical project id (e.g. `sodium`).
    pub id: String,
    /// URL slug, if known.
    pub slug: String,
    /// Display name.
    pub name: String,
    /// Short one-line summary.
    pub summary: String,
    /// Longer description (often empty for offline entries).
    #[serde(default)]
    pub description: String,
    /// Mod or modpack.
    pub kind: ProjectKind,
    /// Where this entry came from.
    pub source: CatalogSource,
    /// Curated categories.
    #[serde(default)]
    pub categories: Vec<ModCategory>,
    /// Free-form tags (also holds unmapped slugs / FCL aliases).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Client / server / both.
    pub environment: ModEnvironment,
    /// Loaders this project supports.
    #[serde(default)]
    pub loaders: Vec<ModLoader>,
    /// Concrete Minecraft versions (or ranges) it targets.
    #[serde(default)]
    pub game_versions: Vec<String>,
    /// Mod ids this project depends on.
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Total downloads (popularity signal).
    #[serde(default)]
    pub downloads: u64,
    /// Editorial "featured" flag.
    #[serde(default)]
    pub featured: bool,
    /// Icon URL (may be empty for offline entries).
    #[serde(default)]
    pub icon_url: String,
    /// Project / source page URL.
    #[serde(default)]
    pub project_url: String,
}

impl CatalogMod {
    /// Does this entry target the given Minecraft `version`?
    ///
    /// An empty version list means the version is *unknown* (typical for the
    /// offline FCL dictionary) and therefore it is **not** considered a match
    /// when a specific version is requested via [`Catalog::filter`]. Each listed
    /// entry is parsed as a [`VersionConstraint`] so both exact versions
    /// (`1.20.1`) and ranges (`>=1.16`) work.
    pub fn matches_game_version(&self, version: &str) -> bool {
        if self.game_versions.is_empty() {
            // Unknown -> compatible only when no specific version is being tested.
            return true;
        }
        self.game_versions.iter().any(|v| {
            VersionConstraint::parse(v)
                .map(|c| c.matches(version))
                .unwrap_or(false)
        })
    }

    /// Does this entry support `loader`? Empty loader lists accept nothing when a
    /// specific loader is requested (version-filter semantics).
    pub fn supports_loader(&self, loader: ModLoader) -> bool {
        self.loaders.iter().any(|l| *l == loader)
    }

    /// Does this entry run in the requested `env`?
    pub fn supports_environment(&self, env: ModEnvironment) -> bool {
        match env {
            ModEnvironment::Both => self.environment == ModEnvironment::Both,
            ModEnvironment::Client => {
                self.environment == ModEnvironment::Client
                    || self.environment == ModEnvironment::Both
            }
            ModEnvironment::Server => {
                self.environment == ModEnvironment::Server
                    || self.environment == ModEnvironment::Both
            }
        }
    }

    /// Case-insensitive substring match across id / slug / name / summary /
    /// categories / tags.
    pub fn matches_query(&self, query: &str) -> bool {
        let q = query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return true;
        }
        self.search_blob().contains(&q)
    }

    fn search_blob(&self) -> String {
        let mut s = format!(
            "{} {} {} {} {}",
            self.id, self.slug, self.name, self.summary, self.project_url
        );
        for c in &self.categories {
            s.push(' ');
            s.push_str(c.slug());
        }
        for t in &self.tags {
            s.push(' ');
            s.push_str(t);
        }
        s.to_ascii_lowercase()
    }
}

/// Filter applied by [`Catalog::filter`].
#[derive(Debug, Clone, Default)]
pub struct CatalogFilter {
    pub query: Option<String>,
    pub kind: Option<ProjectKind>,
    pub game_version: Option<String>,
    pub loader: Option<ModLoader>,
    pub category: Option<ModCategory>,
    pub environment: Option<ModEnvironment>,
    pub featured_only: bool,
}

/// The aggregated, queryable catalog (offline entries + optional merged online
/// results).
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    entries: Vec<CatalogMod>,
}

impl Catalog {
    /// An empty catalog.
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Build a catalog directly from typed entries.
    pub fn from_entries(entries: Vec<CatalogMod>) -> Self {
        Self { entries }
    }

    /// The curated, **compiled-in** offline catalog: FCL's `mod_data.txt` /
    /// `modpack_data.txt` dictionaries merged with the rich curated overlay.
    pub fn builtin() -> Self {
        Self::build_from_fcl(FCL_MOD_DATA, FCL_MODPACK_DATA)
    }

    /// Load the offline catalog from on-disk FCL-style data files (e.g. the APK
    /// assets), then enrich with the embedded curated overlay. Missing files are
    /// tolerated so callers always get a usable (if sparse) catalog.
    pub fn load_offline_data(mod_path: &Path, modpack_path: &Path) -> RcResult<Self> {
        let t1 = std::fs::read_to_string(mod_path).unwrap_or_default();
        let t2 = std::fs::read_to_string(modpack_path).unwrap_or_default();
        Ok(Self::build_from_fcl(&t1, &t2))
    }

    fn build_from_fcl(mod_text: &str, modpack_text: &str) -> Self {
        let mut entries: Vec<CatalogMod> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();

        // 1) FCL offline dictionary (id -> name fallback), kind-aware.
        for e in parse_fcl_mod_data(mod_text) {
            merge_entry(
                &mut entries,
                &mut index,
                fcl_to_catalog(e, ProjectKind::Mod),
                false,
            );
        }
        for e in parse_fcl_mod_data(modpack_text) {
            merge_entry(
                &mut entries,
                &mut index,
                fcl_to_catalog(e, ProjectKind::Modpack),
                false,
            );
        }
        // 2) Rich curated overlay (categories / loaders / versions / deps).
        for cm in parse_catalog_text(FEATURED_MODS, ProjectKind::Mod) {
            merge_entry(&mut entries, &mut index, cm, true);
        }
        for cm in parse_catalog_text(FEATURED_MODPACKS, ProjectKind::Modpack) {
            merge_entry(&mut entries, &mut index, cm, true);
        }
        Self { entries }
    }

    /// Merge another catalog's entries in (de-duplicating by id + source).
    pub fn merge(&mut self, other: Catalog) {
        let mut index: HashMap<String, usize> = self
            .entries
            .iter()
            .map(|e| e.id.clone())
            .enumerate()
            .map(|(i, id)| (id, i))
            .collect();
        for m in other.entries {
            merge_entry(&mut self.entries, &mut index, m, true);
        }
    }

    /// All entries.
    pub fn entries(&self) -> &[CatalogMod] {
        &self.entries
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Mod entries only.
    pub fn mods(&self) -> impl Iterator<Item = &CatalogMod> {
        self.entries.iter().filter(|m| m.kind == ProjectKind::Mod)
    }

    /// Modpack entries only.
    pub fn modpacks(&self) -> impl Iterator<Item = &CatalogMod> {
        self.entries
            .iter()
            .filter(|m| m.kind == ProjectKind::Modpack)
    }

    /// Look up a single entry by exact id.
    pub fn by_id(&self, id: &str) -> Option<&CatalogMod> {
        self.entries.iter().find(|m| m.id.eq_ignore_ascii_case(id))
    }

    /// Look up a single entry by id, slug or any alias/tag. This is what makes
    /// the FCL dictionary aliases (e.g. `irisshaders`) resolvable even though the
    /// canonical catalog id is `iris`.
    pub fn find_by_id_or_alias(&self, id: &str) -> Option<&CatalogMod> {
        let lower = id.to_ascii_lowercase();
        // Prefer an exact id / slug match: that is the canonical (richer) entry.
        // An alias/tag match only backs up a missing canonical entry so legacy
        // FCL ids (e.g. `irisshaders`) still resolve.
        if let Some(m) = self
            .entries
            .iter()
            .find(|m| m.id.eq_ignore_ascii_case(id) || m.slug.eq_ignore_ascii_case(id))
        {
            return Some(m);
        }
        self.entries
            .iter()
            .find(|m| m.tags.iter().any(|t| t.eq_ignore_ascii_case(&lower)))
    }

    /// Free-text search (returns references ordered by popularity).
    pub fn search(&self, query: &str) -> Vec<&CatalogMod> {
        let mut out: Vec<&CatalogMod> = self
            .entries
            .iter()
            .filter(|m| m.matches_query(query))
            .collect();
        out.sort_by(|a, b| b.downloads.cmp(&a.downloads));
        out
    }

    /// Apply every populated field of `filter` (AND-combined). Entries with
    /// unknown version/loader info are excluded when that dimension is filtered,
    /// keeping version/loader filtering precise.
    pub fn filter(&self, f: &CatalogFilter) -> Vec<&CatalogMod> {
        let mut out: Vec<&CatalogMod> = self
            .entries
            .iter()
            .filter(|m| {
                if let Some(kind) = f.kind {
                    if m.kind != kind {
                        return false;
                    }
                }
                if let Some(gv) = &f.game_version {
                    if m.game_versions.is_empty() || !m.matches_game_version(gv) {
                        return false;
                    }
                }
                if let Some(loader) = f.loader {
                    if m.loaders.is_empty() || !m.supports_loader(loader) {
                        return false;
                    }
                }
                if let Some(cat) = &f.category {
                    if !m.categories.contains(cat) {
                        return false;
                    }
                }
                if let Some(env) = f.environment {
                    if !m.supports_environment(env) {
                        return false;
                    }
                }
                if f.featured_only && !m.featured {
                    return false;
                }
                if let Some(q) = &f.query {
                    if !m.matches_query(q) {
                        return false;
                    }
                }
                true
            })
            .collect();
        out.sort_by(|a, b| b.downloads.cmp(&a.downloads));
        out
    }

    /// Resolve the selected mod ids against `game_version`:
    ///
    /// * flags entries incompatible with the target Minecraft version, and
    /// * collects missing dependencies, separating those that exist in the
    ///   catalog (auto-resolvable) from those that do not.
    pub fn resolve(&self, selected: &[String], game_version: &str) -> DependencyResolution {
        let selected_set: HashSet<&str> = selected.iter().map(|s| s.as_str()).collect();

        let mut version_conflicts: Vec<(String, String)> = Vec::new();
        for id in selected {
            if let Some(m) = self.find_by_id_or_alias(id) {
                if !m.matches_game_version(game_version) {
                    version_conflicts.push((
                        id.clone(),
                        format!("not compatible with Minecraft {game_version}"),
                    ));
                }
            }
        }

        let mut missing: Vec<String> = Vec::new();
        for id in selected {
            if let Some(m) = self.find_by_id_or_alias(id) {
                for dep in &m.dependencies {
                    if dep == "minecraft" {
                        continue;
                    }
                    if selected_set.contains(dep.as_str()) {
                        continue;
                    }
                    if !missing.iter().any(|x| x == dep) {
                        missing.push(dep.clone());
                    }
                }
            }
        }

        let mut resolvable: Vec<CatalogMod> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for dep in &missing {
            if let Some(m) = self.find_by_id_or_alias(dep) {
                if seen.insert(m.id.clone()) {
                    resolvable.push(m.clone());
                }
            }
        }

        DependencyResolution {
            game_version: game_version.to_string(),
            selected: selected.to_vec(),
            version_conflicts,
            missing_dependencies: missing,
            resolvable_dependencies: resolvable,
        }
    }

    /// Pull live results from Modrinth (through the China-mainland-aware
    /// [`NetworkClient`]) and merge them in. Returns the number of *new* entries
    /// added. Network / parse errors are surfaced to the caller so the UI can
    /// show a fallback banner while still keeping the offline data usable.
    pub async fn augment_with_online(
        &mut self,
        client: &NetworkClient,
        filter: &CatalogFilter,
    ) -> RcResult<usize> {
        let mc = ModrinthCatalogClient::new(client);
        let online = mc.search(filter).await?;
        let before = self.entries.len();
        for m in online {
            if !self
                .entries
                .iter()
                .any(|e| e.id.eq_ignore_ascii_case(&m.id) && e.source == m.source)
            {
                self.entries.push(m);
            }
        }
        Ok(self.entries.len() - before)
    }
}

/// Outcome of [`Catalog::resolve`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyResolution {
    /// The Minecraft version the selection was validated against.
    pub game_version: String,
    /// The originally selected mod ids.
    pub selected: Vec<String>,
    /// `(mod_id, reason)` for selected mods incompatible with `game_version`.
    pub version_conflicts: Vec<(String, String)>,
    /// Dependency ids referenced by the selection but not present in it.
    pub missing_dependencies: Vec<String>,
    /// Of the missing dependencies, those available in the catalog (can be
    /// auto-installed).
    pub resolvable_dependencies: Vec<CatalogMod>,
}

// ---------------------------------------------------------------------------
// Offline data files (compiled in via include_str! and shipped as APK assets)
// ---------------------------------------------------------------------------

/// FCL `mod_data.txt` (mcmod.cn id -> name dictionary), byte-identical to FCL.
const FCL_MOD_DATA: &str = include_str!("fcl_data/mod_data.txt");
/// FCL `modpack_data.txt`, byte-identical to FCL.
const FCL_MODPACK_DATA: &str = include_str!("fcl_data/modpack_data.txt");
/// Curated rich overlay: popular mods with categories / loaders / versions / deps.
const FEATURED_MODS: &str = include_str!("catalog_data/featured_mods.tsv");
/// Curated rich overlay: popular modpacks.
const FEATURED_MODPACKS: &str = include_str!("catalog_data/featured_modpacks.tsv");

/// Insert `cm` into `entries`, de-duplicating by id via `index`.
///
/// * If the id is new, append it.
/// * If the id already exists and `rich` is true, replace it with `cm` while
///   preserving the previously collected alias tags (FCL dictionary aliases).
/// * If the id already exists and `rich` is false (FCL dictionary pass), keep
///   the existing entry (never downgrade to the name-only record).
fn merge_entry(
    entries: &mut Vec<CatalogMod>,
    index: &mut HashMap<String, usize>,
    cm: CatalogMod,
    rich: bool,
) {
    if let Some(&i) = index.get(&cm.id) {
        if rich {
            let existing_tags = std::mem::take(&mut entries[i].tags);
            let mut merged = cm;
            for t in existing_tags {
                if !merged.tags.contains(&t) {
                    merged.tags.push(t);
                }
            }
            entries[i] = merged;
        }
    } else {
        index.insert(cm.id.clone(), entries.len());
        entries.push(cm);
    }
}

/// One row of FCL's `mod_data.txt` / `modpack_data.txt`
/// (`slug;id;aliases;cn_name;en_name;abbr`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FclModEntry {
    pub slug: String,
    pub id: String,
    pub aliases: Vec<String>,
    pub cn_name: String,
    pub en_name: String,
    pub abbr: String,
}

/// Parse FCL's semicolon-delimited dictionary. `#` comments and blank lines are
/// skipped; lines without any identifier (slug / id / name) are skipped.
pub fn parse_fcl_mod_data(text: &str) -> Vec<FclModEntry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split(';').collect();
        // A real dictionary record has slug;id;aliases;cn;en;abbr. Anything with
        // fewer than 4 fields cannot represent even (slug,id,cn,en) and is
        // malformed, so skip it rather than emitting a junk entry.
        if cols.len() < 4 {
            continue;
        }
        let slug = cols.get(0).unwrap_or(&"").to_string();
        let id = cols.get(1).unwrap_or(&"").to_string();
        let aliases = cols.get(2).map(|s| split_csv(*s)).unwrap_or_default();
        let cn_name = cols.get(3).unwrap_or(&"").to_string();
        let en_name = cols.get(4).unwrap_or(&"").to_string();
        let abbr = cols.get(5).unwrap_or(&"").to_string();
        if slug.is_empty() && id.is_empty() && cn_name.is_empty() && en_name.is_empty() {
            continue;
        }
        out.push(FclModEntry {
            slug,
            id,
            aliases,
            cn_name,
            en_name,
            abbr,
        });
    }
    out
}

/// Offline id -> display-name dictionary built from FCL's `mod_data.txt` /
/// `modpack_data.txt`. This is the offline fallback that lets the launcher show
/// a friendly name for any installed mod id even with no network.
#[derive(Debug, Clone)]
pub struct ModDictionary {
    entries: Vec<FclModEntry>,
    index: HashMap<String, usize>,
}

impl ModDictionary {
    /// Build the dictionary from the compiled-in FCL data.
    pub fn from_builtin() -> Self {
        let mut entries = parse_fcl_mod_data(FCL_MOD_DATA);
        entries.extend(parse_fcl_mod_data(FCL_MODPACK_DATA));
        Self::from_entries(entries)
    }

    /// Build the dictionary from explicit text (e.g. APK assets on disk).
    pub fn from_text(mod_text: &str, modpack_text: &str) -> Self {
        let mut entries = parse_fcl_mod_data(mod_text);
        entries.extend(parse_fcl_mod_data(modpack_text));
        Self::from_entries(entries)
    }

    fn from_entries(entries: Vec<FclModEntry>) -> Self {
        let mut index: HashMap<String, usize> = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            let mut keys: Vec<String> = Vec::new();
            if !e.slug.is_empty() {
                keys.push(e.slug.clone());
            }
            keys.extend(e.aliases.clone());
            if !e.id.is_empty() {
                keys.push(e.id.clone());
            }
            for k in keys {
                let k = k.to_ascii_lowercase();
                index.entry(k).or_insert(i);
            }
        }
        Self { entries, index }
    }

    /// Resolve an entry by id, slug or alias.
    pub fn lookup(&self, key: &str) -> Option<&FclModEntry> {
        self.index
            .get(&key.to_ascii_lowercase())
            .map(|&i| &self.entries[i])
    }

    /// Best display name for `key`: English name, then Chinese name, then slug.
    pub fn display_name(&self, key: &str) -> Option<String> {
        self.lookup(key).map(|e| {
            if !e.en_name.is_empty() {
                e.en_name.clone()
            } else if !e.cn_name.is_empty() {
                e.cn_name.clone()
            } else {
                e.slug.clone()
            }
        })
    }

    /// All dictionary entries.
    pub fn entries(&self) -> &[FclModEntry] {
        &self.entries
    }
}

fn fcl_to_catalog(e: FclModEntry, kind: ProjectKind) -> CatalogMod {
    let id = if !e.slug.is_empty() {
        e.slug.clone()
    } else if !e.aliases.is_empty() {
        e.aliases[0].clone()
    } else {
        e.id.clone()
    };
    let name = if !e.en_name.is_empty() {
        e.en_name.clone()
    } else if !e.cn_name.is_empty() {
        e.cn_name.clone()
    } else {
        id.clone()
    };
    let mut tags = e.aliases.clone();
    if !e.abbr.is_empty() && !tags.contains(&e.abbr) {
        tags.push(e.abbr.clone());
    }
    CatalogMod {
        id,
        slug: e.slug.clone(),
        name,
        summary: String::new(),
        description: String::new(),
        kind,
        source: CatalogSource::Builtin,
        categories: Vec::new(),
        tags,
        environment: ModEnvironment::Both,
        loaders: Vec::new(),
        game_versions: Vec::new(),
        dependencies: Vec::new(),
        downloads: 0,
        featured: false,
        icon_url: String::new(),
        project_url: String::new(),
    }
}

/// Parse TAB-separated catalog data (the curated overlay). Each line:
/// `id \t slug \t name \t summary \t categories(csv) \t loaders(csv) \t
/// game_versions(csv) \t environment \t source \t dependencies(csv) \t tags(csv)
/// \t downloads \t featured \t project_url`. `#` comments and blank lines are
/// skipped; malformed lines are skipped rather than aborting the whole parse.
pub fn parse_catalog_text(text: &str, default_kind: ProjectKind) -> Vec<CatalogMod> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 4 || cols[0].is_empty() {
            continue;
        }
        let id = cols[0].to_string();
        let slug = if cols.len() > 1 && !cols[1].is_empty() {
            cols[1].to_string()
        } else {
            id.clone()
        };
        let name = if cols.len() > 2 && !cols[2].is_empty() {
            cols[2].to_string()
        } else {
            id.clone()
        };
        let summary = cols.get(3).map(|s| s.to_string()).unwrap_or_default();
        let categories = cols
            .get(4)
            .map(|s| split_csv(*s))
            .unwrap_or_default()
            .into_iter()
            .map(|s| ModCategory::from_slug(&s))
            .collect();
        let loaders = cols
            .get(5)
            .map(|s| split_csv(*s))
            .unwrap_or_default()
            .into_iter()
            .map(|s| ModLoader::from_str(s.as_str()).unwrap_or(ModLoader::Vanilla))
            .collect();
        let game_versions = cols.get(6).map(|s| split_csv(*s)).unwrap_or_default();
        let environment = cols
            .get(7)
            .map(|s| parse_env(*s))
            .unwrap_or(ModEnvironment::Both);
        let source = cols
            .get(8)
            .map(|s| parse_source(*s))
            .unwrap_or(CatalogSource::Builtin);
        let dependencies = cols.get(9).map(|s| split_csv(*s)).unwrap_or_default();
        let tags = cols.get(10).map(|s| split_csv(*s)).unwrap_or_default();
        let downloads = cols
            .get(11)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let featured = cols.get(12).map(|s| *s == "1").unwrap_or(false);
        let project_url = cols.get(13).map(|s| s.to_string()).unwrap_or_default();

        out.push(CatalogMod {
            id,
            slug,
            name,
            summary,
            description: String::new(),
            kind: default_kind,
            source,
            categories,
            tags,
            environment,
            loaders,
            game_versions,
            dependencies,
            downloads,
            featured,
            icon_url: String::new(),
            project_url,
        });
    }
    out
}

fn split_csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

fn parse_env(s: &str) -> ModEnvironment {
    match s.trim().to_ascii_lowercase().as_str() {
        "server" => ModEnvironment::Server,
        "both" | "client_server" => ModEnvironment::Both,
        _ => ModEnvironment::Client,
    }
}

fn parse_source(s: &str) -> CatalogSource {
    match s.trim().to_ascii_lowercase().as_str() {
        "modrinth" => CatalogSource::Modrinth,
        "curseforge" => CatalogSource::CurseForge,
        _ => CatalogSource::Builtin,
    }
}

// ---------------------------------------------------------------------------
// Live Modrinth source (China-mainland aware via `NetworkClient`)
// ---------------------------------------------------------------------------

/// Live Modrinth catalog client. All requests go through the shared
/// [`NetworkClient`] so they inherit mirror fallback, DoH and proxy support.
pub struct ModrinthCatalogClient<'a> {
    client: &'a NetworkClient,
    base: String,
}

impl<'a> ModrinthCatalogClient<'a> {
    /// Default base URL (`https://api.modrinth.com/v2`).
    pub fn new(client: &'a NetworkClient) -> Self {
        Self {
            client,
            base: "https://api.modrinth.com/v2".to_string(),
        }
    }

    /// Override the base URL (e.g. a mainland Modrinth mirror).
    pub fn with_base(client: &'a NetworkClient, base: &str) -> Self {
        Self {
            client,
            base: base.trim_end_matches('/').to_string(),
        }
    }

    /// Search Modrinth, applying `filter`. Mirrors / DoH / proxy are handled by
    /// the underlying client.
    pub async fn search(&self, filter: &CatalogFilter) -> RcResult<Vec<CatalogMod>> {
        use url::Url;

        let mut url = Url::parse(&format!("{}/search", self.base))
            .map_err(|e| crate::error::RcError::Other(format!("bad modrinth base url: {e}")))?;
        {
            let mut qp = url.query_pairs_mut();
            if let Some(q) = &filter.query {
                if !q.is_empty() {
                    qp.append_pair("query", q);
                }
            }
            qp.append_pair("limit", "50");

            let mut inner: Vec<String> = Vec::new();
            if let Some(kind) = filter.kind {
                let k = match kind {
                    ProjectKind::Mod => "mod",
                    ProjectKind::Modpack => "modpack",
                };
                inner.push(format!("[\"project_type:{k}\"]"));
            }
            if let Some(gv) = &filter.game_version {
                inner.push(format!("[\"versions:{}\"]", gv));
            }
            if let Some(cat) = &filter.category {
                inner.push(format!("[\"categories:{}\"]", cat.slug()));
            }
            if let Some(loader) = filter.loader {
                inner.push(format!("[\"categories:{}\"]", loader.as_str()));
            }
            if !inner.is_empty() {
                qp.append_pair("facets", &format!("[{}]", inner.join(",")));
            }
        }

        let resp = self
            .client
            .fetch_json::<ModrinthSearchResponse>(url.as_str())
            .await?;
        Ok(resp.hits.into_iter().map(CatalogMod::from).collect())
    }
}

#[derive(Deserialize)]
struct ModrinthSearchResponse {
    hits: Vec<ModrinthHit>,
}

#[derive(Deserialize)]
struct ModrinthHit {
    project_id: String,
    slug: Option<String>,
    title: String,
    description: String,
    categories: Vec<String>,
    client_side: Option<String>,
    server_side: Option<String>,
    versions: Vec<String>,
    downloads: Option<u64>,
    icon_url: Option<String>,
    project_type: Option<String>,
}

impl From<ModrinthHit> for CatalogMod {
    fn from(h: ModrinthHit) -> Self {
        let kind = match h.project_type.as_deref() {
            Some("modpack") => ProjectKind::Modpack,
            _ => ProjectKind::Mod,
        };
        let mut loaders = Vec::new();
        let mut tags = Vec::new();
        let mut categories = Vec::new();
        for c in &h.categories {
            match loader_slug(c) {
                Some(l) => {
                    if !loaders.contains(&l) {
                        loaders.push(l);
                    }
                }
                None => {
                    let cat = ModCategory::from_slug(c);
                    if let ModCategory::Other(_) = cat {
                        tags.push(c.clone());
                    } else if !categories.contains(&cat) {
                        categories.push(cat);
                    }
                }
            }
        }
        let cs = h.client_side.as_deref().unwrap_or("required");
        let ss = h.server_side.as_deref().unwrap_or("required");
        let environment = match (cs, ss) {
            ("unsupported", _) => ModEnvironment::Server,
            (_, "unsupported") => ModEnvironment::Client,
            _ => ModEnvironment::Both,
        };
        let slug = h.slug.clone().unwrap_or_else(|| h.project_id.clone());
        let project_url = format!(
            "https://modrinth.com/{}/{}",
            match kind {
                ProjectKind::Mod => "mod",
                ProjectKind::Modpack => "modpack",
            },
            slug
        );
        CatalogMod {
            id: h.project_id,
            slug,
            name: h.title,
            summary: h.description,
            description: String::new(),
            kind,
            source: CatalogSource::Modrinth,
            categories,
            tags,
            environment,
            loaders,
            game_versions: h.versions,
            dependencies: Vec::new(),
            downloads: h.downloads.unwrap_or(0),
            featured: false,
            icon_url: h.icon_url.unwrap_or_default(),
            project_url,
        }
    }
}

fn loader_slug(s: &str) -> Option<ModLoader> {
    match s {
        "fabric" => Some(ModLoader::Fabric),
        "forge" => Some(ModLoader::Forge),
        "quilt" => Some(ModLoader::Quilt),
        "neoforge" => Some(ModLoader::Forge),
        "liteloader" => Some(ModLoader::LiteLoader),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_is_nonempty() {
        let c = Catalog::builtin();
        assert!(!c.is_empty());
        // The FCL dictionary contributes thousands of mods and modpacks.
        assert!(c.mods().count() > 1000);
        assert!(c.modpacks().count() > 100);
    }

    #[test]
    fn known_mods_present_and_enriched() {
        let c = Catalog::builtin();
        // FCL dictionary provides `sodium` by slug; the rich overlay adds metadata.
        let sodium = c.by_id("sodium").expect("sodium present");
        assert_eq!(sodium.kind, ProjectKind::Mod);
        assert!(sodium.loaders.contains(&ModLoader::Fabric));
        assert!(sodium.game_versions.iter().any(|v| v == "1.20.1"));
        // iris aliases to `irisshaders`; the rich overlay adds the dependency edge.
        let iris = c.find_by_id_or_alias("iris").expect("iris present");
        assert!(iris.dependencies.iter().any(|d| d == "sodium"));
        // all-the-mods-9 comes from the FCL modpack dictionary and is enriched.
        assert!(c.by_id("all-the-mods-9").is_some());
    }

    #[test]
    fn fcl_dictionary_resolves_names() {
        let dict = ModDictionary::from_builtin();
        assert_eq!(dict.display_name("sodium").as_deref(), Some("Sodium"));
        assert_eq!(
            dict.display_name("irisshaders").as_deref(),
            Some("Iris Shaders")
        );
        assert_eq!(
            dict.display_name("jei").as_deref(),
            Some("Just Enough Items")
        );
        // alias lookup (IC2 is an alias of industrial-craft).
        assert!(dict.display_name("ic2").is_some());
    }

    #[test]
    fn search_finds_by_name_and_tag() {
        let c = Catalog::builtin();
        let r = c.search("shader");
        assert!(r.iter().any(|m| m.id == "iris"));
        let r = c.search("sodium");
        assert!(r.iter().any(|m| m.id == "sodium"));
    }

    #[test]
    fn game_version_filtering_excludes_unknown_and_incompatible() {
        let c = Catalog::builtin();
        let f = CatalogFilter {
            game_version: Some("1.20.1".to_string()),
            ..Default::default()
        };
        let r = c.filter(&f);
        assert!(r.iter().any(|m| m.id == "sodium"));
        // rlcraft targets 1.12.2 and must NOT appear under 1.20.1.
        assert!(!r.iter().any(|m| m.id == "rlcraft"));
        // FCL legacy entries without version info are excluded from a version filter.
        assert!(!r.iter().any(|m| m.id == "3"));
    }

    #[test]
    fn loader_filtering_excludes_unknown() {
        let c = Catalog::builtin();
        let f = CatalogFilter {
            loader: Some(ModLoader::Fabric),
            ..Default::default()
        };
        let r = c.filter(&f);
        assert!(r.iter().any(|m| m.id == "sodium"));
        assert!(!r.iter().any(|m| m.id == "applied-energistics-2"));
    }

    #[test]
    fn category_and_environment_filters_combine() {
        let c = Catalog::builtin();
        let f = CatalogFilter {
            category: Some(ModCategory::Optimization),
            environment: Some(ModEnvironment::Client),
            ..Default::default()
        };
        let r = c.filter(&f);
        assert!(r.iter().any(|m| m.id == "sodium"));
        // lithium is Both, so it runs on the client too and must be present.
        assert!(r.iter().any(|m| m.id == "lithium"));
    }

    #[test]
    fn environment_server_filter_excludes_client_only() {
        let c = Catalog::builtin();
        let f = CatalogFilter {
            environment: Some(ModEnvironment::Server),
            ..Default::default()
        };
        let r = c.filter(&f);
        // sodium is client-only -> excluded under a server filter.
        assert!(!r.iter().any(|m| m.id == "sodium"));
        // lithium is Both -> present.
        assert!(r.iter().any(|m| m.id == "lithium"));
    }

    #[test]
    fn dependency_resolution_reports_missing_and_resolvable() {
        let c = Catalog::builtin();
        // Select iris (depends on sodium) and jei (depends on fabric-api).
        let selected = vec!["iris".to_string(), "jei".to_string()];
        let res = c.resolve(&selected, "1.20.1");
        assert!(res.missing_dependencies.contains(&"sodium".to_string()));
        assert!(res.missing_dependencies.contains(&"fabric-api".to_string()));
        assert!(res.resolvable_dependencies.iter().any(|m| m.id == "sodium"));
        assert!(res
            .resolvable_dependencies
            .iter()
            .any(|m| m.id == "fabric-api"));
        assert!(res.version_conflicts.is_empty());
    }

    #[test]
    fn dependency_resolution_flags_version_conflict() {
        let c = Catalog::builtin();
        let selected = vec!["jei".to_string()];
        let res = c.resolve(&selected, "1.7.10");
        assert!(res.version_conflicts.iter().any(|(id, _)| id == "jei"));
    }

    #[test]
    fn modrinth_mapping_handles_environment_and_loaders() {
        let hit = ModrinthHit {
            project_id: "AABBCCDD".into(),
            slug: Some("demo-mod".into()),
            title: "Demo Mod".into(),
            description: "A demo".into(),
            categories: vec!["fabric".into(), "technology".into(), "fabric".into()],
            client_side: Some("required".into()),
            server_side: Some("unsupported".into()),
            versions: vec!["1.20.1".into()],
            downloads: Some(1234),
            icon_url: None,
            project_type: Some("mod".into()),
        };
        let m = CatalogMod::from(hit);
        assert_eq!(m.id, "AABBCCDD");
        assert!(m.loaders.contains(&ModLoader::Fabric));
        assert_eq!(m.environment, ModEnvironment::Client);
        assert!(m.categories.contains(&ModCategory::Technology));
        assert_eq!(m.game_versions, vec!["1.20.1".to_string()]);
    }

    #[test]
    fn parse_roundtrip_offline_files() {
        let mods = parse_catalog_text(FEATURED_MODS, ProjectKind::Mod);
        let packs = parse_catalog_text(FEATURED_MODPACKS, ProjectKind::Modpack);
        assert!(!mods.is_empty());
        assert!(!packs.is_empty());
        for m in mods.iter().chain(packs.iter()) {
            assert!(!m.id.is_empty());
            assert!(!m.name.is_empty());
        }
        // FCL files parse to a large dictionary.
        let fcl = parse_fcl_mod_data(FCL_MOD_DATA);
        assert!(fcl.len() > 1000);
    }

    #[test]
    fn category_serializes_as_slug() {
        let json = serde_json::to_string(&ModCategory::Optimization).unwrap();
        assert_eq!(json, "\"optimization\"");
        let back: ModCategory = serde_json::from_str("\"magic\"").unwrap();
        assert_eq!(back, ModCategory::Magic);
        let other: ModCategory = serde_json::from_str("\"somethingnew\"").unwrap();
        assert_eq!(other, ModCategory::Other("somethingnew".into()));
    }

    #[test]
    fn resolve_handles_empty_and_unknown_selection() {
        let c = Catalog::builtin();
        // Empty selection -> no conflicts, no missing.
        let empty = c.resolve(&[], "1.20.1");
        assert!(empty.version_conflicts.is_empty());
        assert!(empty.missing_dependencies.is_empty());
        assert!(empty.resolvable_dependencies.is_empty());
        // Unknown selected id -> no panic, nothing resolved.
        let unknown = c.resolve(&["definitely_not_a_real_mod_xyz".to_string()], "1.20.1");
        assert!(unknown.version_conflicts.is_empty());
        assert!(unknown.missing_dependencies.is_empty());
    }

    #[test]
    fn mod_dictionary_resolves_aliases() {
        let dict = ModDictionary::from_builtin();
        // industrial-craft is known by slug; ic2 is one of its aliases.
        let slug = dict.display_name("industrial-craft");
        let alias = dict.display_name("ic2");
        assert!(slug.is_some());
        assert!(alias.is_some());
        // alias resolves to the same project name as the slug.
        assert_eq!(slug, alias);
    }

    #[test]
    fn parse_fcl_skips_malformed_lines() {
        let text = "# comment\n\nindustrial-craft;2;IC2,ic2;SomeCnName;Industrial Craft 2;IC2\njust-one-field\n;3;;SomeOtherCn;Flying Road 3;\n";
        let entries = parse_fcl_mod_data(text);
        // Only the two well-formed lines should survive (the one-field line and a
        // fully-empty line are skipped).
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].slug, "industrial-craft");
        assert_eq!(entries[0].en_name, "Industrial Craft 2");
        assert_eq!(entries[1].id, "3");
        assert_eq!(entries[1].en_name, "Flying Road 3");
    }
}
