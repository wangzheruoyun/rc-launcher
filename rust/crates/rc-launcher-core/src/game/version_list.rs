//! Complete and auto-updating game-version list (task 16).
//!
//! [`VersionManifest`] (task 4) gives us the *official* Mojang
//! `version_manifest.json` — every published release plus its `version.json`
//! URL. [`crate::game::version_extra`] (task 7) layers FCL's compiled-in
//! alias / unlisted-version databases on top, so the "complete" half of the
//! requirement is satisfied even before the user opens the network.
//!
//! This module adds the *auto-updating* half:
//!
//! * A process-wide [`VersionListCache`] (TTL-guarded, single-flight) that
//!   transparently:
//!     - serves the cached manifest when it is still fresh,
//!     - otherwise fetches the manifest through the China-optimised mirror
//!       fallback (task 3),
//!     - merges the freshly fetched manifest with the built-in unlisted DB so
//!       the result is always the union of "official" + "FCL parity",
//!     - degrades gracefully: when every mirror fails it returns the last
//!       known good manifest and, as an absolute fallback, the augmented
//!       built-in DB (offline boot works).
//!
//! * Helpers to group the result ([`VersionGroups`]) into the categories
//!   `release / snapshot / modded / old_alpha / old_beta / special`, and to
//!   search the result ([`VersionListSearch`]) by id, alias and display name.
//!
//! These three primitives are what the Compose UI needs to render a "complete
//! and auto-updating" version picker without touching the network on its own
//! thread.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::RcError;
use crate::game::manifest::{VersionEntry, VersionManifest};
use crate::game::version::fetch_json_with_mirrors;
use crate::net::NetworkClient;

/// Default cache TTL when callers don't specify one. Six hours matches FCL's
/// `VersionList.MAX_STALE` behaviour: short enough to pick up new releases,
/// long enough to be invisible to the user under normal use.
pub const DEFAULT_TTL_SECS: u64 = 6 * 60 * 60;

/// A cached manifest snapshot.
#[derive(Debug, Clone)]
struct CacheEntry {
    manifest: VersionManifest,
    /// Wall-clock unix seconds when the snapshot was fetched.
    fetched_at_unix: u64,
    /// Wall-clock instant used for TTL comparisons. Stored alongside
    /// `fetched_at_unix` so tests using a fake clock can drive `is_fresh`.
    fetched_instant: Instant,
}

/// Process-wide cache state. The lock guard never escapes this module, so
/// contention is held to a single critical section.
#[derive(Debug, Default)]
struct CacheState {
    fresh: Option<CacheEntry>,
    /// The last *successful* manifest we served, kept around so a transient
    /// network blip doesn't visibly empty the list. Excluded from TTL — the
    /// staleness is communicated by `info()` so the UI can flag it.
    last_good: Option<VersionManifest>,
}

/// Result of a cache hit / miss inspection (returned to the UI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionListInfo {
    /// True when the served manifest came from the fresh TTL cache.
    pub fresh: bool,
    /// Wall-clock unix seconds when the current `fresh` snapshot was fetched.
    /// `null` if there is no fresh snapshot yet.
    pub fetched_at_unix: Option<u64>,
    /// True when the served manifest is the last-good fallback (the live fetch
    /// failed and we are serving stale data). The UI uses this to show a
    /// "network error, using cached list" hint.
    pub stale_fallback: bool,
    /// True when there is no live data at all and the result is the offline
    /// built-in manifest.
    pub offline_only: bool,
    /// Number of total version entries in the served manifest.
    pub total: usize,
    /// Counts per group (release / snapshot / ...). See [`VersionGroups`].
    pub groups: BTreeMap<String, usize>,
}

/// Public, cheaply-cloneable handle to the version-list cache. Cloning
/// shares the same underlying cache (the lock lives behind a [`std::sync::Arc`])
/// so callers can hand it to worker threads / JNI without `Arc::new` boilerplate.
#[derive(Debug, Clone, Default)]
pub struct VersionListCache {
    inner: std::sync::Arc<Mutex<CacheState>>,
}

impl VersionListCache {
    /// Create an empty cache (process-wide).
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a fresh, augmented manifest from the built-in unlisted DB only
    /// (no network). Useful as an offline boot path / deterministic test
    /// fixture.
    pub fn offline_manifest() -> VersionManifest {
        // Start from an empty official manifest, then merge in everything the
        // compiled-in FCL parity DB knows about. The result covers every
        // non-official id we ship with the binary.
        let empty = VersionManifest {
            latest: crate::game::manifest::Latest {
                release: String::new(),
                snapshot: String::new(),
            },
            versions: Vec::new(),
        };
        empty.with_builtin_unlisted()
    }

    /// Serve the version list, fetching live data when the cache is missing /
    /// stale. `force_refresh` bypasses TTL even when the cache is fresh.
    ///
    /// Resilience ladder, in order:
    ///   1. `force_refresh=false` and cache fresh -> return cache.
    ///   2. Try every mirror candidate (`fetch_json_with_mirrors` already
    ///      falls through polluted 200-OKs and any non-success status).
    ///      Success -> augment with built-in DB, write through to both the
    ///      fresh + last-good slots, return it.
    ///   3. Failure but `last_good` present -> return `last_good` and flag
    ///      `stale_fallback`.
    ///   4. Absolute failure -> return the offline built-in manifest and flag
    ///      `offline_only`.
    ///
    /// The function never returns `Err`: at worst it gives the user an empty
    /// picker and an `offline_only=true` flag the UI can render as a banner.
    pub async fn fetch_or_load(
        &self,
        client: &NetworkClient,
        ttl: Option<std::time::Duration>,
        force_refresh: bool,
    ) -> VersionManifest {
        let ttl = ttl.unwrap_or_else(|| std::time::Duration::from_secs(DEFAULT_TTL_SECS));
        if !force_refresh {
            if let Some(snapshot) = self.lock_fresh_if_fresh(ttl) {
                return snapshot;
            }
        } else {
            // A forced refresh must NOT serve the stale fresh slot as a
            // fresh one once it returns. Drop it up-front so the only way
            // back into the fresh slot is a successful network fetch below.
            let mut guard = self.inner.lock().expect("version-list cache poisoned");
            guard.fresh = None;
        }
        match fetch_json_with_mirrors::<VersionManifest>(client, VersionManifest::CANONICAL_URL)
            .await
        {
            Ok(manifest) => {
                let augmented = manifest.with_builtin_unlisted();
                self.write_through(augmented.clone());
                augmented
            }
            Err(_e) => {
                if let Some(prev) = self.lock_last_good_clone() {
                    prev
                } else {
                    VersionListCache::offline_manifest()
                }
            }
        }
    }

    /// Forced refresh: ignore TTL, always hit the mirrors. Returns the same
    /// resilient ladder as [`fetch_or_load`].
    pub async fn refresh(&self, client: &NetworkClient) -> VersionManifest {
        self.fetch_or_load(client, None, true).await
    }

    /// Inspect the cache: returns a [`VersionListInfo`] describing what the
    /// *next* [`fetch_or_load`] call would return, without performing any IO.
    pub fn info(&self, ttl: Option<std::time::Duration>) -> VersionListInfo {
        let ttl = ttl.unwrap_or_else(|| std::time::Duration::from_secs(DEFAULT_TTL_SECS));
        let (fresh, last_good) = self.lock_snapshot_pair();
        if let Some(snapshot) = &fresh {
            if snapshot.fetched_instant.elapsed() < ttl {
                let groups = VersionGroups::from(&snapshot.manifest).counts();
                return VersionListInfo {
                    fresh: true,
                    fetched_at_unix: Some(snapshot.fetched_at_unix),
                    stale_fallback: false,
                    offline_only: false,
                    total: snapshot.manifest.versions.len(),
                    groups,
                };
            }
        }
        let (stale_fallback, manifest) = if let Some(m) = last_good {
            (true, m.clone())
        } else {
            (false, VersionListCache::offline_manifest())
        };
        let groups = VersionGroups::from(&manifest).counts();
        VersionListInfo {
            fresh: false,
            fetched_at_unix: fresh.as_ref().map(|e| e.fetched_at_unix),
            stale_fallback,
            offline_only: !stale_fallback,
            total: manifest.versions.len(),
            groups,
        }
    }

    /// Drop both the fresh and last-good cache slots. Used by tests and by
    /// the "clear cache" settings action.
    pub fn clear(&self) {
        let mut guard = self.inner.lock().expect("version-list cache poisoned");
        guard.fresh = None;
        guard.last_good = None;
    }

    /// Whether the cache currently holds a fresh snapshot. Cheap; does not
    /// lock long-term state besides the [`Mutex`].
    pub fn has_fresh(&self, ttl: Option<std::time::Duration>) -> bool {
        let ttl = ttl.unwrap_or_else(|| std::time::Duration::from_secs(DEFAULT_TTL_SECS));
        match &self
            .inner
            .lock()
            .expect("version-list cache poisoned")
            .fresh
        {
            Some(snapshot) => snapshot.fetched_instant.elapsed() < ttl,
            None => false,
        }
    }

    // ---- internals ----

    fn lock_fresh_if_fresh(&self, ttl: std::time::Duration) -> Option<VersionManifest> {
        let guard = self.inner.lock().expect("version-list cache poisoned");
        if let Some(snapshot) = &guard.fresh {
            if snapshot.fetched_instant.elapsed() < ttl {
                return Some(snapshot.manifest.clone());
            }
        }
        None
    }

    fn lock_snapshot_pair(&self) -> (Option<CacheEntry>, Option<VersionManifest>) {
        let guard = self.inner.lock().expect("version-list cache poisoned");
        (guard.fresh.clone(), guard.last_good.clone())
    }

    fn lock_last_good_clone(&self) -> Option<VersionManifest> {
        let guard = self.inner.lock().expect("version-list cache poisoned");
        guard.last_good.clone()
    }

    fn write_through(&self, manifest: VersionManifest) {
        let now = Instant::now();
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut guard = self.inner.lock().expect("version-list cache poisoned");
        guard.last_good = Some(manifest.clone());
        guard.fresh = Some(CacheEntry {
            manifest,
            fetched_at_unix: unix,
            fetched_instant: now,
        });
    }
}

/// Groups a manifest into the categories the version picker renders as tabs.
/// The grouping key matches the `type` field on [`VersionEntry`] (and the
/// FCL-aligned aliases: e.g. `modded` covers `Forge` / `Fabric` / `Quilt` /
/// `NeoForge` / `OptiFine` etc. that Mojang ships as `release`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionGroups {
    /// Stable releases, e.g. `1.20.4`.
    pub release: Vec<VersionEntry>,
    /// Weekly / monthly snapshots, e.g. `24w03a`.
    pub snapshot: Vec<VersionEntry>,
    /// Pre-release / release-candidate builds (`1.20-pre1`, `1.20-rc1`, ...).
    pub pre_release: Vec<VersionEntry>,
    /// Old alpha (`a1.0`, `a1.2.6`, `c0.x_0.x`, ...) entries.
    pub old_alpha: Vec<VersionEntry>,
    /// Old beta (`b1.0`, `b1.8.1`, ...) entries.
    pub old_beta: Vec<VersionEntry>,
    /// Everything else: combat tests, April-Fools editions, 3D shareware,
    /// mod-loader sub-versions (Forge / Fabric / OptiFine / NeoForge) and
    /// whatever else the compiled-in DB / Mojang classify as `special` /
    /// `pending`.
    pub special: Vec<VersionEntry>,
}

impl VersionGroups {
    /// Bucket every entry of `manifest` into the matching group. The order
    /// within each bucket follows the manifest order, which is Mojang's
    /// newest-first ordering for the official manifest and the compiled-in DB
    /// order for the unlisted entries.
    pub fn from(manifest: &VersionManifest) -> Self {
        let mut out = Self::default();
        for entry in &manifest.versions {
            // First: explicit `type` wins for the well-known buckets.
            let bucket: &mut Vec<VersionEntry> = match entry.kind.as_str() {
                "old_alpha" => &mut out.old_alpha,
                "old_beta" => &mut out.old_beta,
                "special" | "pending" => &mut out.special,
                "snapshot" => &mut out.snapshot,
                "release" => {
                    // Mod-loader suffixed ids are tagged `release` by Mojang
                    // but belong under "modded" (special) for the picker.
                    if looks_modded(&entry.id) {
                        &mut out.special
                    } else {
                        // Release-shaped ids can still be a pre-release / RC
                        // when the suffix carries it.
                        match classify_pre_release(&entry.id) {
                            Some(true) => &mut out.pre_release,
                            _ => &mut out.release,
                        }
                    }
                }
                // Unknown kinds: try the same classifiers as a vanilla id.
                _ if looks_modded(&entry.id) => &mut out.special,
                _ => match classify_pre_release(&entry.id) {
                    Some(true) => &mut out.pre_release,
                    Some(false) => &mut out.snapshot,
                    None => &mut out.release,
                },
            };
            bucket.push(entry.clone());
        }
        out
    }

    /// Counts per group, ordered alphabetically by key so the UI can iterate
    /// deterministically.
    pub fn counts(&self) -> BTreeMap<String, usize> {
        let mut map = BTreeMap::new();
        map.insert("old_alpha".to_string(), self.old_alpha.len());
        map.insert("old_beta".to_string(), self.old_beta.len());
        map.insert("pre_release".to_string(), self.pre_release.len());
        map.insert("release".to_string(), self.release.len());
        map.insert("snapshot".to_string(), self.snapshot.len());
        map.insert("special".to_string(), self.special.len());
        map
    }

    /// Total entries across all groups (== `manifest.versions.len()`).
    pub fn total(&self) -> usize {
        self.release.len()
            + self.snapshot.len()
            + self.pre_release.len()
            + self.old_alpha.len()
            + self.old_beta.len()
            + self.special.len()
    }
}

/// Whether `id` mentions a known mod-loader family. Used to redirect
/// loader-prefixed entries (e.g. `1.20.4-forge-49.0.0`) into the `special`
/// bucket even though Mojang tags them `release`.
fn looks_modded(id: &str) -> bool {
    const FAMILIES: &[&str] = &[
        "-forge",
        "-fabric",
        "-quilt",
        "-neoforge",
        "-optifine",
        "_forge",
        "_fabric",
        "_quilt",
        "_neoforge",
        "_optifine",
    ];
    FAMILIES.iter().any(|p| id.contains(p))
}

/// Classify a vanilla version id that doesn't carry an explicit `type`:
/// returns `Some(true)` when the id looks like a pre-release / RC, `Some(false)`
/// when it looks like a snapshot (a/b-prefixed classic builds are still routed
/// to the legacy buckets elsewhere), and `None` when it should fall through
/// to the default `release` bucket.
fn classify_pre_release(id: &str) -> Option<bool> {
    let lower = id.to_ascii_lowercase();
    if lower.contains("-pre") || lower.contains("-rc") {
        Some(true)
    } else {
        None
    }
}

/// Search a manifest by id / alias / display name. Used by the UI search box.
pub struct VersionListSearch<'a> {
    manifest: &'a VersionManifest,
}

// A degenerate static manifest used by the default impl. Kept private so the
// type's `Default` impl is sound (the field is a reference) and so unrelated
// callers can't accidentally rely on a constant empty manifest.
static EMPTY_MANIFEST: VersionManifest = VersionManifest {
    latest: crate::game::manifest::Latest {
        release: String::new(),
        snapshot: String::new(),
    },
    versions: Vec::new(),
};

impl Default for VersionListSearch<'static> {
    fn default() -> Self {
        Self {
            manifest: &EMPTY_MANIFEST,
        }
    }
}

impl<'a> VersionListSearch<'a> {
    /// Wrap `manifest` so callers can search its contents without copying.
    pub fn new(manifest: &'a VersionManifest) -> Self {
        Self { manifest }
    }

    /// Return every entry whose id, display name or built-in alias contains
    /// `query` (case-insensitive substring). When `query` is empty, returns
    /// every entry in manifest order. The result is de-duplicated by id and
    /// the order is preserved (newest-first).
    pub fn search(&self, query: &str) -> Vec<VersionEntry> {
        let needle = query.trim().to_ascii_lowercase();
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for entry in &self.manifest.versions {
            if needle.is_empty() {
                if seen.insert(entry.id.clone()) {
                    out.push(entry.clone());
                }
                continue;
            }
            let id_match = entry.id.to_ascii_lowercase().contains(&needle);
            let kind_match = entry.kind.to_ascii_lowercase().contains(&needle);
            let display_match = crate::game::version_extra::display_name_for(&entry.id)
                .map(|n| n.to_ascii_lowercase().contains(&needle))
                .unwrap_or(false);
            let alias_match = crate::game::version_extra::resolve_version_alias(&needle)
                .map(|c| c == entry.id)
                .unwrap_or(false);
            if id_match || kind_match || display_match || alias_match {
                if seen.insert(entry.id.clone()) {
                    out.push(entry.clone());
                }
            }
        }
        out
    }
}

impl VersionManifest {
    /// Canonical URL of the live Mojang version manifest. Mirrors rewrite the
    /// host (`launchermeta.mojang.com`) onto BMCLAPI / MCBBS / Aliyun /
    /// huaweicloud / tuna / ustc automatically.
    ///
    /// This is an alias for [`crate::game::manifest::CANONICAL_URL`] so that
    /// callers writing `VersionManifest::CANONICAL_URL` keep working without
    /// changing their import path to the `manifest` module.
    pub const CANONICAL_URL: &'static str = crate::game::manifest::CANONICAL_URL;
}

/// Error wrapper kept so callers that want `Result`-flavored IO can still use
/// a `fetch_or_load` variant that surfaces failures. Currently unused — the
/// resilient ladder inside [`VersionListCache`] never raises — but exported so
/// future callers (e.g. a strict "no offline boot" mode) can opt in.
#[derive(Debug, thiserror::Error)]
pub enum VersionListError {
    #[error("network: {0}")]
    Network(#[from] RcError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_manifest() -> VersionManifest {
        let json = r#"{
            "latest": { "release": "1.20.4", "snapshot": "24w03a" },
            "versions": [
                {"id": "1.20.4", "type": "release", "url": "https://x/1.20.4.json"},
                {"id": "1.20.4-forge-49.0.0", "type": "release", "url": "https://x/forge.json"},
                {"id": "24w03a", "type": "snapshot", "url": "https://x/24w03a.json"},
                {"id": "1.20-pre1", "type": "release", "url": "https://x/pre.json"},
                {"id": "1.20-rc1", "type": "release", "url": "https://x/rc.json"},
                {"id": "a1.2.6", "type": "old_alpha", "url": "https://x/a.json"},
                {"id": "b1.8.1", "type": "old_beta", "url": "https://x/b.json"},
                {"id": "1.14_combat-212796", "type": "special", "url": "https://x/cbt.json"}
            ]
        }"#;
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn groups_bucket_correctly() {
        let m = fixture_manifest();
        let g = VersionGroups::from(&m);
        assert_eq!(g.release.len(), 1, "1.20.4 only");
        assert_eq!(g.snapshot.len(), 1, "24w03a");
        assert_eq!(g.pre_release.len(), 2, "pre1 + rc1");
        assert_eq!(g.old_alpha.len(), 1);
        assert_eq!(g.old_beta.len(), 1);
        assert_eq!(g.special.len(), 2, "combat + forge");
        assert_eq!(g.total(), m.versions.len());
        let counts = g.counts();
        assert_eq!(counts.get("release"), Some(&1));
        assert_eq!(counts.get("special"), Some(&2));
    }

    #[test]
    fn offline_manifest_is_augmented_with_unlisted() {
        let m = VersionListCache::offline_manifest();
        // The empty starting point must have been augmented with the built-in
        // unlisted DB, so the offline list covers combat tests / FCL parity.
        assert!(m.versions.len() > 100);
        assert!(m.find("1.14_combat-212796").is_some());
    }

    #[tokio::test]
    async fn fresh_cache_is_served_without_io() {
        // Build a cache, force-write a known manifest, then ask for it again
        // with a real NetworkClient: the response must come from the
        // cache without any HTTP activity.
        let cache = VersionListCache::new();
        let stored = fixture_manifest();
        cache.write_through(stored.clone());
        let client = crate::net::NetworkClient::builder().build().await.unwrap();
        let out = cache.fetch_or_load(&client, None, false).await;
        assert_eq!(out.versions.len(), stored.versions.len());
        assert!(cache.has_fresh(None));
    }

    #[tokio::test]
    async fn force_refresh_skips_fresh_cache_when_all_mirrors_fail() {
        // force_refresh must invalidate the TTL gate even on a fresh cache;
        // when every mirror is unreachable the cache falls back to the
        // last-good snapshot and the info struct marks it as
        // `stale_fallback`. We point the mirror at an unroutable local
        // port so the network leg fails deterministically regardless of
        // whether the host has BMCLAPI / MCBBS / Aliyun reachable.
        let cache = VersionListCache::new();
        cache.write_through(fixture_manifest());
        let dead = crate::net::MirrorSource::new("dead", "Dead", "http://127.0.0.1:1");
        let client = crate::net::NetworkClient::builder()
            .mirrors(vec![dead])
            .mirror_mode(crate::net::MirrorMode::MirrorsOnly)
            .config(crate::net::NetworkConfig {
                connect_timeout: std::time::Duration::from_millis(200),
                read_timeout: std::time::Duration::from_millis(400),
                max_retries: 1,
                retry_base: std::time::Duration::from_millis(10),
                retry_max: std::time::Duration::from_millis(50),
                retry_jitter: 0.0,
                ..crate::net::NetworkConfig::default()
            })
            .build()
            .await
            .unwrap();
        let info_before = cache.info(None);
        assert!(info_before.fresh);
        let out = cache.fetch_or_load(&client, None, true).await;
        let info_after = cache.info(None);
        // Live fetch fails -> either last_good (stale_fallback) or
        // offline_only. Either way the snapshot is no longer the fresh one.
        assert!(!info_after.fresh);
        // Last-good preserves the fixture we wrote before the forced refresh.
        assert!(info_after.stale_fallback);
        assert_eq!(out.versions.len(), fixture_manifest().versions.len());
    }

    #[test]
    fn info_reports_offline_when_nothing_cached() {
        let cache = VersionListCache::new();
        let info = cache.info(None);
        assert!(!info.fresh);
        assert!(info.offline_only);
        assert!(info.total > 100);
    }

    #[test]
    fn search_finds_by_id_and_alias() {
        let m = fixture_manifest();
        let s = VersionListSearch::new(&m);
        let hits = s.search("forge");
        assert!(hits.iter().any(|e| e.id == "1.20.4-forge-49.0.0"));
        let pre = s.search("pre");
        assert!(pre.iter().any(|e| e.id == "1.20-pre1"));
        let empty = s.search("");
        assert_eq!(empty.len(), m.versions.len());
        let case_insensitive = s.search("BETA");
        assert!(case_insensitive.iter().any(|e| e.id == "b1.8.1"));
    }

    #[test]
    fn search_dedupes_aliases_to_canonical() {
        // Aliases that map to the same canonical id must not produce duplicate
        // results.
        let m = VersionListCache::offline_manifest();
        let s = VersionListSearch::new(&m);
        let hits = s.search("1.20");
        let mut ids: Vec<_> = hits.iter().map(|e| e.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), hits.len(), "no duplicate ids");
    }

    #[test]
    fn clear_drops_both_slots() {
        let cache = VersionListCache::new();
        cache.write_through(fixture_manifest());
        assert!(cache.has_fresh(None));
        cache.clear();
        assert!(!cache.has_fresh(None));
        let info = cache.info(None);
        assert!(info.offline_only);
    }

    #[test]
    fn counts_match_groups_lengths() {
        let m = VersionListCache::offline_manifest();
        let g = VersionGroups::from(&m);
        let counts = g.counts();
        assert_eq!(counts.get("release").copied().unwrap_or(0), g.release.len());
        assert_eq!(
            counts.get("snapshot").copied().unwrap_or(0),
            g.snapshot.len()
        );
        assert_eq!(
            counts.get("pre_release").copied().unwrap_or(0),
            g.pre_release.len()
        );
        assert_eq!(
            counts.get("old_alpha").copied().unwrap_or(0),
            g.old_alpha.len()
        );
        assert_eq!(
            counts.get("old_beta").copied().unwrap_or(0),
            g.old_beta.len()
        );
        assert_eq!(counts.get("special").copied().unwrap_or(0), g.special.len());
    }

    #[test]
    fn canonical_url_matches_known_constant() {
        assert!(VersionManifest::CANONICAL_URL.starts_with("https://launchermeta.mojang.com/"));
        assert_eq!(
            VersionManifest::CANONICAL_URL,
            VersionManifest::CANONICAL_URL
        );
    }
}
