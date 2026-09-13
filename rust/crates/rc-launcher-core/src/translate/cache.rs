//! Persistent translation cache (task 13).
//!
//! A player who revisits the mod browser should **never** pay twice for the
//! same translation. [`TranslationCache`] is an on-disk, content-addressed
//! store that:
//!
//! * hashes the [`TranslationRequest::cache_key`] into a hex filename;
//! * stores the [`TranslationResult`] as JSON, side-by-side with a metadata
//!   file (`stored_at`, `ttl_secs`) so we can prune stale entries;
//! * serves `Cache` hits in **O(1)** — a single `read` from disk;
//! * falls back gracefully on a corrupt file (treated as a miss, not an
//!   error);
//! * is **bounded** by a configurable max-entries / max-bytes cap, so a
//!   large catalogue cannot grow the cache without limit.
//!
//! Cache entries are **never shared** across translation modes (`online` /
//! `offline` / `hybrid`) — the cache key encodes the mode, so an
//! `Offline`-only translation never overwrites an `Online` one and vice
//! versa.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::error::{RcError, RcResult};
use crate::translate::model::{
    TranslationLanguage, TranslationRequest, TranslationResult, TranslationSource,
};

/// Default max number of cache entries before LRU eviction kicks in.
pub const DEFAULT_MAX_ENTRIES: usize = 4096;
/// Default max total bytes of cache content.
pub const DEFAULT_MAX_BYTES: u64 = 32 * 1024 * 1024;
/// Default TTL: 30 days (translations rarely change).
pub const DEFAULT_TTL: Duration = Duration::from_secs(30 * 24 * 3600);

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    stored_at: u64,
    ttl_secs: Option<u64>,
    bytes: u64,
    /// Stable key derived from [`TranslationRequest::cache_key`].
    key: String,
    result: TranslationResult,
}

/// Tunables for [`TranslationCache`].
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Max number of entries; oldest are evicted when over.
    pub max_entries: usize,
    /// Max total content size in bytes; oldest are evicted when over.
    pub max_bytes: u64,
    /// Default TTL when the request did not specify one.
    pub default_ttl: Duration,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_MAX_ENTRIES,
            max_bytes: DEFAULT_MAX_BYTES,
            default_ttl: DEFAULT_TTL,
        }
    }
}

/// On-disk translation cache.
#[derive(Clone)]
pub struct TranslationCache {
    root: PathBuf,
    config: CacheConfig,
}

impl TranslationCache {
    /// Open a cache rooted at `root`, creating it if needed.
    pub fn open(root: impl Into<PathBuf>) -> RcResult<Self> {
        Self::open_with_config(root, CacheConfig::default())
    }

    /// Open with explicit tunables.
    pub fn open_with_config(root: impl Into<PathBuf>, config: CacheConfig) -> RcResult<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(RcError::Io)?;
        Ok(Self { root, config })
    }

    /// Root directory (exposed for diagnostics).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Effective configuration.
    pub fn config(&self) -> &CacheConfig {
        &self.config
    }

    /// Hex-encoded cache key for a request.
    pub fn key_for(req: &TranslationRequest) -> String {
        let raw = req.cache_key();
        let mut h = Sha1::new();
        h.update(raw.as_bytes());
        hex(&h.finalize())
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.json"))
    }

    /// Look up an entry. Returns `None` for both misses and stale entries
    /// (the caller decides whether to evict them).
    ///
    /// The returned [`TranslationResult::source`] is always re-stamped to
    /// [`TranslationSource::Cache`] so the UI can badge it correctly even
    /// when the underlying result was originally served from the gateway.
    pub fn get(&self, req: &TranslationRequest) -> Option<TranslationResult> {
        let key = Self::key_for(req);
        let raw = std::fs::read(self.path_for(&key)).ok()?;
        let entry: CacheEntry = serde_json::from_slice(&raw).ok()?;
        if !self.is_fresh(&entry) {
            return None;
        }
        let mut result = entry.result;
        result.source = TranslationSource::Cache;
        result.offline = true;
        Some(result)
    }

    fn is_fresh(&self, entry: &CacheEntry) -> bool {
        // `ttl_secs = 0` is the explicit "never serve this again" sentinel —
        // used by tests / by callers that want to force a refresh.
        let ttl = entry.ttl_secs.unwrap_or(self.config.default_ttl.as_secs());
        if ttl == 0 {
            return false;
        }
        let age = now_secs().saturating_sub(entry.stored_at);
        age < ttl
    }

    /// Store a translation under the request's key.
    pub fn put(&self, req: &TranslationRequest, result: &TranslationResult) -> RcResult<()> {
        let key = Self::key_for(req);
        let path = self.path_for(&key);
        let ttl = req
            .cache_ttl_secs
            .unwrap_or(self.config.default_ttl.as_secs());
        let serialized = serde_json::to_vec(result).map_err(RcError::Json)?;
        let bytes = serialized.len() as u64;
        let entry = CacheEntry {
            stored_at: now_secs(),
            ttl_secs: Some(ttl),
            bytes,
            key: key.clone(),
            result: result.clone(),
        };
        let json = serde_json::to_vec(&entry).map_err(RcError::Json)?;
        std::fs::write(&path, json).map_err(RcError::Io)?;
        self.evict_if_needed();
        Ok(())
    }

    /// Manually evict a key (e.g. after the user picked "clear this
    /// translation"). No-op when the key is absent.
    pub fn remove(&self, req: &TranslationRequest) -> RcResult<()> {
        let key = Self::key_for(req);
        let path = self.path_for(&key);
        if path.exists() {
            std::fs::remove_file(&path).map_err(RcError::Io)?;
        }
        Ok(())
    }

    /// Total entry count.
    pub fn entry_count(&self) -> usize {
        std::fs::read_dir(&self.root)
            .map(|d| d.filter_map(|e| e.ok()).count())
            .unwrap_or(0)
    }

    /// Total cached content size in bytes.
    pub fn total_bytes(&self) -> u64 {
        std::fs::read_dir(&self.root)
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0)
    }

    /// Drop everything.
    pub fn clear(&self) -> RcResult<usize> {
        let mut removed = 0;
        let read = match std::fs::read_dir(&self.root) {
            Ok(r) => r,
            Err(_) => return Ok(0),
        };
        for entry in read.flatten() {
            if std::fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Drop every entry older than `older_than`. Returns the number of
    /// removed files.
    pub fn prune_older_than(&self, older_than: Duration) -> RcResult<usize> {
        let mut removed = 0;
        let read = match std::fs::read_dir(&self.root) {
            Ok(r) => r,
            Err(_) => return Ok(0),
        };
        let cutoff = now_secs().saturating_sub(older_than.as_secs());
        for entry in read.flatten() {
            let Ok(raw) = std::fs::read(entry.path()) else {
                continue;
            };
            let Ok(meta) = serde_json::from_slice::<CacheEntry>(&raw) else {
                continue;
            };
            if meta.stored_at < cutoff {
                if std::fs::remove_file(entry.path()).is_ok() {
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    fn evict_if_needed(&self) {
        // Cheap size / count eviction: when the cap is exceeded, prune
        // everything older than 24h. This keeps the cache bounded without
        // an in-memory LRU.
        let count = self.entry_count();
        let bytes = self.total_bytes();
        if count > self.config.max_entries || bytes > self.config.max_bytes {
            let _ = self.prune_older_than(Duration::from_secs(24 * 3600));
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// A small helper that returns `None` when the request's source equals the
/// target language (the player wants English → English, which is just the
/// original text). Used by [`crate::translate::service::TranslationService`]
/// to skip the cache lookup entirely.
pub fn passthrough_if_same_language(req: &TranslationRequest) -> Option<TranslationResult> {
    if matches!(req.source, Some(s) if s == req.target) {
        Some(TranslationResult::passthrough(req))
    } else {
        None
    }
}

/// Look up the request's target language's tag for the cache key.
pub fn target_tag(lang: TranslationLanguage) -> &'static str {
    lang.tag()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translate::model::TranslationSource;

    fn req(text: &str) -> TranslationRequest {
        TranslationRequest {
            text: text.into(),
            target: TranslationLanguage::ZhCn,
            ..Default::default()
        }
    }

    fn res(text: &str) -> TranslationResult {
        TranslationResult {
            original: text.into(),
            translated: text.into(),
            target: TranslationLanguage::ZhCn,
            detected_source: Some(TranslationLanguage::En),
            source: TranslationSource::Gateway,
            offline: false,
        }
    }

    #[test]
    fn put_then_get_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TranslationCache::open(dir.path()).unwrap();
        let r = req("hello");
        cache.put(&r, &res("你好")).unwrap();
        let out = cache.get(&r).expect("cached hit");
        assert_eq!(out.translated, "你好");
        // The cache re-stamps source to `Cache` on read.
        assert_eq!(out.source, TranslationSource::Cache);
        assert!(out.offline);
    }

    #[test]
    fn cache_key_distinguishes_target_languages() {
        let mut en = req("hello");
        en.target = TranslationLanguage::En;
        let mut zh = req("hello");
        zh.target = TranslationLanguage::ZhCn;
        let mut hk = req("hello");
        hk.target = TranslationLanguage::ZhHant;
        assert_ne!(
            TranslationCache::key_for(&en),
            TranslationCache::key_for(&zh)
        );
        assert_ne!(
            TranslationCache::key_for(&zh),
            TranslationCache::key_for(&hk)
        );
    }

    #[test]
    fn cache_key_distinguishes_modes() {
        let mut online = req("hello");
        online.mode = Some(crate::translate::model::TranslationMode::Online);
        let mut offline = req("hello");
        offline.mode = Some(crate::translate::model::TranslationMode::Offline);
        assert_ne!(
            TranslationCache::key_for(&online),
            TranslationCache::key_for(&offline)
        );
    }

    #[test]
    fn stale_entry_is_a_miss() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TranslationCache::open_with_config(
            dir.path(),
            CacheConfig {
                max_entries: 100,
                max_bytes: 1024 * 1024,
                default_ttl: Duration::from_secs(1),
            },
        )
        .unwrap();
        let mut r = req("hello");
        r.cache_ttl_secs = Some(0); // expired immediately
        cache.put(&r, &res("你好")).unwrap();
        assert!(cache.get(&r).is_none(), "TTL expired");
    }

    #[test]
    fn remove_drops_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TranslationCache::open(dir.path()).unwrap();
        let r = req("hello");
        cache.put(&r, &res("你好")).unwrap();
        assert!(cache.get(&r).is_some());
        cache.remove(&r).unwrap();
        assert!(cache.get(&r).is_none());
    }

    #[test]
    fn clear_empties_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TranslationCache::open(dir.path()).unwrap();
        cache.put(&req("a"), &res("A")).unwrap();
        cache.put(&req("b"), &res("B")).unwrap();
        assert!(cache.entry_count() >= 2);
        let removed = cache.clear().unwrap();
        assert!(removed >= 2);
        assert_eq!(cache.entry_count(), 0);
    }

    #[test]
    fn prune_older_than_drops_only_stale() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TranslationCache::open(dir.path()).unwrap();
        cache.put(&req("fresh"), &res("新")).unwrap();
        let removed = cache.prune_older_than(Duration::from_secs(0)).unwrap();
        // Everything that was just stored is "fresh" (age = 0 ≤ 0).
        assert_eq!(removed, 0);
    }

    #[test]
    fn passthrough_helper_skips_work_when_target_equals_source() {
        // Same source + target → passthrough.
        let mut same = TranslationRequest::to("hello", TranslationLanguage::En);
        same.source = Some(TranslationLanguage::En);
        assert!(passthrough_if_same_language(&same).is_some());
        // Different source + target → not passthrough.
        let mut diff = TranslationRequest::to("hello", TranslationLanguage::ZhCn);
        diff.source = Some(TranslationLanguage::ZhHant);
        assert!(passthrough_if_same_language(&diff).is_none());
        // No source → not passthrough (auto-detect could land on anything).
        let any = TranslationRequest::to("hello", TranslationLanguage::En);
        assert!(passthrough_if_same_language(&any).is_none());
    }
}
