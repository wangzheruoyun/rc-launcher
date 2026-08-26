//! On-disk content cache + offline degradation (task 19).
//!
//! China-mainland networks are flaky: a mirror 404s, a TLS handshake stalls,
//! DNS gets poisoned. Rather than failing the whole install, [`fetch_cached`]
//! **degrades** — on a transient network error it serves a previously cached
//! copy if one exists, so the launcher keeps working while the link is bad. A
//! `force_offline` flag skips the network entirely (e.g. aeroplane mode / the
//! user explicitly choosing "use cached assets").
//!
//! [`CacheStore`] is a tiny, dependency-free, disk-backed key→bytes store with a
//! side-car JSON metadata file recording when each entry was stored (so
//! [`CachePolicy`] can report staleness). Callers hash the URL into a stable
//! hex key via [`cache_key`].

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::download::HttpSource;
use crate::error::{RcError, RcResult};
use crate::robust::retry::{retry, RetryPolicy};

/// Maximum age of a cached entry before it is considered stale. A stale entry is
/// still usable as a *degraded* fallback, but [`CacheStore::is_fresh`] returns
/// `false` so the UI can warn "served from cache".
#[derive(Debug, Clone)]
pub struct CachePolicy {
    pub max_age: Option<Duration>,
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            max_age: Some(Duration::from_secs(7 * 24 * 3600)),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct EntryMeta {
    stored_at: u64,
    ttl_secs: Option<u64>,
}

/// A disk-backed content cache keyed by caller-supplied (hex) keys.
pub struct CacheStore {
    root: PathBuf,
}

impl CacheStore {
    /// Open (creating if needed) a cache rooted at `root`.
    pub fn open(root: impl Into<PathBuf>) -> RcResult<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(RcError::Io)?;
        Ok(Self { root })
    }

    /// The cache root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn bin_path(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.bin"))
    }
    fn meta_path(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.meta"))
    }

    /// Read a cached entry, if present.
    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        tokio::fs::read(self.bin_path(key)).await.ok()
    }

    /// Whether a (non-empty) entry exists for `key`.
    pub async fn contains(&self, key: &str) -> bool {
        tokio::fs::metadata(self.bin_path(key)).await.is_ok()
    }

    /// Whether the cached entry for `key` is still within its freshness window.
    pub async fn is_fresh(&self, key: &str, policy: &CachePolicy) -> bool {
        let Ok(bytes) = tokio::fs::read(self.meta_path(key)).await else {
            return false;
        };
        let Ok(meta) = serde_json::from_slice::<EntryMeta>(&bytes) else {
            return false;
        };
        let age_limit = match (meta.ttl_secs, policy.max_age) {
            (Some(ttl), _) => meta.stored_at.saturating_add(ttl),
            (None, Some(max)) => meta.stored_at.saturating_add(max.as_secs()),
            (None, None) => return true,
        };
        now_secs() <= age_limit
    }

    /// Store `data` under `key` with an optional TTL.
    pub async fn put(&self, key: &str, data: &[u8], ttl: Option<Duration>) -> RcResult<()> {
        tokio::fs::write(self.bin_path(key), data)
            .await
            .map_err(RcError::Io)?;
        let meta = EntryMeta {
            stored_at: now_secs(),
            ttl_secs: ttl.map(|d| d.as_secs()),
        };
        let bytes = serde_json::to_vec(&meta).map_err(RcError::Json)?;
        tokio::fs::write(self.meta_path(key), bytes)
            .await
            .map_err(RcError::Io)?;
        Ok(())
    }

    /// Remove an entry (both the data and its metadata).
    pub async fn remove(&self, key: &str) -> RcResult<()> {
        let _ = tokio::fs::remove_file(self.bin_path(key)).await;
        let _ = tokio::fs::remove_file(self.meta_path(key)).await;
        Ok(())
    }

    /// Number of cached data entries currently present (best-effort: a directory
    /// read error yields `0` rather than failing the caller).
    pub async fn entry_count(&self) -> usize {
        let mut count = 0usize;
        let mut rd = match tokio::fs::read_dir(&self.root).await {
            Ok(rd) => rd,
            Err(_) => return 0,
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            if let Some(name) = entry.file_name().to_str() {
                if name.ends_with(".bin") {
                    count += 1;
                }
            }
        }
        count
    }

    /// Approximate total size in bytes of every file in the cache directory
    /// (data + side-car metadata). Best-effort.
    pub async fn total_size_bytes(&self) -> u64 {
        let mut total = 0u64;
        let mut rd = match tokio::fs::read_dir(&self.root).await {
            Ok(rd) => rd,
            Err(_) => return 0,
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            if let Ok(meta) = entry.metadata().await {
                if meta.is_file() {
                    total += meta.len();
                }
            }
        }
        total
    }

    /// Remove *every* cached entry (data + metadata). Returns the number of files
    /// deleted. Best-effort: a single undeletable file does not abort the sweep
    /// (task 19 — cache management).
    pub async fn clear(&self) -> RcResult<u64> {
        let mut rd = tokio::fs::read_dir(&self.root).await.map_err(RcError::Io)?;
        let mut removed = 0u64;
        while let Some(entry) = rd.next_entry().await? {
            if entry.metadata().await.map(|m| m.is_file()).unwrap_or(false)
                && tokio::fs::remove_file(entry.path()).await.is_ok()
            {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Delete entries whose cached copy is older than `older_than`, according to
    /// the side-car metadata `stored_at`. Entries without metadata are left
    /// untouched (treated as still fresh). Returns the number of entries pruned
    /// (task 19 — cache management).
    pub async fn prune_older_than(&self, older_than: Duration) -> RcResult<u64> {
        let threshold = now_secs().saturating_sub(older_than.as_secs());
        let mut rd = tokio::fs::read_dir(&self.root).await.map_err(RcError::Io)?;
        let mut to_remove: Vec<String> = Vec::new();
        while let Some(entry) = rd.next_entry().await? {
            let path = entry.path();
            if !path.to_string_lossy().ends_with(".meta") {
                continue;
            }
            if let Ok(bytes) = tokio::fs::read(&path).await {
                if let Ok(meta) = serde_json::from_slice::<EntryMeta>(&bytes) {
                    if meta.stored_at < threshold {
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            to_remove.push(stem.to_string());
                        }
                    }
                }
            }
        }
        let mut pruned = 0u64;
        for stem in to_remove {
            let _ = tokio::fs::remove_file(self.bin_path(&stem)).await;
            let _ = tokio::fs::remove_file(self.meta_path(&stem)).await;
            pruned += 1;
        }
        Ok(pruned)
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Stable hex key for a URL (optionally with a range suffix). Uses SHA-1 so the
/// key is fixed-width, path-safe and independent of URL length.
pub fn cache_key(url: &str) -> String {
    let mut h = Sha1::new();
    h.update(url.as_bytes());
    format!("{:x}", h.finalize())
}

/// Like [`cache_key`] but also folds the requested byte range into the key, so a
/// partial range fetch is cached under a *distinct* key from the full resource
/// and can never clobber the whole-file entry (task 19 robustness fix).
pub fn cache_key_range(url: &str, start: u64, end: Option<u64>) -> String {
    let suffix = match end {
        Some(e) => format!("{}:{}", start, e),
        None => format!("{}:", start),
    };
    cache_key(&format!("{}#{}", url, suffix))
}

/// The outcome of a [`fetch_cached`] call.
#[derive(Debug, Clone)]
pub struct Cached {
    pub data: Vec<u8>,
    /// `true` if the bytes came from the local cache, not the network.
    pub from_cache: bool,
    /// `true` if the network failed/was skipped and we degraded to the cache.
    pub degraded: bool,
}

/// Fetch `url` (byte range `[start, end]`) through `source`, caching the result
/// on success.
///
/// On a **transient** network failure the function **degrades** to the cached
/// copy (if any) instead of erroring — this is the offline path that keeps the
/// launcher usable on a flaky connection. A non-transient failure (e.g. a 404)
/// is *not* degraded: the resource genuinely does not exist, so serving a stale
/// copy would be wrong.
///
/// With `force_offline`, the network is skipped entirely; a cache miss returns
/// [`RcError::Offline`].
pub async fn fetch_cached(
    source: &dyn HttpSource,
    cache: &CacheStore,
    key: &str,
    url: &str,
    start: u64,
    end: Option<u64>,
    retry_policy: &RetryPolicy,
    force_offline: bool,
) -> RcResult<Cached> {
    // We only ever cache / serve the *entire* resource. A partial-range fetch
    // must not clobber the whole-file cache, nor be answered from a whole-file
    // cache (that would hand back the wrong bytes). Range requests therefore
    // bypass the cache entirely (task 19 robustness fix).
    let is_full = start == 0 && end.is_none();

    if !force_offline {
        let fetched = retry(retry_policy, || source.fetch_range(url, start, end)).await;
        match fetched {
            Ok(r) => {
                // Only a *full* fetch is cached; a range fetch is never persisted
                // (a cache-write failure must also never fail the fetch).
                if is_full {
                    let _ = cache.put(key, &r.bytes, None).await;
                }
                return Ok(Cached {
                    data: r.bytes,
                    from_cache: false,
                    degraded: false,
                });
            }
            Err(e) => {
                if !e.is_transient() || !is_full {
                    // Non-transient (404, bad status, checksum) or a range
                    // request we cannot answer from a whole-file cache: propagate.
                    return Err(e);
                }
                // Transient full-fetch failure: fall through to cache degradation.
            }
        }
    }

    // Offline / degraded path — only valid for a full fetch.
    if is_full && cache.contains(key).await {
        if let Some(data) = cache.get(key).await {
            return Ok(Cached {
                data,
                from_cache: true,
                degraded: true,
            });
        }
    }
    Err(RcError::Offline(format!(
        "no cached copy of {url} and the network is unavailable"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::FetchResult;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};

    struct MockSource {
        // A script of results popped per call (FIFO). When empty, returns `Ok`.
        script: Mutex<Vec<RcResult<FetchResult>>>,
        calls: AtomicU32,
    }

    impl MockSource {
        fn new(script: Vec<RcResult<FetchResult>>) -> Arc<Self> {
            Arc::new(Self {
                script: Mutex::new(script),
                calls: AtomicU32::new(0),
            })
        }
    }

    #[async_trait]
    impl HttpSource for MockSource {
        async fn fetch_range(
            &self,
            _url: &str,
            _start: u64,
            _end: Option<u64>,
        ) -> RcResult<FetchResult> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            // When the script is exhausted keep returning a transient error so a
            // retrying caller (see `fetch_cached`) keeps failing and eventually
            // degrades to the cache, rather than spuriously "succeeding".
            let mut g = self.script.lock().unwrap();
            match g.pop() {
                Some(r) => r,
                None => Err(RcError::Network("mock source script exhausted".into())),
            }
        }
    }

    fn ok(bytes: &[u8]) -> RcResult<FetchResult> {
        Ok(FetchResult {
            bytes: bytes.to_vec(),
            total_size: bytes.len() as u64,
            supports_range: true,
        })
    }

    #[tokio::test]
    async fn cache_key_is_stable_and_distinct() {
        assert_eq!(cache_key("https://a/x"), cache_key("https://a/x"));
        assert_ne!(cache_key("https://a/x"), cache_key("https://a/y"));
    }

    #[tokio::test]
    async fn fetch_from_network_caches_result() {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::open(dir.path()).unwrap();
        let src = MockSource::new(vec![ok(b"hello")]);
        let key = cache_key("https://example/file");
        let r = fetch_cached(
            &*src,
            &store,
            &key,
            "https://example/file",
            0,
            None,
            &RetryPolicy::for_tests(),
            false,
        )
        .await
        .unwrap();
        assert_eq!(r.data, b"hello");
        assert!(!r.from_cache);
        assert!(!r.degraded);
        assert!(store.contains(&key).await);
        assert_eq!(store.get(&key).await.unwrap(), b"hello");
    }

    #[tokio::test]
    async fn transient_failure_degrades_to_cache() {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::open(dir.path()).unwrap();
        let key = cache_key("https://example/file");
        store.put(&key, b"cached-copy", None).await.unwrap();

        let src = MockSource::new(vec![Err(RcError::Network("timeout".into()))]);
        let r = fetch_cached(
            &*src,
            &store,
            &key,
            "https://example/file",
            0,
            None,
            &RetryPolicy::for_tests(),
            false,
        )
        .await
        .unwrap();
        assert_eq!(r.data, b"cached-copy");
        assert!(r.from_cache);
        assert!(r.degraded);
    }

    #[tokio::test]
    async fn non_transient_failure_is_not_degraded() {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::open(dir.path()).unwrap();
        let key = cache_key("https://example/missing");
        store.put(&key, b"stale", None).await.unwrap();

        // A 404 is a Download (recoverable, non-transient) error.
        let src = MockSource::new(vec![Err(RcError::Download(
            "unexpected HTTP status 404".into(),
        ))]);
        let r = fetch_cached(
            &*src,
            &store,
            &key,
            "https://example/missing",
            0,
            None,
            &RetryPolicy::for_tests(),
            false,
        )
        .await;
        assert!(r.is_err());
        assert!(!r.unwrap_err().is_transient());
    }

    #[tokio::test]
    async fn force_offline_serves_cache() {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::open(dir.path()).unwrap();
        let key = cache_key("https://example/file");
        store.put(&key, b"offline-data", None).await.unwrap();

        let src = MockSource::new(vec![]); // never used
        let r = fetch_cached(
            &*src,
            &store,
            &key,
            "https://example/file",
            0,
            None,
            &RetryPolicy::for_tests(),
            true,
        )
        .await
        .unwrap();
        assert_eq!(r.data, b"offline-data");
        assert!(r.from_cache);
        assert!(r.degraded);
    }

    #[tokio::test]
    async fn force_offline_miss_is_offline_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::open(dir.path()).unwrap();
        let key = cache_key("https://example/file");
        let src = MockSource::new(vec![]);
        let r = fetch_cached(
            &*src,
            &store,
            &key,
            "https://example/file",
            0,
            None,
            &RetryPolicy::for_tests(),
            true,
        )
        .await;
        assert!(matches!(r, Err(RcError::Offline(_))));
    }

    #[tokio::test]
    async fn cache_freshness_respects_policy() {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::open(dir.path()).unwrap();
        let key = cache_key("https://example/file");
        store
            .put(&key, b"x", Some(Duration::from_secs(60)))
            .await
            .unwrap();
        let fresh = CachePolicy {
            max_age: Some(Duration::from_secs(10)),
        };
        // ttl=60s overrides the 10s policy window -> still fresh
        assert!(store.is_fresh(&key, &fresh).await);
        // a 0s ttl-equivalent max_age with huge policy window is also fresh
        let wide = CachePolicy {
            max_age: Some(Duration::from_secs(3600)),
        };
        assert!(store.is_fresh(&key, &wide).await);
        // missing key is never fresh
        assert!(!store.is_fresh(&cache_key("nope"), &wide).await);
    }

    #[tokio::test]
    async fn range_fetch_is_never_cached() {
        // A partial range fetch must not clobber the whole-file cache.
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::open(dir.path()).unwrap();
        let key = cache_key_range("https://example/file", 0, Some(3));
        let src = MockSource::new(vec![ok(b"hel")]);
        let r = fetch_cached(
            &*src,
            &store,
            &key,
            "https://example/file",
            0,
            Some(3),
            &RetryPolicy::for_tests(),
            false,
        )
        .await
        .unwrap();
        assert_eq!(r.data, b"hel");
        assert!(!r.from_cache);
        assert!(!r.degraded);
        // Nothing was persisted under the range key.
        assert!(!store.contains(&key).await);
        // And the full-file key was not clobbered either.
        assert!(!store.contains(&cache_key("https://example/file")).await);
    }

    #[tokio::test]
    async fn range_transient_failure_is_not_degraded() {
        // We cannot answer a range request from the whole-file cache, so a
        // transient network error for a range must surface (not silently serve
        // the wrong bytes).
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::open(dir.path()).unwrap();
        let full_key = cache_key("https://example/file");
        store
            .put(&full_key, b"full-file-contents", None)
            .await
            .unwrap();

        let key = cache_key_range("https://example/file", 0, Some(3));
        let src = MockSource::new(vec![Err(RcError::Network("timeout".into()))]);
        let r = fetch_cached(
            &*src,
            &store,
            &key,
            "https://example/file",
            0,
            Some(3),
            &RetryPolicy::for_tests(),
            false,
        )
        .await;
        // A range request must NOT be answered from the whole-file cache (that
        // would hand back the wrong bytes); it surfaces the network error.
        assert!(matches!(r, Err(RcError::Network(_))));
    }

    #[tokio::test]
    async fn cache_management_count_size_clear() {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::open(dir.path()).unwrap();
        store
            .put(&cache_key("a"), b"aaaa", Some(Duration::from_secs(60)))
            .await
            .unwrap();
        store.put(&cache_key("b"), b"bb", None).await.unwrap();
        assert_eq!(store.entry_count().await, 2);
        assert!(store.total_size_bytes().await >= 6);
        // older_than=0 prunes nothing that was just stored.
        assert_eq!(
            store
                .prune_older_than(Duration::from_secs(0))
                .await
                .unwrap(),
            0
        );
        assert_eq!(store.entry_count().await, 2);
        // Clear wipes both data and metadata files.
        let removed = store.clear().await.unwrap();
        assert_eq!(removed, 4);
        assert_eq!(store.entry_count().await, 0);
    }

    #[tokio::test]
    async fn prune_older_than_removes_only_stale_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::open(dir.path()).unwrap();
        let fresh_key = cache_key("fresh");
        store.put(&fresh_key, b"x", None).await.unwrap();

        // Back-date one entry's metadata to simulate a stale, ancient cache entry.
        let stale_key = cache_key("stale");
        store.put(&stale_key, b"y", None).await.unwrap();
        let backdated = EntryMeta {
            stored_at: 1,
            ttl_secs: None,
        };
        let bytes = serde_json::to_vec(&backdated).unwrap();
        tokio::fs::write(store.meta_path(&stale_key), bytes)
            .await
            .unwrap();

        let pruned = store
            .prune_older_than(Duration::from_secs(3600))
            .await
            .unwrap();
        assert_eq!(pruned, 1);
        assert_eq!(store.entry_count().await, 1);
        assert!(store.contains(&fresh_key).await);
        assert!(!store.contains(&stale_key).await);
    }
}
