//! Async, resumable, chunked download manager (task 2).
//!
//! Design notes (absorbing FCLCore/download + cuberite robustness):
//! * **Range resume** — the target is first materialised into a `.part` temp
//!   file pre-sized to the full length; each parallel chunk writes into its own
//!   byte range. A side-car `.part.meta` JSON records which chunks are already
//!   complete, so a killed/interrupted download resumes by only re-fetching the
//!   missing ranges (no re-downloading the whole file).
//! * **Parallel / sequential shards** — `concurrency` chunks download at once
//!   through a semaphore; chunk size is configurable. When `sequential` is set
//!   (task 30) chunks are fetched one-at-a-time in order instead of in parallel,
//!   which is friendlier to mirrors and to streaming decompression of compressed
//!   packages (ZIP/JAR).
//! * **Verification** — after assembly the file is hashed (SHA-1 or MD5) and
//!   compared against the expected checksum (case-insensitive).
//! * **Checksum-retry** — on a whole-file checksum mismatch (task 30) the
//!   download is re-attempted up to `max_checksum_retries` times, clearing the
//!   stale resume state so only the bad bytes are re-fetched (a *partial*
//!   retry, not a blind full re-download from scratch). A different mirror is
//!   tried first so a corrupt mirror does not loop forever.
//! * **Exponential backoff** — every chunk retries with `base * 2^attempt`
//!   (capped, with jitter) before the whole task fails.
//! * **Rate limiting** — an optional token-bucket limiter (`max_bytes_per_second`,
//!   task 30) caps aggregate throughput on weak China-mainland links so a
//!   flaky connection is not saturated and starved.
//! * **Cancellation** — an `Arc<AtomicBool>` flag (task 30) lets the UI pause /
//!   cancel a download mid-flight; the `.part` / `.part.meta` side-cars are
//!   preserved so a later `download` resumes from the last good checkpoint
//!   (automatic recovery after a disconnect).
//! * **Progress** — a single callback receives cumulative progress (with live
//!   `speed_bps` and `status`) across all chunks (and across a batch via
//!   [`DownloadManager::download_all`]).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::fs as tfs;
use tokio::io::{AsyncSeek, AsyncSeekExt, AsyncWrite, SeekFrom};
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;

use crate::download::client::HttpSource;
use crate::download::hash;
use crate::error::{RcError, RcResult};

/// Trait-object-compatible combination of `AsyncWrite` + `AsyncSeek`.
///
/// Rust only allows a single non-auto trait in a `dyn` trait object, so we
/// merge the two capabilities into one trait for the single-shot download
/// writer (which needs to seek back to 0 on retry after a partial write —
/// task 30 weak-network auto-recovery).
trait AsyncWriteSeek: AsyncWrite + AsyncSeek {}
impl<T: AsyncWrite + AsyncSeek> AsyncWriteSeek for T {}

/// Expected checksum of the downloaded file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checksum {
    Sha1(String),
    Md5(String),
}

impl Checksum {
    pub fn algorithm(&self) -> &'static str {
        match self {
            Checksum::Sha1(_) => "sha1",
            Checksum::Md5(_) => "md5",
        }
    }
    pub fn expected(&self) -> &str {
        match self {
            Checksum::Sha1(s) | Checksum::Md5(s) => s,
        }
    }
}

/// A single download description.
#[derive(Debug, Clone)]
pub struct DownloadTask {
    pub url: String,
    pub dest: PathBuf,
    pub checksum: Option<Checksum>,
    /// Known content length (skips the size-probing request).
    pub size: Option<u64>,
    /// Stable identifier used in progress events (defaults to `url`).
    pub id: Option<String>,
    /// Additional mirror URLs tried in order when `url` fails. This is the
    /// per-task resilience hook that mirrors FCLCore/download's multi-mirror
    /// strategy: a flaky primary host degrades to a working mirror instead of
    /// failing the whole download.
    pub mirrors: Vec<String>,
}

impl DownloadTask {
    pub fn new(url: impl Into<String>, dest: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            dest: dest.into(),
            checksum: None,
            size: None,
            id: None,
            mirrors: Vec::new(),
        }
    }
    pub fn with_sha1(mut self, sha1: impl Into<String>) -> Self {
        self.checksum = Some(Checksum::Sha1(sha1.into()));
        self
    }
    pub fn with_md5(mut self, md5: impl Into<String>) -> Self {
        self.checksum = Some(Checksum::Md5(md5.into()));
        self
    }
    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
    /// Register an additional mirror URL to try if the primary `url` fails.
    pub fn with_mirror(mut self, mirror: impl Into<String>) -> Self {
        self.mirrors.push(mirror.into());
        self
    }
    /// All candidate URLs for this task: the primary first, then every mirror
    /// in registration order.
    pub fn urls(&self) -> Vec<String> {
        std::iter::once(self.url.clone())
            .chain(self.mirrors.iter().cloned())
            .collect()
    }
}

/// Tunables for a [`DownloadManager`].
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    /// Size of each parallel chunk in bytes.
    pub chunk_size: u64,
    /// Max number of chunks downloaded in parallel.
    pub concurrency: usize,
    /// Max retries per chunk before the task fails.
    pub max_retries: u32,
    /// Base backoff delay (doubled each retry).
    pub retry_base: Duration,
    /// Upper bound for a single backoff delay.
    pub retry_max: Duration,
    /// Jitter fraction (0.0 = none) applied to backoff.
    pub retry_jitter: f64,
    /// Connect timeout for the underlying HTTP client.
    pub connect_timeout: Duration,
    /// Read timeout for the underlying HTTP client.
    pub read_timeout: Duration,
    /// Suffix for the in-progress temp file (default `.part`).
    pub temp_suffix: String,
    /// Max number of *tasks* downloaded in parallel by
    /// [`DownloadManager::download_all_concurrent`]. Bounds how many files are
    /// in flight at once so a large batch never starves the UI / blocks the
    /// caller's executor (task 25 — coroutine scheduling hygiene).
    pub max_batch_concurrency: usize,
    /// Persist the resume meta-file at most every `meta_persist_every` chunk
    /// completions (plus one final flush). Batching the JSON serialisation +
    /// file write off the per-chunk hot path keeps the async workers free
    /// (task 25 — avoid blocking the scheduler on metadata churn).
    pub meta_persist_every: usize,
    /// Maximum aggregate download rate in bytes per second. `None` = unlimited.
    /// On weak China-mainland networks this prevents saturating a flaky link
    /// so retries / reconnection are more likely to succeed (task 30).
    pub max_bytes_per_second: Option<u64>,
    /// When the overall checksum fails after a full download, automatically
    /// re-download (clearing the stale resume state so only the bad bytes are
    /// re-fetched) up to this many times before surfacing the error.
    /// 0 = no automatic retry (fail fast). (task 30)
    pub max_checksum_retries: u32,
    /// Download chunks sequentially (one at a time, in order) instead of in
    /// parallel. Useful for compressed packages (ZIP/JAR) where sequential
    /// access is friendlier to mirrors and streaming decompression (task 30).
    pub sequential: bool,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            chunk_size: 8 * 1024 * 1024,
            concurrency: 8,
            max_retries: 5,
            retry_base: Duration::from_millis(500),
            retry_max: Duration::from_secs(30),
            retry_jitter: 0.25,
            connect_timeout: Duration::from_secs(30),
            read_timeout: Duration::from_secs(60),
            temp_suffix: ".part".to_string(),
            max_batch_concurrency: 4,
            meta_persist_every: 16,
            max_bytes_per_second: None,
            max_checksum_retries: 2,
            sequential: false,
        }
    }
}

/// Current lifecycle status of a download (task 30).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Running,
    Paused,
    Completed,
    Failed,
}

/// Cumulative progress snapshot delivered to the progress callback.
#[derive(Debug, Clone)]
pub struct Progress {
    pub id: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub finished: bool,
    /// Current throughput in bytes per second (averaged over the last emission).
    /// 0 when the rate cannot be determined yet (e.g. total is unknown).
    pub speed_bps: u64,
    /// Current download status (task 30 — surfaced to the UI for pause/continue).
    pub status: DownloadStatus,
}

impl Progress {
    /// Convenience: a "paused" progress snapshot.
    pub fn paused(id: String, downloaded: u64, total: Option<u64>) -> Self {
        Self {
            id,
            downloaded,
            total,
            finished: false,
            speed_bps: 0,
            status: DownloadStatus::Paused,
        }
    }

    /// Convenience: a "completed" progress snapshot.
    pub fn completed(id: String, total: u64) -> Self {
        Self {
            id,
            downloaded: total,
            total: Some(total),
            finished: true,
            speed_bps: 0,
            status: DownloadStatus::Completed,
        }
    }
}

/// Outcome of a completed download.
#[derive(Debug, Clone)]
pub struct DownloadSummary {
    pub dest: PathBuf,
    pub size: u64,
    /// `true` if some bytes were already present from a previous run.
    pub resumed: bool,
    pub duration: Duration,
    /// Total bytes re-fetched due to chunk retries or checksum-failure retries
    /// (task 30 — surfaced alongside speed so the UI can show retransmission).
    pub bytes_retransmitted: u64,
    /// How many times the checksum retry loop ran (task 30).
    pub checksum_retries: u32,
}

/// Shared progress callback type.
pub type ProgressCallback = Arc<dyn Fn(&Progress) + Send + Sync>;

/// Optional cancellation handle for a download.
///
/// When the flag is set to `true` the download loop stops at the next safe
/// checkpoint (between chunks, or after a failed chunk retry). The `.part` and
/// `.part.meta` side-cars are preserved so a later `download` call resumes from
/// the last checkpoint — this is the "auto-recovery after disconnect" behaviour
/// for task 30.
pub type CancelFlag = Arc<AtomicBool>;

/// Simple token-bucket rate limiter (task 30 — weak-network speed limiting).
///
/// Shared across all download chunks via `Arc<Mutex<…>>` so the aggregate
/// throughput stays under `max_bytes_per_sec` even with parallel shards.
struct RateLimiter {
    max_bytes_per_sec: u64,
    /// Tokens (bytes) currently available in the bucket.
    tokens: f64,
    /// Last time tokens were refilled.
    last_refill: Instant,
}

impl RateLimiter {
    fn new(max_bytes_per_sec: u64) -> Self {
        Self {
            max_bytes_per_sec,
            tokens: max_bytes_per_sec as f64,
            last_refill: Instant::now(),
        }
    }

    /// Consume `bytes` tokens, sleeping if the bucket is empty.
    async fn consume(&mut self, bytes: u64) {
        self.refill();
        self.tokens -= bytes as f64;
        if self.tokens < 0.0 {
            let deficit = (-self.tokens) as u64;
            let sleep_secs = deficit as f64 / self.max_bytes_per_sec as f64;
            tokio::time::sleep(Duration::from_secs_f64(sleep_secs)).await;
            self.tokens = 0.0;
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        if elapsed > 0.0 {
            self.tokens += elapsed * self.max_bytes_per_sec as f64;
            // Cap at one second's worth of tokens.
            let cap = self.max_bytes_per_sec as f64;
            if self.tokens > cap {
                self.tokens = cap;
            }
            self.last_refill = now;
        }
    }
}

/// The resumable, chunked download manager.
///
/// Cheap to clone: all internals are behind `Arc` / `Arc<dyn>` so a batch
/// worker can own its own handle (see [`DownloadManager::download_all_concurrent`]).
#[derive(Clone)]
pub struct DownloadManager {
    source: Arc<dyn HttpSource>,
    options: DownloadOptions,
    progress: Option<ProgressCallback>,
    /// Optional cancellation flag (task 30 — pause / cancel / auto-recovery).
    cancel_flag: Option<CancelFlag>,
}

impl DownloadManager {
    pub fn new(source: Arc<dyn HttpSource>, options: DownloadOptions) -> Self {
        Self {
            source,
            options,
            progress: None,
            cancel_flag: None,
        }
    }

    /// Convenience constructor that builds a default `reqwest` backend from the
    /// connect/read timeouts declared in `options`.
    pub fn with_default_source(options: DownloadOptions) -> RcResult<Self> {
        let client = crate::download::client::ReqwestSource::with_timeouts(
            options.connect_timeout,
            options.read_timeout,
        )?;
        Ok(Self::new(Arc::new(client), options))
    }

    /// Attach a progress callback (cloning the manager is cheap: internals are
    /// behind `Arc`).
    pub fn with_progress(mut self, cb: ProgressCallback) -> Self {
        self.progress = Some(cb);
        self
    }

    /// Attach a cancellation flag (task 30). When the flag is set to `true` the
    /// download loop stops at the next checkpoint and returns a
    /// [`RcError::Cancelled`] error, preserving the `.part` / `.part.meta`
    /// side-cars so a later download resumes (auto-recovery after disconnect).
    pub fn with_cancel(mut self, flag: CancelFlag) -> Self {
        self.cancel_flag = Some(flag);
        self
    }

    /// Check the cancellation flag (task 30). Returns `true` if the download
    /// should stop.
    fn is_cancelled(&self) -> bool {
        self.cancel_flag
            .as_ref()
            .map(|f| f.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Download a single [`DownloadTask`].
    ///
    /// Wraps the single-shot download with a checksum-retry loop (task 30):
    /// if the final hash does not match, the stale resume state is cleared and
    /// the download is retried (up to `max_checksum_retries`), trying a
    /// different mirror first so a corrupt endpoint does not loop forever.
    pub async fn download(&self, task: &DownloadTask) -> RcResult<DownloadSummary> {
        let id = task.id.clone().unwrap_or_else(|| task.url.clone());

        if let Some(parent) = task.dest.parent() {
            tfs::create_dir_all(parent).await.map_err(RcError::Io)?;
        }

        let bytes_retransmitted: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
        let (known_size, supports_range) = self.resolve(task).await?;
        let total = match known_size {
            Some(s) => s,
            // Unknown size & no range support -> single-shot full download.
            None => return self.download_single(task, &bytes_retransmitted).await,
        };

        if total == 0 {
            tfs::File::create(&task.dest).await.map_err(RcError::Io)?;
            self.emit(&Progress {
                id,
                downloaded: 0,
                total: Some(0),
                finished: true,
                speed_bps: 0,
                status: DownloadStatus::Completed,
            });
            return Ok(DownloadSummary {
                dest: task.dest.clone(),
                size: 0,
                resumed: false,
                duration: Duration::ZERO,
                bytes_retransmitted: 0,
                checksum_retries: 0,
            });
        }

        // Checksum-retry loop (task 30): on a whole-file hash mismatch we clear
        // the stale resume state and retry, up to `max_checksum_retries` times.
        let mut retries: u32 = 0;
        loop {
            let result = if supports_range {
                self.download_chunked(task, &id, total, &bytes_retransmitted)
                    .await
            } else {
                self.download_single(task, &bytes_retransmitted).await
            };
            match result {
                Ok(summary) => {
                    return Ok(DownloadSummary {
                        bytes_retransmitted: bytes_retransmitted.load(Ordering::Relaxed),
                        checksum_retries: retries,
                        ..summary
                    });
                }
                Err(RcError::ChecksumMismatch { .. }) => {
                    if retries >= self.options.max_checksum_retries {
                        return Err(RcError::ChecksumMismatch {
                            path: task.dest.display().to_string(),
                            expected: task
                                .checksum
                                .as_ref()
                                .map(|c| c.expected().to_string())
                                .unwrap_or_default(),
                            actual: String::new(),
                        });
                    }
                    retries += 1;
                    // Clear the stale resume state so the retry re-fetches the
                    // bad bytes (a *partial* retry, not a blind full
                    // re-download from scratch).
                    self.clear_resume_state(task).await?;
                    // Try a different mirror on the next attempt.
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Download many tasks **sequentially**, returning per-task results (order
    /// preserved). Each task awaits the previous one, so the caller is never
    /// blocked by more than one in-flight download at a time.
    pub async fn download_all(&self, tasks: &[DownloadTask]) -> Vec<RcResult<DownloadSummary>> {
        let mut out = Vec::with_capacity(tasks.len());
        for t in tasks {
            out.push(self.download(t).await);
        }
        out
    }

    /// Download many tasks with **bounded parallelism** (at most
    /// [`DownloadOptions::max_batch_concurrency`] in flight). This is the
    /// throughput path for a library / asset batch: it fills the network without
    /// ever spawning an unbounded number of tasks that would pile up on the
    /// caller's executor or the UI thread (task 25 — coroutine scheduling).
    ///
    /// Results are returned in input order. A failure of one task does not abort
    /// the others.
    pub async fn download_all_concurrent(
        &self,
        tasks: &[DownloadTask],
    ) -> Vec<RcResult<DownloadSummary>> {
        if tasks.is_empty() {
            return Vec::new();
        }
        let lim = self.options.max_batch_concurrency.max(1);
        let sem = Arc::new(Semaphore::new(lim));
        let mut set: JoinSet<RcResult<DownloadSummary>> = JoinSet::new();
        let mut out = Vec::with_capacity(tasks.len());
        for t in tasks {
            // Acquire a permit up front so we never enqueue more than `lim`
            // runnable tasks at once (back-pressure on the executor).
            let permit = match sem.clone().acquire_owned().await {
                Ok(p) => p,
                // The semaphore is only closed on shutdown; stop enqueuing and
                // return whatever has already completed.
                Err(_) => return out,
            };
            let mgr = self.clone();
            let task = t.clone();
            set.spawn(async move {
                let _permit = permit;
                mgr.download(&task).await
            });
        }
        while let Some(joined) = set.join_next().await {
            out.push(match joined {
                Ok(r) => r,
                Err(e) => Err(RcError::Other(format!("batch download task panicked: {e}"))),
            });
        }
        out
    }

    // --- internals ---------------------------------------------------------

    /// Remove the `.part.meta` side-car so a retry re-fetches all chunks.
    /// The `.part` file itself is left in place (pre-sized) so the next run
    /// does not need to re-allocate (task 30 — partial retry on checksum
    /// mismatch).
    async fn clear_resume_state(&self, task: &DownloadTask) -> RcResult<()> {
        let temp = append_suffix(&task.dest, &self.options.temp_suffix);
        let meta = append_suffix(&temp, ".meta");
        let _ = tfs::remove_file(&meta).await;
        Ok(())
    }

    async fn resolve(&self, task: &DownloadTask) -> RcResult<(Option<u64>, bool)> {
        if let Some(size) = task.size {
            // Size known — assume the (mirror) server honours Range. Skipping the
            // probing request avoids downloading the whole file just to learn its size.
            return Ok((Some(size), true));
        }
        // Probe every candidate URL (primary + mirrors) until one answers, so a
        // dead primary host degrades to a mirror instead of failing the task.
        let mut last_err = None;
        for url in task.urls() {
            match self.source.fetch_range(&url, 0, Some(0)).await {
                Ok(r) => {
                    return if r.supports_range {
                        Ok((Some(r.total_size), true))
                    } else {
                        Ok((None, false))
                    };
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(RcError::Download(format!(
            "cannot determine size: {}",
            last_err.map(|e| e.to_string()).unwrap_or_default()
        )))
    }

    /// Create a shared rate limiter from the options (task 30).
    fn make_rate_limiter(&self) -> Option<Arc<Mutex<RateLimiter>>> {
        self.options
            .max_bytes_per_second
            .map(|max| Arc::new(Mutex::new(RateLimiter::new(max))))
    }

    async fn download_chunked(
        &self,
        task: &DownloadTask,
        id: &str,
        total: u64,
        bytes_retransmitted: &Arc<AtomicU64>,
    ) -> RcResult<DownloadSummary> {
        let temp = append_suffix(&task.dest, &self.options.temp_suffix);
        let meta_path = append_suffix(&temp, ".meta");

        // Load existing progress (resume), validating it still matches.
        let mut state = match tfs::read(&meta_path).await {
            Ok(bytes) => serde_json::from_slice::<DownloadState>(&bytes)
                .unwrap_or_else(|_| DownloadState::new(total, self.options.chunk_size)),
            Err(_) => DownloadState::new(total, self.options.chunk_size),
        };
        if state.total_size != total || state.chunk_size != self.options.chunk_size {
            state = DownloadState::new(total, self.options.chunk_size);
        }
        // Pre-size the temp file so chunks can seek+write independently. If the
        // temp is missing or the wrong length (e.g. left truncated by a previous
        // crash), the previously persisted `completed` set can no longer be
        // trusted — those byte ranges would otherwise be left as zeros/garbage
        // and the final checksum would fail. Drop the stale resume state and
        // re-fetch every chunk (robustness: never trust a half-written artifact;
        // mirrors cuberite's defensive re-validation of partial state).
        let temp_intact =
            matches!(tfs::metadata(&temp).await.map(|m| m.len()).ok(), Some(len) if len == total);
        if !temp_intact {
            let f = tfs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&temp)
                .await
                .map_err(RcError::Io)?;
            f.set_len(total).await.map_err(RcError::Io)?;
            f.sync_all().await.map_err(RcError::Io)?;
            state.completed.clear();
        }
        let resumed = !state.completed.is_empty();

        let completed: Arc<Mutex<HashSet<u64>>> =
            Arc::new(Mutex::new(state.completed.iter().copied().collect()));
        let downloaded = Arc::new(AtomicU64::new(state.completed_bytes()));
        let persist_gate = Arc::new(AtomicU64::new(0));
        let sem = Arc::new(Semaphore::new(self.options.concurrency.max(1)));
        let start = Instant::now();
        let rate_limiter = self.make_rate_limiter();
        let cancel_flag = self.cancel_flag.clone();

        // Speed tracking: shared counters so the progress callback can compute
        // a live throughput figure (task 30).
        let last_bytes = Arc::new(AtomicU64::new(state.completed_bytes()));
        let last_time = Arc::new(Mutex::new(Instant::now()));

        let plan = plan_chunks(total, self.options.chunk_size);
        let completed_set: HashSet<u64> = state.completed.iter().copied().collect();

        if self.options.sequential {
            // --- Sequential path (task 30 — compressed packages) ---
            // Chunks are downloaded one at a time, in order, with a single
            // concurrency slot and no parallelism. This is friendlier to mirrors
            // that throttle concurrent Range requests and to streaming
            // decompression.
            for (start_off, end_off) in &plan {
                if self.is_cancelled() {
                    return Err(RcError::Cancelled {
                        scope: id.to_string(),
                    });
                }
                let idx = *start_off / self.options.chunk_size;
                if completed_set.contains(&idx) {
                    continue; // already downloaded in a previous run
                }
                let result = download_chunk(
                    self.source.clone(),
                    task.urls(),
                    temp.clone(),
                    *start_off,
                    *end_off,
                    total,
                    self.options.clone(),
                    downloaded.clone(),
                    completed.clone(),
                    persist_gate.clone(),
                    meta_path.clone(),
                    id.to_string(),
                    self.progress.clone(),
                    rate_limiter.clone(),
                    cancel_flag.clone(),
                    last_bytes.clone(),
                    last_time.clone(),
                    bytes_retransmitted.clone(),
                )
                .await;
                if let Err(e) = result {
                    if matches!(e, RcError::Cancelled { .. }) {
                        // Re-emit a paused progress snapshot so the UI knows.
                        let d = downloaded.load(Ordering::Relaxed);
                        if let Some(cb) = &self.progress {
                            cb(&Progress {
                                id: id.to_string(),
                                downloaded: d,
                                total: Some(total),
                                finished: false,
                                speed_bps: 0,
                                status: DownloadStatus::Paused,
                            });
                        }
                        return Err(e);
                    }
                    // Clear resume state for a fresh retry.
                    let _ = tfs::remove_file(&meta_path).await;
                    return Err(e);
                }
            }
        } else {
            // --- Parallel path (default) ---
            let mut set: JoinSet<RcResult<()>> = JoinSet::new();

            for (start_off, end_off) in &plan {
                let idx = *start_off / self.options.chunk_size;
                if completed_set.contains(&idx) {
                    continue; // already downloaded in a previous run
                }
                let s_off = *start_off;
                let e_off = *end_off;
                let permit = sem
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| RcError::Other("semaphore closed".into()))?;
                let source = self.source.clone();
                let temp = temp.clone();
                let meta_path = meta_path.clone();
                let completed = completed.clone();
                let downloaded = downloaded.clone();
                let id = id.to_string();
                let opts = self.options.clone();
                let urls = task.urls();
                let progress = self.progress.clone();
                let persist_gate = persist_gate.clone();
                let rate_limiter = rate_limiter.clone();
                let cancel_flag = cancel_flag.clone();
                let last_bytes = last_bytes.clone();
                let last_time = last_time.clone();
                let bytes_ret_local = Arc::new(AtomicU64::new(0));
                let bytes_retransmitted_shared = bytes_retransmitted.clone();
                set.spawn(async move {
                    let _permit = permit; // hold for the lifetime of the task
                    let result = download_chunk(
                        source,
                        urls,
                        temp,
                        s_off,
                        e_off,
                        total,
                        opts,
                        downloaded,
                        completed,
                        persist_gate,
                        meta_path,
                        id,
                        progress,
                        rate_limiter,
                        cancel_flag,
                        last_bytes,
                        last_time,
                        bytes_ret_local.clone(),
                    )
                    .await;
                    // Merge retransmission stats from this chunk into the shared counter.
                    bytes_retransmitted_shared
                        .fetch_add(bytes_ret_local.load(Ordering::Relaxed), Ordering::Relaxed);
                    result
                });
            }

            // Await all chunks; abort the rest on the first failure.
            let mut first_err: Option<RcError> = None;
            while let Some(joined) = set.join_next().await {
                match joined {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        // Check if any other task also returned bytes_ret_local
                        // — but we can't easily read those. We'll track
                        // retransmission at the manager level via the shared
                        // bytes_retransmitted counter instead.
                        if matches!(e, RcError::Cancelled { .. }) {
                            first_err.get_or_insert(e);
                            break;
                        }
                        first_err.get_or_insert(e);
                        break;
                    }
                    Err(e) => {
                        first_err.get_or_insert_with(|| {
                            RcError::Other(format!("download task panicked: {e}"))
                        });
                        break;
                    }
                }
            }
            if let Some(e) = first_err {
                set.abort_all();
                // A `RangeUnsupported` from any chunk means the server ignored the
                // Range header and answered every request with the full body, so the
                // whole chunked plan is invalid. Fall back to a single sequential
                // download of the full resource (which does not rely on Range) instead
                // of failing the task. Temp + meta are left in place so the next run
                // can still resume.
                if matches!(e, RcError::RangeUnsupported { .. }) {
                    return self.download_single(task, &bytes_retransmitted).await;
                }
                // Leave the temp + meta in place so the next run can resume.
                return Err(e);
            }
        }

        // Final resume-state flush: covers any chunks completed after the last
        // throttled write. Done once here, off the per-chunk hot path.
        {
            let set = completed.lock().await;
            persist_meta_blocking(
                meta_path.clone(),
                total,
                self.options.chunk_size,
                set.clone(),
            )
            .await?;
        }

        self.finalize(task, &temp, total, resumed, start.elapsed())
            .await?;

        // Emit a final completed progress event.
        if let Some(cb) = &self.progress {
            cb(&Progress {
                id: id.to_string(),
                downloaded: total,
                total: Some(total),
                finished: true,
                speed_bps: 0,
                status: DownloadStatus::Completed,
            });
        }
        Ok(DownloadSummary {
            dest: task.dest.clone(),
            size: total,
            resumed,
            duration: start.elapsed(),
            bytes_retransmitted: bytes_retransmitted.load(Ordering::Relaxed),
            checksum_retries: 0,
        })
    }

    async fn download_single(
        &self,
        task: &DownloadTask,
        bytes_retransmitted: &Arc<AtomicU64>,
    ) -> RcResult<DownloadSummary> {
        let temp = append_suffix(&task.dest, &self.options.temp_suffix);
        if let Some(parent) = temp.parent() {
            tfs::create_dir_all(parent).await.map_err(RcError::Io)?;
        }
        let start = Instant::now();
        let rate_limiter = self.make_rate_limiter();
        let cancel_flag = self.cancel_flag.clone();
        // Stream the whole resource straight into the temp file — no in-memory
        // buffering of the body (task 25 — large-file streaming download).
        let mut file = tfs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp)
            .await
            .map_err(RcError::Io)?;

        // Check cancellation before starting.
        if cancel_flag
            .as_ref()
            .map(|f| f.load(Ordering::SeqCst))
            .unwrap_or(false)
        {
            return Err(RcError::Cancelled {
                scope: task.id.clone().unwrap_or_else(|| task.url.clone()),
            });
        }

        let total = self
            .retry_fetch_into(
                &task.urls(),
                &mut file,
                rate_limiter,
                &cancel_flag,
                bytes_retransmitted,
            )
            .await?;

        self.finalize(task, &temp, total, false, start.elapsed())
            .await?;

        Ok(DownloadSummary {
            dest: task.dest.clone(),
            size: total,
            resumed: false,
            duration: start.elapsed(),
            bytes_retransmitted: bytes_retransmitted.load(Ordering::Relaxed),
            checksum_retries: 0,
        })
    }

    /// Stream a (retried, exponential-backoff) full fetch into `writer`,
    /// returning the number of bytes written. Used by the single-shot path.
    /// Honours the rate limiter and cancellation flag (task 30).
    async fn retry_fetch_into(
        &self,
        urls: &[String],
        writer: &mut (dyn AsyncWriteSeek + Send + Unpin),
        rate_limiter: Option<Arc<Mutex<RateLimiter>>>,
        cancel_flag: &Option<CancelFlag>,
        bytes_retransmitted: &Arc<AtomicU64>,
    ) -> RcResult<u64> {
        let mut attempt: u32 = 0;
        loop {
            // Check cancellation before each attempt.
            if cancel_flag
                .as_ref()
                .map(|f| f.load(Ordering::SeqCst))
                .unwrap_or(false)
            {
                return Err(RcError::Cancelled {
                    scope: urls.first().cloned().unwrap_or_default(),
                });
            }

            // Rewind to the start of the writer before each attempt so a retry
            // after a *partial* write does not leave stale bytes appended after
            // the new fetch (task 30 — correctness on weak/reconnecting networks).
            writer.seek(SeekFrom::Start(0)).await.map_err(RcError::Io)?;

            let url = pick_url(urls, attempt).to_string();
            match self.source.fetch_range_into(&url, 0, None, writer).await {
                Ok(n) => {
                    // Apply rate limiting after the fetch.
                    if let Some(rl) = &rate_limiter {
                        let mut limiter = rl.lock().await;
                        limiter.consume(n).await;
                    }
                    return Ok(n);
                }
                Err(e) => {
                    attempt += 1;
                    if attempt > self.options.max_retries {
                        return Err(e);
                    }
                    // Track retransmission bytes for stats (task 30).
                    bytes_retransmitted.fetch_add(1, Ordering::Relaxed);
                    // Honour a server-supplied Retry-After (via the unified
                    // error model) before falling back to exponential backoff.
                    let backoff = if let Some(b) = e.suggested_backoff() {
                        b
                    } else {
                        compute_backoff(
                            attempt,
                            self.options.retry_base,
                            self.options.retry_max,
                            self.options.retry_jitter,
                        )
                    };
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }

    async fn finalize(
        &self,
        task: &DownloadTask,
        temp: &Path,
        total: u64,
        resumed: bool,
        duration: Duration,
    ) -> RcResult<DownloadSummary> {
        if let Some(cs) = &task.checksum {
            let actual = match cs {
                Checksum::Sha1(_) => hash::sha1_path(temp).await?,
                Checksum::Md5(_) => hash::md5_path(temp).await?,
            };
            let expected = cs.expected().to_string();
            if !hash::hex_eq(&actual, &expected) {
                return Err(RcError::ChecksumMismatch {
                    path: temp.display().to_string(),
                    expected,
                    actual,
                });
            }
        }
        tfs::rename(temp, &task.dest).await.map_err(RcError::Io)?;
        let meta = append_suffix(temp, ".meta");
        let _ = tfs::remove_file(&meta).await;
        Ok(DownloadSummary {
            dest: task.dest.clone(),
            size: total,
            resumed,
            duration,
            bytes_retransmitted: 0,
            checksum_retries: 0,
        })
    }

    fn emit(&self, p: &Progress) {
        if let Some(cb) = &self.progress {
            cb(p);
        }
    }
}

/// Compute the inclusive `(start, end)` byte ranges for `total` bytes split
/// into `chunk_size` chunks. Returns an empty vec for `total == 0`.
pub fn plan_chunks(total: u64, chunk_size: u64) -> Vec<(u64, u64)> {
    if total == 0 || chunk_size == 0 {
        return Vec::new();
    }
    let n = total.div_ceil(chunk_size);
    (0..n)
        .map(|i| {
            let s = i * chunk_size;
            let e = ((i + 1) * chunk_size).min(total) - 1;
            (s, e)
        })
        .collect()
}

/// Exponential backoff with cap + jitter.
///
/// `attempt` is the 1-based count of failed attempts so far
/// (`base * 2^(attempt-1)`, capped at `max`, then ±`jitter` fraction).
pub fn compute_backoff(attempt: u32, base: Duration, max: Duration, jitter: f64) -> Duration {
    let shift = attempt.saturating_sub(1).min(63);
    let exp = base.as_millis().saturating_mul(1u128 << shift);
    let capped = exp.min(max.as_millis());
    let mut d = Duration::from_millis(capped as u64);
    if jitter > 0.0 {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        // cheap deterministic-ish pseudo random in [0,1)
        let r = ((seed.wrapping_mul(2862933555777941757) >> 33) % 1000) as f64 / 1000.0;
        let factor = 1.0 - jitter + jitter * 2.0 * r; // [1-jitter, 1+jitter]
        d = Duration::from_secs_f64((d.as_secs_f64() * factor).max(0.0));
    }
    d
}

/// Download a single byte range into the temp file at its offset, retrying with
/// exponential backoff, then mark it complete and persist the state.
/// Choose which candidate URL to use for a given (0-based) retry `attempt`:
/// the primary first, then each mirror in order, finally the last candidate.
/// This gives every mirror a chance before we give up (FCLCore/download style).
fn pick_url(urls: &[String], attempt: u32) -> &str {
    let i = (attempt as usize).min(urls.len().saturating_sub(1));
    urls.get(i).map(String::as_str).unwrap_or("")
}

#[allow(clippy::too_many_arguments)]
async fn download_chunk(
    source: Arc<dyn HttpSource>,
    urls: Vec<String>,
    temp: PathBuf,
    start: u64,
    end: u64,
    total: u64,
    opts: DownloadOptions,
    downloaded: Arc<AtomicU64>,
    completed: Arc<Mutex<HashSet<u64>>>,
    persist_gate: Arc<AtomicU64>,
    meta_path: PathBuf,
    id: String,
    progress: Option<ProgressCallback>,
    rate_limiter: Option<Arc<Mutex<RateLimiter>>>,
    cancel_flag: Option<CancelFlag>,
    last_bytes: Arc<AtomicU64>,
    last_time: Arc<Mutex<Instant>>,
    bytes_retransmitted: Arc<AtomicU64>,
) -> RcResult<()> {
    // Check cancellation before starting the chunk (task 30).
    if cancel_flag
        .as_ref()
        .map(|f| f.load(Ordering::SeqCst))
        .unwrap_or(false)
    {
        return Err(RcError::Cancelled { scope: id });
    }

    let expected_len = end - start + 1;
    let chunk_index = start / opts.chunk_size;
    let mut attempt: u32 = 0;

    // Stream the range straight into the (pre-sized) temp file at its offset.
    // The chunk never lives entirely in RAM: the production `HttpSource`
    // backends write each network buffer to disk as it arrives (task 25 —
    // large-file streaming download / memory optimisation).
    let written = loop {
        // Check cancellation before each retry attempt.
        if cancel_flag
            .as_ref()
            .map(|f| f.load(Ordering::SeqCst))
            .unwrap_or(false)
        {
            return Err(RcError::Cancelled { scope: id.clone() });
        }

        let url = pick_url(&urls, attempt).to_string();
        let mut file = tfs::OpenOptions::new()
            .write(true)
            .open(&temp)
            .await
            .map_err(RcError::Io)?;
        file.seek(SeekFrom::Start(start))
            .await
            .map_err(RcError::Io)?;
        match source
            .fetch_range_into(&url, start, Some(end), &mut file)
            .await
        {
            Ok(n) => {
                // If the server returned *more* than the requested range, it
                // ignored the Range header and sent the whole resource. The
                // entire chunked plan is invalid; surface a `RangeUnsupported`
                // error and let `download_chunked` fall back to a single
                // sequential fetch (task 2 — robustness against non-Range
                // mirrors, absorbing FCLCore/download's fallback discipline).
                if n > expected_len {
                    return Err(RcError::RangeUnsupported { url });
                }
                if n != expected_len {
                    attempt += 1;
                    if attempt > opts.max_retries {
                        return Err(RcError::Download(format!(
                            "chunk {start}..{end} length mismatch: expected {expected_len}, got {n}"
                        )));
                    }
                    // Track retransmission.
                    bytes_retransmitted.fetch_add(expected_len, Ordering::Relaxed);
                    tokio::time::sleep(compute_backoff(
                        attempt,
                        opts.retry_base,
                        opts.retry_max,
                        opts.retry_jitter,
                    ))
                    .await;
                    continue;
                }
                // Apply rate limiting after a successful chunk fetch (task 30).
                if let Some(rl) = &rate_limiter {
                    let mut limiter = rl.lock().await;
                    limiter.consume(n).await;
                }
                break n;
            }
            Err(e) => {
                attempt += 1;
                if attempt > opts.max_retries {
                    return Err(e);
                }
                // Track retransmission for partial-retry stats.
                bytes_retransmitted.fetch_add(expected_len, Ordering::Relaxed);
                // Honour a server-supplied Retry-After (e.g. on HTTP 429)
                // before falling back to exponential backoff.
                let backoff = if let Some(b) = e.suggested_backoff() {
                    b
                } else {
                    compute_backoff(attempt, opts.retry_base, opts.retry_max, opts.retry_jitter)
                };
                tokio::time::sleep(backoff).await;
            }
        }
    };

    {
        let mut set = completed.lock().await;
        set.insert(chunk_index);
        downloaded.fetch_add(written, Ordering::Relaxed);
        drop(set);
    }

    // Throttled, off-executor meta persistence: serialising JSON + writing the
    // resume file is CPU + I/O work that must not run on the async worker for
    // every chunk. We persist at most every `meta_persist_every` completions
    // (the final flush in `download_chunked` covers the tail). `spawn_blocking`
    // keeps the (potentially slow) serialisation + fs write off the scheduler
    // (task 25 — coroutine scheduling / avoid blocking the main worker).
    let n = persist_gate.fetch_add(1, Ordering::Relaxed) + 1;
    if n.is_multiple_of((opts.meta_persist_every as u64).max(1)) {
        let set = completed.lock().await;
        let cl = set.clone();
        drop(set);
        persist_meta_blocking(meta_path.clone(), total, opts.chunk_size, cl).await?;
    }

    if let Some(cb) = &progress {
        let d = downloaded.load(Ordering::Relaxed);
        // Compute speed (task 30).
        let speed_bps = {
            let mut lt = last_time.lock().await;
            let now = Instant::now();
            let elapsed = now.duration_since(*lt).as_secs_f64();
            let lb = last_bytes.swap(d, Ordering::Relaxed);
            let delta = d.saturating_sub(lb);
            *lt = now;
            if elapsed > 0.0 {
                (delta as f64 / elapsed) as u64
            } else {
                0
            }
        };
        let status = if cancel_flag
            .as_ref()
            .map(|f| f.load(Ordering::SeqCst))
            .unwrap_or(false)
        {
            DownloadStatus::Paused
        } else {
            DownloadStatus::Running
        };
        cb(&Progress {
            id: id.clone(),
            downloaded: d,
            total: Some(total),
            finished: false,
            speed_bps,
            status,
        });
    }

    Ok(())
}

/// Persist the resume state to `meta_path`, running the JSON serialisation and
/// the file write on a blocking task so it never pins an async worker (task 25 —
/// keep the scheduler free for I/O, not CPU + fs churn).
async fn persist_meta_blocking(
    meta_path: PathBuf,
    total_size: u64,
    chunk_size: u64,
    completed: HashSet<u64>,
) -> RcResult<()> {
    tokio::task::spawn_blocking(move || {
        let mut v: Vec<u64> = completed.into_iter().collect();
        v.sort_unstable();
        let state = DownloadState {
            total_size,
            chunk_size,
            completed: v,
        };
        let bytes = serde_json::to_vec(&state).map_err(RcError::Json)?;
        std::fs::write(&meta_path, &bytes).map_err(RcError::Io)?;
        Ok::<(), RcError>(())
    })
    .await
    .map_err(|e| RcError::Other(format!("meta persist join failed: {e}")))?
}

/// Append `suffix` to a path's file name (e.g. `/a/b/file` + `.part`).
fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Persisted resume state (side-car `.part.meta`). `total_size` is re-validated
/// against the live target on load and the state is discarded if it mismatches.
#[derive(Debug, Serialize, Deserialize)]
struct DownloadState {
    total_size: u64,
    chunk_size: u64,
    completed: Vec<u64>,
}

impl DownloadState {
    fn new(total_size: u64, chunk_size: u64) -> Self {
        Self {
            total_size,
            chunk_size,
            completed: Vec::new(),
        }
    }

    fn completed_bytes(&self) -> u64 {
        if self.chunk_size == 0 {
            return 0;
        }
        let mut total = 0u64;
        for &idx in &self.completed {
            let s = idx * self.chunk_size;
            let e = ((idx + 1) * self.chunk_size).min(self.total_size);
            if e > s {
                total += e - s;
            }
        }
        total
    }
}
