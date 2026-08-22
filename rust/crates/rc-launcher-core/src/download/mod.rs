//! Asynchronous download manager with resume + verification (task 2).
//!
//! The public surface mirrors FCLCore's `download` package but is implemented
//! in safe Rust on top of `tokio` + `reqwest`:
//!
//! * [`DownloadManager`] — resumable, parallel, chunked downloads.
//! * [`Checksum`] / [`DownloadTask`] / [`DownloadOptions`] — request config.
//! * [`client::HttpSource`] / [`client::ReqwestSource`] — HTTP backend.
//! * [`hash`] — SHA-1 / MD5 verification helpers.
//! * [`compute_backoff`] / [`plan_chunks`] — retry + chunk-planning helpers.
//!
//! Robustness is taken from FCLCore/download + cuberite: Range-based resume,
//! parallel shards, checksum verification, exponential backoff, and a single
//! cumulative progress callback.

mod client;
mod hash;
mod manager;

#[cfg(test)]
pub(crate) mod testing;

pub use client::{FetchResult, HttpSource, ReqwestSource};
pub use hash::{hex_eq, md5_bytes, md5_path, sha1_bytes, sha1_path};
pub use manager::{
    compute_backoff, plan_chunks, Checksum, DownloadManager, DownloadOptions, DownloadSummary,
    DownloadTask, Progress, ProgressCallback,
};
