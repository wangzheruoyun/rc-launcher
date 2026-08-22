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
    if !force_offline {
        let fetched = retry(retry_policy, || source.fetch_range(url, start, end)).await;
        match fetched {
            Ok(r) => {
                // Update the cache; a cache-write failure must never fail the
                // fetch, so it is intentionally ignored.
                let _ = cache.put(key, &r.bytes, None).await;
                return Ok(Cached {
                    data: r.bytes,
                    from_cache: false,
                    degraded: false,
                });
            }
            Err(e) => {
                if !e.is_transient() {
                    // Non-transient (404, bad status, checksum) — do not degrade.
                    return Err(e);
                }
                // transient: fall through to cache degradation.
            }
        }
    }

    // Offline / degraded path.
    if cache.contains(key).await {
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
}
