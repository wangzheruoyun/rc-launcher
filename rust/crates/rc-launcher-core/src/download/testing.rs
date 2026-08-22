//! Offline tests for the download manager.
//!
//! These run on the host in CI (no network) against an in-memory
//! [`MockSource`] that honours `Range` requests, so the full chunked /
//! resumable / verified download path is exercised deterministically.

use std::collections::HashMap;
use std::io::{Seek, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::download::client::{FetchResult, HttpSource};
use crate::download::hash;
use crate::download::manager::plan_chunks;
use crate::download::{DownloadManager, DownloadOptions, DownloadTask, Progress, ProgressCallback};
use crate::error::{RcError, RcResult};

/// A configurable in-memory resource that implements [`HttpSource`].
pub struct MockSource {
    data: Vec<u8>,
    supports_range: bool,
    /// Map of start-offset -> number of times to fail before succeeding.
    fail_map: Arc<Mutex<HashMap<u64, usize>>>,
    /// Number of `fetch_range` calls made (used for resume assertions).
    calls: Arc<AtomicU64>,
}

impl MockSource {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            supports_range: true,
            fail_map: Arc::new(Mutex::new(HashMap::new())),
            calls: Arc::new(AtomicU64::new(0)),
        }
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

    pub fn call_count(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl HttpSource for MockSource {
    async fn fetch_range(&self, _url: &str, start: u64, end: Option<u64>) -> RcResult<FetchResult> {
        self.calls.fetch_add(1, Ordering::Relaxed);
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
        let bytes = self.data[start as usize..=end as usize].to_vec();
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
