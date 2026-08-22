//! Unified error model for the Rust core.
//!
//! A single error type keeps the FFI boundary simple: every fallible core
//! operation returns [`RcResult`], which the JNI layer converts into a Java
//! exception payload (see [`crate::ffi`] and task 19 — robustness).
//!
//! The model is *unified* across every subsystem (download, net, auth, launch,
//! mods, runtime, plugins) and every error carries **recoverability metadata**
//! (task 19):
//!
//! * [`RcError::severity`] — `Transient` / `Recoverable` / `Fatal`,
//! * [`RcError::is_transient`] / [`RcError::is_retryable`] — whether a
//!   network-jitter retry should be attempted,
//! * [`RcError::suggested_backoff`] — an explicit wait (e.g. a `Retry-After`
//!   from a 429) before the next attempt.
//!
//! The retry/backoff layer ([`crate::robust::retry`]) and the offline cache
//! ([`crate::robust::cache`]) both consult this metadata so the whole core
//! agrees on what is worth retrying and what should degrade to a local copy.
//! This absorbs cuberite's "fail fast, retry the network" discipline and
//! FCLCore's defensive download/launch handling.

use std::io;
use std::time::Duration;

use thiserror::Error;

/// How severe an error is — drives the retry and offline-degradation policy.
///
/// Absorbs cuberite's defensive discipline and FCLCore's download/launch
/// resilience: only `Transient` errors are retried with backoff, `Recoverable`
/// ones may be worked around (mirror fallback, cached copy, offline account),
/// and `Fatal` ones must surface to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    /// A transient hiccup that will likely resolve by retrying (network blip,
    /// 5xx, timeout, rate limit). The operation should be retried with backoff.
    Transient,
    /// A problem the launcher can work around (mirror fallback, cached copy,
    /// offline account, pick another file). Retrying the same input will not
    /// help, but degrading will.
    Recoverable,
    /// A hard failure that will not recover without user/manual intervention
    /// (corrupt file, missing dependency, bad argument, panic).
    Fatal,
}

impl ErrorSeverity {
    /// The i18n key of this severity's human label (task 20).
    pub const fn i18n_key(self) -> &'static str {
        match self {
            ErrorSeverity::Transient => "error.severity.transient",
            ErrorSeverity::Recoverable => "error.severity.recoverable",
            ErrorSeverity::Fatal => "error.severity.fatal",
        }
    }
}

/// All errors that can escape the core.
#[derive(Debug, Error)]
pub enum RcError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("jni error: {0}")]
    Jni(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("auth error: {0}")]
    Auth(String),

    #[error("download error: {0}")]
    Download(String),

    #[error("checksum mismatch for {path}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },

    #[error("remote resource at {url} does not support HTTP Range requests (resume disabled)")]
    RangeUnsupported { url: String },

    #[error("launch error: {0}")]
    Launch(String),

    #[error("mod / resource-pack error: {0}")]
    Mod(String),

    #[error("missing file: {0}")]
    MissingFile(String),

    #[error("unsupported platform: {0}")]
    UnsupportedPlatform(String),

    /// The network is unreachable / DNS poisoned beyond recovery. The caller
    /// should degrade to a cached or offline path instead of failing hard.
    #[error("network unreachable (offline): {0}")]
    Offline(String),

    /// A request timed out (connect or read). Transient — retry with backoff.
    #[error("timeout: {0}")]
    Timeout(String),

    /// A connection-level failure (reset / refused / aborted). Transient.
    #[error("connection error: {0}")]
    Connection(String),

    /// The server asked us to slow down. `retry_after` is the explicit backoff
    /// (e.g. from a `Retry-After` header or similar signal).
    #[error("rate limited")]
    RateLimited { retry_after: Option<Duration> },

    /// A local cache read/write/serialisation failure. Recoverable — the caller
    /// can usually bypass the cache and hit the network.
    #[error("cache error: {0}")]
    Cache(String),

    /// An unexpected panic was caught across an FFI / task boundary. Fatal, but
    /// the launcher should record it and keep running (task 19).
    #[error("internal panic: {0}")]
    Panic(String),

    #[error("{0}")]
    Other(String),
}

impl RcError {
    /// The severity bucket of this error.
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            RcError::Io(e) => io_severity(e),
            RcError::Json(_) => ErrorSeverity::Recoverable,
            RcError::Jni(_) => ErrorSeverity::Fatal,
            RcError::Network(_) => ErrorSeverity::Transient,
            RcError::Auth(_) => ErrorSeverity::Recoverable,
            RcError::Download(_) => ErrorSeverity::Recoverable,
            RcError::ChecksumMismatch { .. } => ErrorSeverity::Fatal,
            RcError::RangeUnsupported { .. } => ErrorSeverity::Recoverable,
            RcError::Launch(_) => ErrorSeverity::Fatal,
            RcError::Mod(_) => ErrorSeverity::Recoverable,
            RcError::MissingFile(_) => ErrorSeverity::Recoverable,
            RcError::UnsupportedPlatform(_) => ErrorSeverity::Fatal,
            RcError::Offline(_) => ErrorSeverity::Recoverable,
            RcError::Timeout(_) => ErrorSeverity::Transient,
            RcError::Connection(_) => ErrorSeverity::Transient,
            RcError::RateLimited { .. } => ErrorSeverity::Transient,
            RcError::Cache(_) => ErrorSeverity::Recoverable,
            RcError::Panic(_) => ErrorSeverity::Fatal,
            RcError::Other(_) => ErrorSeverity::Recoverable,
        }
    }

    /// Whether this is a transient / network-level failure worth retrying with
    /// backoff.
    pub fn is_transient(&self) -> bool {
        self.severity() == ErrorSeverity::Transient
    }

    /// Whether a network-jitter retry with backoff should be attempted.
    /// Currently every `Transient` error is retryable.
    pub fn is_retryable(&self) -> bool {
        self.is_transient()
    }

    /// An explicit backoff the caller should honour before retrying (e.g. a
    /// `Retry-After` from a rate-limited response). `None` means "use the
    /// policy's exponential backoff".
    pub fn suggested_backoff(&self) -> Option<Duration> {
        match self {
            RcError::RateLimited { retry_after } => *retry_after,
            _ => None,
        }
    }

    /// The i18n key of this error's **user-facing** message (task 20).
    ///
    /// The `Display` impl stays developer-facing (English, with technical
    /// detail) and goes to the log; this key resolves to translated copy in
    /// `i18n/<tag>.properties` for the UI. Exhaustive over the variants, so a
    /// new error variant cannot silently ship without user-facing copy.
    pub fn i18n_key(&self) -> &'static str {
        match self {
            RcError::Network(_) => "error.network",
            RcError::Timeout(_) => "error.timeout",
            RcError::Connection(_) => "error.network",
            RcError::Offline(_) => "error.offline",
            RcError::RateLimited { .. } => "error.rate_limited",
            RcError::ChecksumMismatch { .. } => "error.checksum",
            RcError::MissingFile(_) => "error.missing_file",
            RcError::Auth(_) => "error.auth",
            RcError::Launch(_) => "error.launch",
            RcError::Download(_) => "error.network",
            RcError::RangeUnsupported { .. } => "error.network",
            RcError::Io(_)
            | RcError::Json(_)
            | RcError::Jni(_)
            | RcError::Mod(_)
            | RcError::UnsupportedPlatform(_)
            | RcError::Cache(_)
            | RcError::Panic(_)
            | RcError::Other(_) => "error.unknown",
        }
    }

    /// The user-facing message in `language`, with placeholders filled in.
    ///
    /// `{detail}` carries the technical detail (so a bug report is still
    /// actionable) and `{path}` the offending file. Unknown languages resolve
    /// through the Chinese-first fallback chain, so this never returns a raw key
    /// for a shipped locale.
    pub fn localized(&self, language: crate::i18n::Language) -> String {
        let key = self.i18n_key();
        match self {
            RcError::ChecksumMismatch { path, .. } | RcError::MissingFile(path) => {
                crate::i18n::t_args_in(language, key, &[("path", path.as_str())])
            }
            RcError::RateLimited { .. } => crate::i18n::t_in(language, key),
            other => {
                // Every remaining variant renders `{detail}` from Display.
                let detail = other.to_string();
                crate::i18n::t_args_in(language, key, &[("detail", detail.as_str())])
            }
        }
    }

    /// The user-facing message in the current UI language.
    pub fn localized_current(&self) -> String {
        self.localized(crate::i18n::current_language())
    }

    /// Localised label of this error's [`severity`](Self::severity).
    pub fn severity_label(&self, language: crate::i18n::Language) -> String {
        crate::i18n::t_in(language, self.severity().i18n_key())
    }

    /// Seconds to wait before retrying a `RateLimited` error, if any.
    pub fn rate_limit_retry_after_secs(&self) -> Option<u64> {
        match self {
            RcError::RateLimited { retry_after } => retry_after.map(|d| d.as_secs()),
            _ => None,
        }
    }
}

/// Map an `io::ErrorKind` to a [`ErrorSeverity`].
///
/// Interrupted / timed-out / dropped connections are treated as transient so
/// the retry layer replays them; not-found / permission / existence errors are
/// recoverable (the caller can pick another path or surface to the user).
fn io_severity(e: &io::Error) -> ErrorSeverity {
    match e.kind() {
        io::ErrorKind::NotFound
        | io::ErrorKind::PermissionDenied
        | io::ErrorKind::AlreadyExists
        | io::ErrorKind::IsADirectory
        | io::ErrorKind::Unsupported => ErrorSeverity::Recoverable,
        io::ErrorKind::TimedOut
        | io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::BrokenPipe
        | io::ErrorKind::Interrupted => ErrorSeverity::Transient,
        _ => ErrorSeverity::Recoverable,
    }
}

/// Core-wide `Result` alias.
pub type RcResult<T> = Result<T, RcError>;

impl From<reqwest::Error> for RcError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            RcError::Timeout(e.to_string())
        } else if e.is_connect() {
            RcError::Connection(e.to_string())
        } else {
            RcError::Network(e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_io_error() {
        let e = RcError::Other("boom".into());
        assert_eq!(e.to_string(), "boom");
    }

    #[test]
    fn from_io() {
        let io = io::Error::new(io::ErrorKind::NotFound, "missing");
        let e: RcError = io.into();
        assert!(e.to_string().starts_with("io error"));
    }

    #[test]
    fn shows_checksum_mismatch() {
        let e = RcError::ChecksumMismatch {
            path: "/tmp/x".into(),
            expected: "aaa".into(),
            actual: "bbb".into(),
        };
        assert!(e.to_string().contains("aaa"));
        assert!(e.to_string().contains("bbb"));
    }

    #[test]
    fn shows_mod_error() {
        assert_eq!(
            RcError::Mod("bad manifest".into()).to_string(),
            "mod / resource-pack error: bad manifest"
        );
    }

    #[test]
    fn shows_launch_and_missing_file() {
        assert_eq!(
            RcError::Launch("no java".into()).to_string(),
            "launch error: no java"
        );
        assert_eq!(
            RcError::MissingFile("/x/y.jar".into()).to_string(),
            "missing file: /x/y.jar"
        );
    }

    #[test]
    fn shows_range_unsupported() {
        let e = RcError::RangeUnsupported {
            url: "https://example/x".into(),
        };
        assert!(e.to_string().contains("example/x"));
    }

    #[test]
    fn classifies_transient_network_errors() {
        assert!(RcError::Network("down".into()).is_transient());
        assert!(RcError::Timeout("slow".into()).is_transient());
        assert!(RcError::Connection("reset".into()).is_transient());
        assert!(RcError::RateLimited { retry_after: None }.is_transient());
        assert!(RcError::RateLimited {
            retry_after: Some(Duration::from_secs(5))
        }
        .is_transient());
        // all transient errors are retryable
        assert!(RcError::Network("down".into()).is_retryable());
        assert!(RcError::Timeout("slow".into()).is_retryable());
    }

    #[test]
    fn classifies_non_retryable_errors() {
        assert!(!RcError::ChecksumMismatch {
            path: "p".into(),
            expected: "a".into(),
            actual: "b".into()
        }
        .is_transient());
        assert!(!RcError::Launch("x".into()).is_transient());
        assert!(!RcError::Jni("x".into()).is_transient());
        assert!(!RcError::Panic("x".into()).is_transient());
        assert!(!RcError::UnsupportedPlatform("x".into()).is_transient());
        // 404-style download errors are not blindly retried
        assert!(!RcError::Download("unexpected status 404".into()).is_transient());
    }

    #[test]
    fn rate_limited_carries_backoff() {
        let e = RcError::RateLimited {
            retry_after: Some(Duration::from_secs(30)),
        };
        assert_eq!(e.suggested_backoff(), Some(Duration::from_secs(30)));
        assert_eq!(e.rate_limit_retry_after_secs(), Some(30));
        let e2 = RcError::RateLimited { retry_after: None };
        assert_eq!(e2.suggested_backoff(), None);
        assert_eq!(e2.rate_limit_retry_after_secs(), None);
    }

    #[test]
    fn io_kind_maps_to_severity() {
        assert_eq!(
            RcError::Io(io::Error::new(io::ErrorKind::TimedOut, "t")).severity(),
            ErrorSeverity::Transient
        );
        assert_eq!(
            RcError::Io(io::Error::new(io::ErrorKind::ConnectionReset, "r")).severity(),
            ErrorSeverity::Transient
        );
        assert_eq!(
            RcError::Io(io::Error::new(io::ErrorKind::NotFound, "n")).severity(),
            ErrorSeverity::Recoverable
        );
    }

    #[test]
    fn offline_is_recoverable_not_transient() {
        assert!(!RcError::Offline("dns".into()).is_transient());
        assert_eq!(
            RcError::Offline("dns".into()).severity(),
            ErrorSeverity::Recoverable
        );
    }

    // --- i18n integration (task 20) -------------------------------------

    /// Every variant must map to a key that exists in *every* shipped locale,
    /// otherwise a user would see a raw `error.*` key in a failure dialog.
    #[test]
    fn every_variant_has_user_facing_copy_in_every_language() {
        let _g = crate::i18n::GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        use crate::i18n::Language;
        let variants: Vec<RcError> = vec![
            RcError::Io(io::Error::new(io::ErrorKind::NotFound, "missing")),
            RcError::Json(serde_json::from_str::<serde_json::Value>("{").unwrap_err()),
            RcError::Jni("bad env".into()),
            RcError::Network("reset".into()),
            RcError::Auth("token".into()),
            RcError::Download("stalled".into()),
            RcError::ChecksumMismatch {
                path: "/sdcard/a.jar".into(),
                expected: "aa".into(),
                actual: "bb".into(),
            },
            RcError::RangeUnsupported {
                url: "http://x".into(),
            },
            RcError::Launch("no jre".into()),
            RcError::Mod("bad toml".into()),
            RcError::MissingFile("/sdcard/b.jar".into()),
            RcError::UnsupportedPlatform("riscv".into()),
            RcError::Offline("no route".into()),
            RcError::Timeout("connect".into()),
            RcError::Connection("refused".into()),
            RcError::RateLimited {
                retry_after: Some(Duration::from_secs(3)),
            },
            RcError::Cache("corrupt".into()),
            RcError::Panic("boom".into()),
            RcError::Other("misc".into()),
        ];
        for e in &variants {
            let key = e.i18n_key();
            assert!(key.starts_with("error."), "{key}");
            for l in Language::ALL {
                assert!(
                    crate::i18n::has_key(l, key),
                    "{} lacks {} (variant {:?})",
                    l.tag(),
                    key,
                    e
                );
                let msg = e.localized(l);
                assert!(!msg.trim().is_empty());
                assert_ne!(msg, key, "{} / {} rendered as its own key", l.tag(), key);
                assert!(
                    !msg.contains('{'),
                    "{} / {} left a placeholder: {}",
                    l.tag(),
                    key,
                    msg
                );
            }
        }
    }

    #[test]
    fn localized_messages_carry_the_path_and_the_detail() {
        let _g = crate::i18n::GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        use crate::i18n::Language;
        let e = RcError::ChecksumMismatch {
            path: "/sdcard/mods/x.jar".into(),
            expected: "a".into(),
            actual: "b".into(),
        };
        assert!(e.localized(Language::ZhCn).contains("/sdcard/mods/x.jar"));
        assert!(e
            .localized(Language::En)
            .starts_with("Checksum verification failed"));
        // A `detail`-style variant keeps the technical text for bug reports.
        let t = RcError::Timeout("connect timed out after 15s".into());
        assert!(t
            .localized(Language::En)
            .contains("connect timed out after 15s"));
        assert!(t.localized(Language::ZhCn).contains("请求超时"));
        assert!(t.localized(Language::ZhHant).contains("請求逾時"));
        // Display stays developer-facing English regardless of the UI language.
        assert!(t.to_string().starts_with("timeout:"));
    }

    #[test]
    fn severity_labels_are_translated() {
        let _g = crate::i18n::GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        use crate::i18n::Language;
        let e = RcError::Timeout("x".into());
        assert_eq!(e.severity(), ErrorSeverity::Transient);
        assert_eq!(e.severity_label(Language::ZhCn), "临时故障");
        assert_eq!(e.severity_label(Language::En), "Transient");
        assert_eq!(e.severity_label(Language::ZhHant), "暫時性故障");
        for sev in [
            ErrorSeverity::Transient,
            ErrorSeverity::Recoverable,
            ErrorSeverity::Fatal,
        ] {
            for l in Language::ALL {
                assert!(crate::i18n::has_key(l, sev.i18n_key()));
            }
        }
    }

    #[test]
    fn localized_current_follows_the_ui_language() {
        use crate::i18n::{self, Language};
        let _g = i18n::GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let restore = i18n::current_language();
        let e = RcError::Offline("dns".into());
        i18n::set_language(Language::En);
        assert!(e.localized_current().contains("offline mode"));
        i18n::set_language(Language::ZhCn);
        assert!(e.localized_current().contains("离线模式"));
        i18n::set_language(restore);
    }
}
