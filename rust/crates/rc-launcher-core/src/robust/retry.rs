//! Network-jitter retry with exponential backoff + jitter (task 19).
//!
//! A single, reusable retry primitive so the downloader, the network client and
//! the offline cache all replay transient failures the *same* way. Retries are
//! driven by [`crate::error::RcError::is_retryable`] (only `Transient` errors
//! are replayed) and paced by [`compute_backoff`] — exponential growth, capped,
//! with a random ±jitter so a thundering herd of retries does not synchronise.
//!
//! The backoff math lives in [`crate::download::compute_backoff`] (shared with
//! the chunked downloader) and is re-exported here as the canonical entry point.

use std::future::Future;
use std::time::Duration;

use crate::error::{RcError, RcResult};

/// Re-export of the canonical exponential-backoff helper.
pub use crate::download::compute_backoff;

/// Classifies whether an error is worth retrying.
///
/// Implemented for [`RcError`] (delegating to [`RcError::is_retryable`]); a
/// custom error type can implement it to plug into [`retry_with_policy`].
pub trait RetryClassifier {
    /// `true` if the operation should be retried after this error.
    fn is_retryable(&self) -> bool;
}

impl RetryClassifier for RcError {
    fn is_retryable(&self) -> bool {
        RcError::is_retryable(self)
    }
}

/// Exponential-backoff retry policy with jitter (network jitter).
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of *attempts* (the first call counts as attempt 0).
    pub max_attempts: u32,
    /// Base delay, doubled each attempt.
    pub base: Duration,
    /// Upper bound for a single backoff delay.
    pub max_delay: Duration,
    /// Jitter fraction in `[0, 1]`: the actual delay is multiplied by a factor
    /// in `[1 - jitter, 1 + jitter]` so retries desynchronise.
    pub jitter: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base: Duration::from_millis(400),
            max_delay: Duration::from_secs(20),
            jitter: 0.25,
        }
    }
}

/// A policy tuned for unit tests: tiny delays, few attempts.
impl RetryPolicy {
    /// A policy with negligible delays, for tests that exercise retry paths
    /// without spending real time.
    pub fn for_tests() -> Self {
        Self {
            max_attempts: 3,
            base: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
            jitter: 0.0,
        }
    }
}

/// Retry an async operation with exponential backoff + jitter.
///
/// `op` is invoked, and whenever it returns `Err` for which `classifier(&err)`
/// is `true` *and* attempts remain, the task sleeps for [`compute_backoff`] and
/// tries again. Once attempts are exhausted (or the error is not retryable) the
/// last error is returned.
pub async fn retry_with_policy<F, Fut, T, E, C>(
    policy: &RetryPolicy,
    classifier: C,
    mut op: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    C: Fn(&E) -> bool,
{
    let mut attempt: u32 = 0;
    loop {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                let can_retry = classifier(&e);
                if !can_retry || attempt + 1 >= policy.max_attempts {
                    return Err(e);
                }
                let backoff =
                    compute_backoff(attempt + 1, policy.base, policy.max_delay, policy.jitter);
                tokio::time::sleep(backoff).await;
                attempt += 1;
            }
        }
    }
}

/// Convenience retry for [`RcResult`] operations: replays only errors for which
/// [`RcError::is_retryable`] is `true` (i.e. `Transient` network failures).
pub async fn retry<F, Fut, T>(policy: &RetryPolicy, op: F) -> RcResult<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = RcResult<T>>,
{
    retry_with_policy(policy, |e: &RcError| e.is_retryable(), op).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn backoff_grows_and_caps() {
        let b0 = compute_backoff(
            1,
            Duration::from_millis(100),
            Duration::from_millis(1000),
            0.0,
        );
        let b1 = compute_backoff(
            2,
            Duration::from_millis(100),
            Duration::from_millis(1000),
            0.0,
        );
        let b5 = compute_backoff(
            5,
            Duration::from_millis(100),
            Duration::from_millis(1000),
            0.0,
        );
        assert_eq!(b0, Duration::from_millis(100));
        assert_eq!(b1, Duration::from_millis(200));
        assert_eq!(b5, Duration::from_millis(1000)); // capped
    }

    #[test]
    fn backoff_is_monotone_and_within_bounds() {
        let mut prev = Duration::ZERO;
        for a in 1..10u32 {
            let d = compute_backoff(a, Duration::from_millis(50), Duration::from_secs(5), 0.0);
            assert!(d >= prev);
            assert!(d <= Duration::from_secs(5));
            prev = d;
        }
    }

    #[tokio::test]
    async fn succeeds_first_try() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let res: RcResult<u32> = retry(&RetryPolicy::for_tests(), || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(7u32)
            }
        })
        .await;
        assert_eq!(res.unwrap(), 7);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retries_transient_then_succeeds() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let res: RcResult<u32> = retry(&RetryPolicy::for_tests(), || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    Err(RcError::Network("blip".into()))
                } else {
                    Ok(42u32)
                }
            }
        })
        .await;
        assert_eq!(res.unwrap(), 42);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn gives_up_after_max_attempts() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let policy = RetryPolicy {
            max_attempts: 3,
            ..RetryPolicy::for_tests()
        };
        let res: RcResult<u32> = retry(&policy, || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(RcError::Connection("down".into()))
            }
        })
        .await;
        assert!(res.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn does_not_retry_fatal_errors() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let res: RcResult<u32> = retry(&RetryPolicy::for_tests(), || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(RcError::ChecksumMismatch {
                    path: "p".into(),
                    expected: "a".into(),
                    actual: "b".into(),
                })
            }
        })
        .await;
        assert!(res.is_err());
        // fatal errors are not replayed
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn custom_classifier_controls_retry() {
        // Only retry timeouts, not connections.
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let policy = RetryPolicy::for_tests();
        let res: RcResult<u32> = retry_with_policy(
            &policy,
            |e: &RcError| matches!(e, RcError::Timeout(_)),
            || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err(RcError::Connection("refused".into()))
                }
            },
        )
        .await;
        assert!(res.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
