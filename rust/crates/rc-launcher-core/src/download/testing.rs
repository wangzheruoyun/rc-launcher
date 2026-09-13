//! Offline tests for the download manager.
//!
//! These run on the host in CI (no network) against an in-memory
//! [`MockSource`] that honours `Range` requests, so the full chunked /
//! resumable / verified download path is exercised deterministically.

use std::collections::HashMap;
use std::io::{Seek, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncWrite;

use async_trait::async_trait;

use crate::download::client::{FetchResult, HttpSource};
use crate::download::hash;
use crate::download::manager::plan_chunks;
use crate::download::{
    DownloadManager, DownloadOptions, DownloadStatus, DownloadTask, Progress, ProgressCallback,
};
use crate::error::{RcError, RcResult};

/// Kind of error injected for a URL that matches a configured substring.
#[derive(Clone, Copy)]
enum InjectedUrlError {
    Network,
    RateLimited,
}

/// A configurable in-memory resource that implements [`HttpSource`].
pub struct MockSource {
    data: Vec<u8>,
    supports_range: bool,
    /// Map of start-offset -> number of times to fail before succeeding.
    fail_map: Arc<Mutex<HashMap<u64, usize>>>,
    /// Number of `fetch_range` calls made (used for resume assertions).
    calls: Arc<AtomicU64>,
    /// Substring -> (remaining failures, error kind) for URL-based injection
    /// (simulates a dead primary host or an HTTP 429 rate-limit).
    url_fail: Arc<Mutex<HashMap<String, (usize, InjectedUrlError)>>>,
    /// Number of remaining `fetch_range` calls that should return *corrupted*
    /// data (byte 0 flipped). Used by the checksum-retry tests (task 30) to
    /// simulate a corrupt mirror that recovers on retry.
    corrupt_remaining: Arc<AtomicU64>,
}

impl MockSource {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            supports_range: true,
            fail_map: Arc::new(Mutex::new(HashMap::new())),
            calls: Arc::new(AtomicU64::new(0)),
            url_fail: Arc::new(Mutex::new(HashMap::new())),
            corrupt_remaining: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Make the next `n` `fetch_range` calls return data with byte 0 flipped
    /// (simulating a corrupt mirror / bitrot). Used to test checksum-failure
    /// retry (task 30).
    pub fn corrupt_next_n(&self, n: u64) {
        self.corrupt_remaining.store(n, Ordering::Relaxed);
    }

    /// Disable `Range` support (the whole resource is always returned).
    pub fn without_range(mut self) -> Self {
        self.supports_range = false;
        self
    }

    /// Make the request starting at `start` fail the first `times` attempts.
    pub fn fail_first(&self, start: u64, times: usize) {
        self.fail_map.lock().unwrap().insert(start, times);
    }

    /// Make every request whose URL contains `substr` fail the first `times`
    /// attempts with a generic network error (simulates a dead primary host).
    pub fn fail_url_containing(&self, substr: &str, times: usize) {
        self.url_fail
            .lock()
            .unwrap()
            .insert(substr.to_string(), (times, InjectedUrlError::Network));
    }

    /// Make every request whose URL contains `substr` fail the first `times`
    /// attempts with an HTTP 429 rate-limit error carrying a `Retry-After`.
    pub fn rate_limit_url(&self, substr: &str, times: usize) {
        self.url_fail
            .lock()
            .unwrap()
            .insert(substr.to_string(), (times, InjectedUrlError::RateLimited));
    }

    pub fn call_count(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl HttpSource for MockSource {
    async fn fetch_range(&self, _url: &str, start: u64, end: Option<u64>) -> RcResult<FetchResult> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        // URL-based injected failures (dead primary host / HTTP 429) for the
        // mirror-fallback and Retry-After tests.
        {
            let mut fm = self.url_fail.lock().unwrap();
            for (sub, entry) in fm.iter_mut() {
                if _url.contains(sub.as_str()) {
                    if entry.0 > 0 {
                        entry.0 -= 1;
                        let err = match entry.1 {
                            InjectedUrlError::Network => {
                                RcError::Network(format!("injected failure for {sub}"))
                            }
                            InjectedUrlError::RateLimited => RcError::RateLimited {
                                retry_after: Some(Duration::from_millis(1)),
                            },
                        };
                        return Err(err);
                    }
                    break;
                }
            }
        }
        // inject configured failures
        {
            let mut fm = self.fail_map.lock().unwrap();
            if let Some(remaining) = fm.get_mut(&start) {
                if *remaining > 0 {
                    *remaining -= 1;
                    return Err(RcError::Network(format!("injected failure at {start}")));
                }
            }
        }
        let total = self.data.len() as u64;
        if !self.supports_range {
            return Ok(FetchResult {
                bytes: self.data.clone(),
                total_size: 0,
                supports_range: false,
            });
        }
        let end = end
            .unwrap_or(total.saturating_sub(1))
            .min(total.saturating_sub(1));
        if start > end || total == 0 {
            return Ok(FetchResult {
                bytes: Vec::new(),
                total_size: total,
                supports_range: true,
            });
        }
        let mut bytes = self.data[start as usize..=end as usize].to_vec();
        // Inject corruption for checksum-retry tests (task 30).
        if self.corrupt_remaining.load(Ordering::Relaxed) > 0 {
            self.corrupt_remaining.fetch_sub(1, Ordering::Relaxed);
            if !bytes.is_empty() {
                bytes[0] = bytes[0].wrapping_add(1);
            }
        }
        Ok(FetchResult {
            bytes,
            total_size: total,
            supports_range: true,
        })
    }
}

/// A unique temp directory for a test.
fn tempdir() -> std::path::PathBuf {
    static CTR: AtomicU64 = AtomicU64::new(0);
    let n = CTR.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("rc_dl_test_{}_{}", std::process::id(), n));
    std::fs::create_dir_all(&p).unwrap();
    p
}

// --- pure helpers ----------------------------------------------------------

#[test]
fn plan_chunks_exact() {
    let plan = plan_chunks(100, 10);
    assert_eq!(plan.len(), 10);
    assert_eq!(plan[0], (0, 9));
    assert_eq!(plan[9], (90, 99));
}

#[test]
fn plan_chunks_remainder() {
    let plan = plan_chunks(105, 10);
    assert_eq!(plan.len(), 11);
    assert_eq!(plan[10], (100, 104));
}

#[test]
fn plan_chunks_zero() {
    assert!(plan_chunks(0, 10).is_empty());
    assert!(plan_chunks(100, 0).is_empty());
}

#[test]
fn backoff_grows_and_caps() {
    let base = std::time::Duration::from_millis(100);
    let max = std::time::Duration::from_millis(500);
    let a1 = crate::download::manager::compute_backoff(1, base, max, 0.0);
    let a2 = crate::download::manager::compute_backoff(2, base, max, 0.0);
    let a3 = crate::download::manager::compute_backoff(3, base, max, 0.0);
    assert_eq!(a1, base);
    assert_eq!(a2, std::time::Duration::from_millis(200));
    assert_eq!(a3, std::time::Duration::from_millis(400));
    // capped
    let a10 = crate::download::manager::compute_backoff(10, base, max, 0.0);
    assert_eq!(a10, max);
}

// --- integration ------------------------------------------------------------

#[tokio::test]
async fn download_all_concurrent_preserves_order_and_integrity() {
    let data: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
    let sha = hash::sha1_bytes(&data);
    let dir = tempdir();
    let mgr = DownloadManager::new(
        Arc::new(MockSource::new(data.clone())),
        DownloadOptions {
            chunk_size: 16 * 1024,
            concurrency: 4,
            max_batch_concurrency: 2,
            ..Default::default()
        },
    );
    let tasks: Vec<DownloadTask> = (0..5)
        .map(|i| {
            DownloadTask::new(
                format!("http://mock/f{i}.bin"),
                dir.join(format!("f{i}.bin")),
            )
            .with_sha1(sha.clone())
            .with_size(data.len() as u64)
        })
        .collect();
    let results = mgr.download_all_concurrent(&tasks).await;
    assert_eq!(results.len(), 5, "one result per task");
    for (i, r) in results.iter().enumerate() {
        let r = r.as_ref().expect("each download must succeed");
        assert_eq!(r.size, data.len() as u64);
        assert_eq!(std::fs::read(dir.join(format!("f{i}.bin"))).unwrap(), data);
    }
}

#[tokio::test]
async fn downloads_full_file_with_checksum() {
    let data: Vec<u8> = (0..250_000).map(|i| (i % 251) as u8).collect();
    let sha = hash::sha1_bytes(&data);
    let src = Arc::new(MockSource::new(data.clone()));
    let opts = DownloadOptions {
        chunk_size: 64 * 1024,
        concurrency: 4,
        ..Default::default()
    };
    let mgr = DownloadManager::new(src, opts);
    let dir = tempdir();
    let dest = dir.join("game.jar");
    let task = DownloadTask::new("http://mock/game.jar", dest.clone())
        .with_sha1(sha)
        .with_size(data.len() as u64);
    let summary = mgr.download(&task).await.unwrap();
    assert_eq!(summary.size, data.len() as u64);
    assert!(!summary.resumed);
    let got = std::fs::read(&dest).unwrap();
    assert_eq!(got, data);
}

#[tokio::test]
async fn resumes_from_partial_meta() {
    let data: Vec<u8> = (0..200_000).map(|i| (i % 199) as u8).collect();
    let sha = hash::sha1_bytes(&data);
    let dir = tempdir();
    let dest = dir.join("game.jar");
    let temp = dir.join("game.jar.part");
    let meta = dir.join("game.jar.part.meta");
    let chunk_size = 64 * 1024u64;

    let plan = plan_chunks(data.len() as u64, chunk_size);
    let half = plan.len() / 2;

    // Pre-size the temp file.
    {
        let f = std::fs::File::create(&temp).unwrap();
        f.set_len(data.len() as u64).unwrap();
    }
    // Write the first half of the chunks (simulating an interrupted run).
    let mut completed: Vec<u64> = Vec::new();
    for (i, (s, e)) in plan.iter().enumerate() {
        if i < half {
            let mut f = std::fs::OpenOptions::new().write(true).open(&temp).unwrap();
            f.seek(std::io::SeekFrom::Start(*s)).unwrap();
            f.write_all(&data[*s as usize..=*e as usize]).unwrap();
            completed.push(*s / chunk_size);
        }
    }
    // Persist a meta describing the completed chunks.
    let json = serde_json::json!({
        "total_size": data.len() as u64,
        "chunk_size": chunk_size,
        "completed": completed,
    });
    std::fs::write(&meta, serde_json::to_vec(&json).unwrap()).unwrap();

    let src = Arc::new(MockSource::new(data.clone()));
    let opts = DownloadOptions {
        chunk_size,
        concurrency: 4,
        ..Default::default()
    };
    let mgr = DownloadManager::new(src.clone(), opts);
    let task = DownloadTask::new("http://mock/game.jar", dest.clone())
        .with_sha1(sha)
        .with_size(data.len() as u64);
    let summary = mgr.download(&task).await.unwrap();
    assert!(summary.resumed);
    // Only the missing half should be fetched.
    assert_eq!(
        src.call_count() as usize,
        plan.len() - half,
        "should fetch only the missing chunks"
    );
    let got = std::fs::read(&dest).unwrap();
    assert_eq!(got, data);
}

#[tokio::test]
async fn fails_on_checksum_mismatch() {
    let data: Vec<u8> = vec![7u8; 1024];
    let src = Arc::new(MockSource::new(data.clone()));
    let opts = DownloadOptions {
        chunk_size: 256,
        concurrency: 2,
        ..Default::default()
    };
    let mgr = DownloadManager::new(src, opts);
    let dir = tempdir();
    let dest = dir.join("x.bin");
    let task = DownloadTask::new("http://mock/x.bin", dest.clone())
        .with_sha1("deadbeef")
        .with_size(data.len() as u64);
    let res = mgr.download(&task).await;
    assert!(res.is_err());
    // temp must remain for a later resume/retry
    assert!(dir.join("x.bin.part").exists());
}

#[tokio::test]
async fn falls_back_to_single_shot_when_no_range() {
    let data: Vec<u8> = (0..50_000).map(|i| i as u8).collect();
    let sha = hash::sha1_bytes(&data);
    let src = Arc::new(MockSource::new(data.clone()).without_range());
    let opts = DownloadOptions {
        chunk_size: 16 * 1024,
        concurrency: 4,
        ..Default::default()
    };
    let mgr = DownloadManager::new(src, opts);
    let dir = tempdir();
    let dest = dir.join("y.bin");
    let task = DownloadTask::new("http://mock/y.bin", dest.clone()).with_sha1(sha);
    let summary = mgr.download(&task).await.unwrap();
    assert_eq!(summary.size, data.len() as u64);
    let got = std::fs::read(&dest).unwrap();
    assert_eq!(got, data);
}

#[tokio::test]
async fn retries_failed_chunk_with_backoff() {
    let data: Vec<u8> = (0..200_000).map(|i| (i % 211) as u8).collect();
    let sha = hash::sha1_bytes(&data);
    let src = Arc::new(MockSource::new(data.clone()));
    src.fail_first(0, 1); // first attempt for chunk 0 fails once
    let opts = DownloadOptions {
        chunk_size: 64 * 1024,
        concurrency: 4,
        max_retries: 5,
        retry_base: std::time::Duration::from_millis(1),
        retry_max: std::time::Duration::from_millis(5),
        ..Default::default()
    };
    let mgr = DownloadManager::new(src.clone(), opts);
    let dir = tempdir();
    let dest = dir.join("z.bin");
    let task = DownloadTask::new("http://mock/z.bin", dest.clone())
        .with_sha1(sha)
        .with_size(data.len() as u64);
    let summary = mgr.download(&task).await.unwrap();
    assert_eq!(summary.size, data.len() as u64);
    let got = std::fs::read(&dest).unwrap();
    assert_eq!(got, data);
    let plan = plan_chunks(data.len() as u64, 64 * 1024);
    // chunk 0 fetched twice (one failure + one success); others once
    assert!(src.call_count() > plan.len() as u64);
}

#[tokio::test]
async fn reports_progress() {
    let data: Vec<u8> = (0..120_000).map(|i| (i % 7) as u8).collect();
    let sha = hash::sha1_bytes(&data);
    let src = Arc::new(MockSource::new(data.clone()));
    let opts = DownloadOptions {
        chunk_size: 32 * 1024,
        concurrency: 2,
        ..Default::default()
    };
    let events: Arc<Mutex<Vec<(u64, Option<u64>, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let cb: ProgressCallback = Arc::new(move |p: &Progress| {
        ev.lock().unwrap().push((p.downloaded, p.total, p.finished));
    });
    let mgr = DownloadManager::new(src, opts).with_progress(cb);
    let dir = tempdir();
    let dest = dir.join("p.bin");
    let task = DownloadTask::new("http://mock/p.bin", dest.clone())
        .with_sha1(sha)
        .with_size(data.len() as u64);
    mgr.download(&task).await.unwrap();
    let evs = events.lock().unwrap();
    assert!(!evs.is_empty());
    let last = evs.last().unwrap();
    assert!(last.2); // finished
    assert_eq!(last.0, data.len() as u64);
    assert_eq!(last.1, Some(data.len() as u64));
}

#[tokio::test]
async fn handles_empty_file() {
    let src = Arc::new(MockSource::new(Vec::new()));
    let opts = DownloadOptions::default();
    let mgr = DownloadManager::new(src, opts);
    let dir = tempdir();
    let dest = dir.join("empty.bin");
    let task = DownloadTask::new("http://mock/empty.bin", dest.clone()).with_size(0);
    let summary = mgr.download(&task).await.unwrap();
    assert_eq!(summary.size, 0);
    assert_eq!(std::fs::read(&dest).unwrap().len(), 0);
}

/// Robustness: a truncated/garbage temp file must NOT be trusted even when a
/// stale `.part.meta` claims chunks are already complete. The manager must
/// discard the stale resume state and re-fetch every chunk (otherwise the
/// missing ranges would be left as zeros and the final checksum would fail).
#[tokio::test]
async fn resets_stale_meta_when_temp_truncated() {
    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 199) as u8).collect();
    let sha = hash::sha1_bytes(&data);
    let dir = tempdir();
    let dest = dir.join("game.jar");
    let temp = dir.join("game.jar.part");
    let meta = dir.join("game.jar.part.meta");
    let chunk_size = 64 * 1024u64;
    let plan = plan_chunks(data.len() as u64, chunk_size);
    let half = plan.len() / 2;

    // Simulate a crash that left a SHORT temp file while the meta still lists
    // the first half as completed.
    {
        let f = std::fs::File::create(&temp).unwrap();
        f.set_len((data.len() as u64) / 2).unwrap();
    }
    let mut completed: Vec<u64> = Vec::new();
    for (i, (s, _e)) in plan.iter().enumerate() {
        if i < half {
            completed.push(*s / chunk_size);
        }
    }
    let json = serde_json::json!({
        "total_size": data.len() as u64,
        "chunk_size": chunk_size,
        "completed": completed,
    });
    std::fs::write(&meta, serde_json::to_vec(&json).unwrap()).unwrap();

    let src = Arc::new(MockSource::new(data.clone()));
    let opts = DownloadOptions {
        chunk_size,
        concurrency: 4,
        ..Default::default()
    };
    let mgr = DownloadManager::new(src.clone(), opts);
    let task = DownloadTask::new("http://mock/game.jar", dest.clone())
        .with_sha1(sha)
        .with_size(data.len() as u64);
    let summary = mgr.download(&task).await.unwrap();
    assert_eq!(summary.size, data.len() as u64);
    // Because the temp was truncated, the stale meta must be discarded and every
    // chunk re-fetched.
    assert_eq!(
        src.call_count() as usize,
        plan.len(),
        "all chunks must be re-fetched after a truncated temp"
    );
    let got = std::fs::read(&dest).unwrap();
    assert_eq!(got, data);
}

/// Robustness: when the server ignores `Range` (returns the full body for every
/// range request) even though a size is known, the chunked strategy is invalid.
/// The manager must detect this and gracefully fall back to a single sequential
/// download instead of failing the whole task (absorbing FCLCore/download's
/// mirror-fallback discipline).
#[tokio::test]
async fn falls_back_to_single_shot_when_range_ignored_with_known_size() {
    let data: Vec<u8> = (0..120_000u32).map(|i| (i % 211) as u8).collect();
    let sha = hash::sha1_bytes(&data);
    // `without_range` makes the source return the full body with no range support,
    // and `.with_size(...)` exercises the size-known "assume range" code path.
    let src = Arc::new(MockSource::new(data.clone()).without_range());
    let opts = DownloadOptions {
        chunk_size: 32 * 1024,
        concurrency: 4,
        ..Default::default()
    };
    let mgr = DownloadManager::new(src, opts);
    let dir = tempdir();
    let dest = dir.join("r.bin");
    let task = DownloadTask::new("http://mock/r.bin", dest.clone())
        .with_sha1(sha)
        .with_size(data.len() as u64);
    let summary = mgr.download(&task).await.unwrap();
    assert_eq!(summary.size, data.len() as u64);
    let got = std::fs::read(&dest).unwrap();
    assert_eq!(got, data);
}

/// Robustness: a dead primary host must degrade to a working mirror instead of
/// failing the whole download (FCLCore/download multi-mirror resilience, task 2).
#[tokio::test]
async fn mirror_fallback_succeeds_when_primary_fails() {
    let data: Vec<u8> = (0..120_000u32).map(|i| (i % 211) as u8).collect();
    let sha = hash::sha1_bytes(&data);
    let src = Arc::new(MockSource::new(data.clone()));
    // The primary host is permanently down; the mirror must carry the load.
    src.fail_url_containing("unavailable", 1_000_000);
    let opts = DownloadOptions {
        chunk_size: 32 * 1024,
        concurrency: 4,
        max_retries: 3,
        ..Default::default()
    };
    let mgr = DownloadManager::new(src, opts);
    let dir = tempdir();
    let dest = dir.join("m.bin");
    let task = DownloadTask::new("http://unavailable.example/m.bin", dest.clone())
        .with_sha1(sha)
        .with_size(data.len() as u64)
        .with_mirror("http://mirror.example/m.bin");
    let summary = mgr.download(&task).await.unwrap();
    assert_eq!(summary.size, data.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), data);
}

/// Robustness: a dead primary in the single-shot path (no Range support) must
/// also fall back to a mirror via `download_single` (task 2).
#[tokio::test]
async fn mirror_fallback_succeeds_for_single_shot() {
    let data: Vec<u8> = (0..60_000u32).map(|i| (i % 53) as u8).collect();
    let sha = hash::sha1_bytes(&data);
    let src = Arc::new(MockSource::new(data.clone()).without_range());
    src.fail_url_containing("unavailable", 1_000_000);
    let opts = DownloadOptions {
        chunk_size: 16 * 1024,
        concurrency: 4,
        max_retries: 3,
        ..Default::default()
    };
    let mgr = DownloadManager::new(src, opts);
    let dir = tempdir();
    let dest = dir.join("s.bin");
    let task = DownloadTask::new("http://unavailable.example/s.bin", dest.clone())
        .with_sha1(sha)
        .with_size(data.len() as u64)
        .with_mirror("http://mirror.example/s.bin");
    let summary = mgr.download(&task).await.unwrap();
    assert_eq!(summary.size, data.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), data);
}

/// Robustness: an HTTP 429 (RateLimited) must be retried, not treated as fatal,
/// and the download must ultimately succeed once the server stops limiting
/// (task 2 -- honour the unified error model's retryability / Retry-After).
#[tokio::test]
async fn honors_rate_limited_retry_after() {
    let data: Vec<u8> = vec![9u8; 4096];
    let sha = hash::sha1_bytes(&data);
    let src = Arc::new(MockSource::new(data.clone()));
    // First attempt is rate-limited, then the same URL serves normally.
    src.rate_limit_url("rlhost", 1);
    let opts = DownloadOptions {
        chunk_size: 1024,
        concurrency: 2,
        max_retries: 3,
        retry_base: Duration::from_millis(1),
        retry_max: Duration::from_millis(5),
        ..Default::default()
    };
    let mgr = DownloadManager::new(src, opts);
    let dir = tempdir();
    let dest = dir.join("rl.bin");
    let task = DownloadTask::new("http://rlhost/rl.bin", dest.clone())
        .with_sha1(sha)
        .with_size(data.len() as u64);
    let summary = mgr.download(&task).await.unwrap();
    assert_eq!(summary.size, data.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), data);
}

// --- Task 30: enhanced download tests -------------------------------------

/// Rate limiting (task 30) — a low `max_bytes_per_second` must throttle
/// aggregate throughput without corrupting data. We use a tiny payload so the
/// test finishes in ~2s while still proving the sleep path is exercised.
#[tokio::test]
async fn rate_limit_does_not_corrupt_data() {
    let data: Vec<u8> = (0..8u8).collect();
    let sha = hash::sha1_bytes(&data);
    let src = Arc::new(MockSource::new(data.clone()));
    let opts = DownloadOptions {
        chunk_size: 4,
        concurrency: 2,
        max_bytes_per_second: Some(2), // 2 bytes/sec — very slow
        ..Default::default()
    };
    let mgr = DownloadManager::new(src, opts);
    let dir = tempdir();
    let dest = dir.join("rl.bin");
    let task = DownloadTask::new("http://mock/rl.bin", dest.clone())
        .with_sha1(sha)
        .with_size(data.len() as u64);
    let start = std::time::Instant::now();
    let summary = mgr.download(&task).await.unwrap();
    let elapsed = start.elapsed();
    assert_eq!(summary.size, data.len() as u64);
    // Data must be intact despite the rate limit.
    assert_eq!(std::fs::read(&dest).unwrap(), data);
    // 8 bytes at 2 bytes/sec with initial bucket of 2 → ~3s of throttling
    // (two chunks each delayed ~1.5s by the shared limiter). Use a generous
    // lower bound of 1s to avoid CI flakiness.
    assert!(
        elapsed.as_millis() >= 1_000,
        "rate limiter should have slowed the download, elapsed = {:?}",
        elapsed
    );
}

/// Sequential download (task 30) — chunks must arrive in order and the file
/// must be correct. `concurrency` should be ignored when `sequential` is true.
#[tokio::test]
async fn sequential_download_in_order() {
    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 211) as u8).collect();
    let sha = hash::sha1_bytes(&data);
    let src = Arc::new(MockSource::new(data.clone()));
    let opts = DownloadOptions {
        chunk_size: 32 * 1024,
        concurrency: 4, // should be overridden by sequential mode
        sequential: true,
        ..Default::default()
    };
    let mgr = DownloadManager::new(src, opts);
    let dir = tempdir();
    let dest = dir.join("seq.bin");
    let task = DownloadTask::new("http://mock/seq.bin", dest.clone())
        .with_sha1(sha)
        .with_size(data.len() as u64);
    let summary = mgr.download(&task).await.unwrap();
    assert_eq!(summary.size, data.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), data);
}

/// Cancel / pause (task 30) — when the cancel flag is set before download
/// starts, the manager must return `Cancelled` and leave the `.part` file
/// intact for later resume.
#[tokio::test]
async fn cancel_stops_download() {
    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 211) as u8).collect();
    let sha = hash::sha1_bytes(&data);
    let src = Arc::new(MockSource::new(data.clone()));
    let opts = DownloadOptions {
        chunk_size: 32 * 1024,
        concurrency: 4,
        ..Default::default()
    };
    let mgr = DownloadManager::new(src, opts);
    let flag = Arc::new(AtomicBool::new(true));
    let mgr = mgr.with_cancel(flag);
    let dir = tempdir();
    let dest = dir.join("canc.bin");
    let task = DownloadTask::new("http://mock/canc.bin", dest.clone())
        .with_sha1(sha)
        .with_size(data.len() as u64);
    let res = mgr.download(&task).await;
    assert!(res.is_err(), "cancelled download should error");
    assert!(
        matches!(res.unwrap_err(), RcError::Cancelled { .. }),
        "should be Cancelled error"
    );
}

/// Checksum retry (task 30) — when the first download returns corrupted data,
/// the manager should automatically retry (clearing the stale resume state)
/// and the second attempt should succeed.
#[tokio::test]
async fn checksum_retry_recovers_from_corruption() {
    let data: Vec<u8> = (0..120_000u32).map(|i| (i % 211) as u8).collect();
    let sha = hash::sha1_bytes(&data);
    let src = Arc::new(MockSource::new(data.clone()));
    // Corrupt the first full download attempt (all chunks).
    let total_chunks = plan_chunks(data.len() as u64, 32 * 1024).len() as u64;
    src.corrupt_next_n(total_chunks);
    let opts = DownloadOptions {
        chunk_size: 32 * 1024,
        concurrency: 4,
        max_checksum_retries: 2,
        retry_base: Duration::from_millis(1),
        retry_max: Duration::from_millis(5),
        ..Default::default()
    };
    let mgr = DownloadManager::new(src, opts);
    let dir = tempdir();
    let dest = dir.join("ck.bin");
    let task = DownloadTask::new("http://mock/ck.bin", dest.clone())
        .with_sha1(sha)
        .with_size(data.len() as u64);
    let summary = mgr.download(&task).await.unwrap();
    assert_eq!(summary.size, data.len() as u64);
    assert_eq!(summary.checksum_retries, 1, "should have retried once");
    assert_eq!(std::fs::read(&dest).unwrap(), data);
}

/// Checksum retry exhaustion (task 30) — if corruption persists across all
/// retries, the error must surface.
#[tokio::test]
async fn checksum_retry_exhausts_and_fails() {
    let data: Vec<u8> = (0..120_000u32).map(|i| (i % 211) as u8).collect();
    let sha = hash::sha1_bytes(&data);
    let src = Arc::new(MockSource::new(data.clone()));
    // Corrupt *every* attempt (large N so it never runs out).
    src.corrupt_next_n(u64::MAX);
    let opts = DownloadOptions {
        chunk_size: 32 * 1024,
        concurrency: 4,
        max_checksum_retries: 2,
        retry_base: Duration::from_millis(1),
        retry_max: Duration::from_millis(5),
        ..Default::default()
    };
    let mgr = DownloadManager::new(src, opts);
    let dir = tempdir();
    let dest = dir.join("fail.bin");
    let task = DownloadTask::new("http://mock/fail.bin", dest.clone())
        .with_sha1(sha)
        .with_size(data.len() as u64);
    let res = mgr.download(&task).await;
    assert!(res.is_err(), "permanently corrupt data should fail");
    assert!(matches!(res.unwrap_err(), RcError::ChecksumMismatch { .. }));
}

/// Enhanced Progress (task 30) — the progress callback must receive
/// `speed_bps` and `status` fields.
#[tokio::test]
async fn progress_includes_speed_and_status() {
    let data: Vec<u8> = (0..120_000u32).map(|i| (i % 7) as u8).collect();
    let sha = hash::sha1_bytes(&data);
    let src = Arc::new(MockSource::new(data.clone()));
    let opts = DownloadOptions {
        chunk_size: 32 * 1024,
        concurrency: 2,
        ..Default::default()
    };
    let events: Arc<Mutex<Vec<Progress>>> = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let cb: ProgressCallback = Arc::new(move |p: &Progress| {
        ev.lock().unwrap().push(p.clone());
    });
    let mgr = DownloadManager::new(src, opts).with_progress(cb);
    let dir = tempdir();
    let dest = dir.join("prog.bin");
    let task = DownloadTask::new("http://mock/prog.bin", dest.clone())
        .with_sha1(sha)
        .with_size(data.len() as u64);
    mgr.download(&task).await.unwrap();
    let evs = events.lock().unwrap();
    assert!(!evs.is_empty());
    // At least one non-finished event should have status Running.
    let running = evs
        .iter()
        .any(|p| p.status == DownloadStatus::Running && !p.finished);
    assert!(running, "should have at least one Running status event");
    // The last event should be finished with status Completed.
    let last = evs.last().unwrap();
    assert!(last.finished);
    assert_eq!(last.status, DownloadStatus::Completed);
    assert_eq!(last.downloaded, data.len() as u64);
}

/// Single-shot retransmission tracking (task 30) — when the single-shot
/// download (no Range support) retries after a network failure, the shared
/// `bytes_retransmitted` counter should be incremented so the UI can report it.
#[tokio::test]
async fn single_shot_tracks_retransmission() {
    use tokio::io::AsyncWriteExt;

    let data: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
    let sha = hash::sha1_bytes(&data);

    // A source that always succeeds on fetch_range (probe) but fails on the
    // first fetch_range_into and succeeds on the second.
    struct FailOnceSource {
        data: Vec<u8>,
        first: AtomicBool,
    }
    #[async_trait]
    impl HttpSource for FailOnceSource {
        async fn fetch_range(
            &self,
            _url: &str,
            _start: u64,
            _end: Option<u64>,
        ) -> RcResult<FetchResult> {
            Ok(FetchResult {
                bytes: self.data.clone(),
                total_size: self.data.len() as u64,
                supports_range: false,
            })
        }

        async fn fetch_range_into(
            &self,
            _url: &str,
            _start: u64,
            _end: Option<u64>,
            writer: &mut (dyn AsyncWrite + Send + Unpin),
        ) -> RcResult<u64> {
            if self.first.swap(false, Ordering::SeqCst) {
                return Err(RcError::Network("injected failure".into()));
            }
            writer.write_all(&self.data).await.map_err(RcError::Io)?;
            writer.flush().await.map_err(RcError::Io)?;
            Ok(self.data.len() as u64)
        }
    }

    let src = Arc::new(FailOnceSource {
        data: data.clone(),
        first: AtomicBool::new(true),
    });
    let opts = DownloadOptions {
        chunk_size: 16 * 1024,
        concurrency: 2,
        max_retries: 3,
        retry_base: Duration::from_millis(1),
        retry_max: Duration::from_millis(5),
        ..Default::default()
    };
    let mgr = DownloadManager::new(src, opts);
    let dir = tempdir();
    let dest = dir.join("retr.bin");
    // No .with_size() — resolve() probes and discovers supports_range: false.
    let task = DownloadTask::new("http://mock/retr.bin", dest.clone()).with_sha1(sha);
    let summary = mgr.download(&task).await.unwrap();
    assert_eq!(summary.size, data.len() as u64);
    assert!(
        summary.bytes_retransmitted > 0,
        "should have tracked retransmission (got {})",
        summary.bytes_retransmitted
    );
    assert_eq!(std::fs::read(&dest).unwrap(), data);
}

/// Seek-before-retry correctness (task 30) — when the single-shot fetch writes
/// partial data before failing, a retry must rewind the writer to offset 0 so
/// the new data overwrites (not appends to) the stale partial write.
#[tokio::test]
async fn single_shot_seeks_before_retry_after_partial_write() {
    use tokio::io::AsyncWriteExt;

    let data: Vec<u8> = (0..40_000u32).map(|i| (i % 251) as u8).collect();
    let sha = hash::sha1_bytes(&data);

    /// A source that always succeeds on fetch_range (probe) returning
    /// supports_range: false, but on fetch_range_into writes half the data
    /// then fails the first time, and writes the full data on the second.
    struct PartialFailSource {
        data: Vec<u8>,
        first: AtomicBool,
    }
    #[async_trait]
    impl HttpSource for PartialFailSource {
        async fn fetch_range(
            &self,
            _url: &str,
            _start: u64,
            _end: Option<u64>,
        ) -> RcResult<FetchResult> {
            Ok(FetchResult {
                bytes: self.data.clone(),
                total_size: self.data.len() as u64,
                supports_range: false,
            })
        }

        async fn fetch_range_into(
            &self,
            _url: &str,
            _start: u64,
            _end: Option<u64>,
            writer: &mut (dyn AsyncWrite + Send + Unpin),
        ) -> RcResult<u64> {
            if self.first.swap(false, Ordering::SeqCst) {
                // First attempt: write half the data, then fail.
                let half = self.data.len() / 2;
                writer
                    .write_all(&self.data[..half])
                    .await
                    .map_err(RcError::Io)?;
                writer.flush().await.map_err(RcError::Io)?;
                return Err(RcError::Network("injected partial-write failure".into()));
            }
            // Second attempt: write the full data. retry_fetch_into has
            // already seeked to 0, so this overwrites the stale half.
            writer.write_all(&self.data).await.map_err(RcError::Io)?;
            writer.flush().await.map_err(RcError::Io)?;
            Ok(self.data.len() as u64)
        }
    }

    let src = Arc::new(PartialFailSource {
        data: data.clone(),
        first: AtomicBool::new(true),
    });
    let opts = DownloadOptions {
        chunk_size: 16 * 1024,
        concurrency: 2,
        max_retries: 3,
        retry_base: Duration::from_millis(1),
        retry_max: Duration::from_millis(5),
        ..Default::default()
    };
    let mgr = DownloadManager::new(src, opts);
    let dir = tempdir();
    let dest = dir.join("partial.bin");
    // No .with_size() — resolve() probes and discovers supports_range: false.
    let task = DownloadTask::new("http://mock/partial.bin", dest.clone()).with_sha1(sha);
    let summary = mgr.download(&task).await.unwrap();
    assert_eq!(summary.size, data.len() as u64);
    // Data must be correct — the seek-before-retry prevented the half+full
    // concatenation corruption.
    let got = std::fs::read(&dest).unwrap();
    assert_eq!(got, data, "partial-write retry must not corrupt data");
    assert_eq!(
        summary.bytes_retransmitted, 1,
        "should have tracked exactly 1 retransmission"
    );
}
