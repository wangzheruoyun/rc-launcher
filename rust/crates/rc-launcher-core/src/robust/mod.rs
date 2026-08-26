//! Robustness layer (task 19).
//!
//! One coherent place for the launcher's defensive machinery, so every
//! subsystem shares the *same* policy instead of re-implementing it:
//!
//! * [`error`] — the unified [`crate::error::RcError`] model already carries
//!   recoverability metadata (`severity` / `is_retryable` / `suggested_backoff`).
//! * [`retry`] — network-jitter retry with exponential backoff + jitter, driven
//!   by that metadata (only `Transient` errors are replayed).
//! * [`cache`] — an on-disk content cache plus offline **degradation**: when the
//!   network is flaky/unreachable we serve a stale-but-valid cached copy
//!   instead of failing, which is what keeps the launcher usable on a weak
//!   China-mainland link.
//! * [`reporter`] — crash logging + reporting: a bounded process-wide log ring,
//!   a serialisable [`reporter::CrashLog`], [`reporter::list_crash_logs`] /
//!   [`reporter::prune_crash_logs`] to manage accumulated reports, and a panic
//!   hook that records every panic so a crash never silently kills the app.
//!
//! The design absorbs cuberite's defensive discipline (fail fast, retry the
//! network, never panic across a boundary) and FCLCore's download/launch
//! resilience (resume, mirror fallback, crash diagnosis).

pub mod cache;
pub mod reporter;
pub mod retry;

pub use cache::{cache_key, cache_key_range, fetch_cached, CachePolicy, CacheStore, Cached};
pub use reporter::{
    emit_crash_event, install_crash_reporter, list_crash_logs, prune_crash_logs, recent_logs,
    record_log, report_crash, write_crash_log, CrashLog, LogEntry, LogRing,
};
pub use retry::{compute_backoff, retry, retry_with_policy, RetryClassifier, RetryPolicy};
