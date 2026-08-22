//! Crash logging + reporting (task 19).
//!
//! A launcher running on a phone has no console, so when the core panics or the
//! game crashes we must capture enough to diagnose *and* surface it to the UI:
//!
//! * a process-wide, bounded [`LogRing`] of recent [`LogEntry`]s, fed by
//!   [`record_log`] (the launch engine pumps each game log line through it),
//! * a [`CrashLog`] struct combining a diagnosis kind + message, an optional
//!   captured backtrace and the recent logs,
//! * [`write_crash_log`] to persist a report under `crash/` for later upload,
//! * [`emit_crash_event`] to push an `error` event onto the global bus
//!   ([`crate::event`], task 10) so the Compose UI can show it without polling,
//! * [`install_crash_reporter`] to install a panic hook recording every panic as
//!   a crash log — so a panic never silently kills the app.
//!
//! All paths are defensive: a write failure or a misbehaving sink can never take
//! down the core (task 19).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{RcError, RcResult};
use crate::event;

/// A single captured log line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub ts: u64,
    pub level: String,
    pub line: String,
}

/// Bounded ring buffer of recent log lines, captured process-wide so a crash
/// report always includes the lead-up to the failure.
pub struct LogRing {
    inner: Mutex<VecDeque<LogEntry>>,
    cap: usize,
}

impl LogRing {
    pub fn new(cap: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(cap)),
            cap: cap.max(1),
        }
    }

    /// Push a line, evicting the oldest when full.
    pub fn push(&self, level: &str, line: &str) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.len() >= self.cap {
            g.pop_front();
        }
        g.push_back(LogEntry {
            ts: now_secs(),
            level: level.to_string(),
            line: line.to_string(),
        });
    }

    /// The most recent `n` lines, newest first.
    pub fn snapshot(&self, n: usize) -> Vec<LogEntry> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.iter().rev().take(n).cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

static RING: OnceLock<LogRing> = OnceLock::new();

fn ring() -> &'static LogRing {
    RING.get_or_init(|| LogRing::new(512))
}

/// Record a log line into the process-wide ring (cheap, bounded).
pub fn record_log(level: &str, line: &str) {
    ring().push(level, line);
}

/// Take a snapshot of the most recent `n` captured log lines (newest first).
pub fn recent_logs(n: usize) -> Vec<LogEntry> {
    ring().snapshot(n)
}

/// A persisted crash report: diagnosis + captured backtrace + recent logs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashLog {
    pub id: String,
    pub timestamp: u64,
    /// Diagnosis kind, e.g. `"panic"`, `"out_of_memory"`, `"native_crash"`.
    pub kind: String,
    /// Human readable summary message.
    pub message: String,
    /// Captured backtrace, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backtrace: Option<String>,
    /// Recent log lines leading up to the crash (newest first).
    pub logs: Vec<LogEntry>,
    /// Arbitrary structured context (exit code, evidence, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

impl CrashLog {
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id: gen_id(),
            timestamp: now_secs(),
            kind: kind.into(),
            message: message.into(),
            backtrace: None,
            logs: recent_logs(200),
            context: None,
        }
    }

    pub fn with_backtrace(mut self, bt: impl Into<String>) -> Self {
        self.backtrace = Some(bt.into());
        self
    }

    pub fn with_logs(mut self, logs: Vec<LogEntry>) -> Self {
        self.logs = logs;
        self
    }

    pub fn with_context(mut self, c: serde_json::Value) -> Self {
        self.context = Some(c);
        self
    }
}

/// Write a crash log as `dir/crash_<ts>_<id>.json` and return the path.
///
/// Best-effort: a failure to write (e.g. read-only storage) is returned as an
/// error rather than panicking, so the caller can decide whether to ignore it.
pub fn write_crash_log(dir: &Path, report: &CrashLog) -> RcResult<PathBuf> {
    std::fs::create_dir_all(dir).map_err(RcError::Io)?;
    let name = format!("crash_{}_{}.json", report.timestamp, report.id);
    let path = dir.join(name);
    let bytes = serde_json::to_vec_pretty(report).map_err(RcError::Json)?;
    std::fs::write(&path, bytes).map_err(RcError::Io)?;
    Ok(path)
}

/// Push a crash log onto the global event bus as an `error` event
/// ([`crate::event`], task 10), so the Compose UI can surface it.
pub fn emit_crash_event(report: &CrashLog) {
    event::publish(event::Event::error(
        "crash",
        format!("[{}] {}", report.kind, report.message),
    ));
}

/// Record + persist + emit a crash log. Convenience used by the launch engine
/// and the panic hook.
pub fn report_crash(dir: &Path, report: &CrashLog) -> RcResult<PathBuf> {
    let path = write_crash_log(dir, report)?;
    emit_crash_event(report);
    Ok(path)
}

static REPORTER_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Install a panic hook that writes a crash log (with captured backtrace +
/// recent logs) under `data_dir/crash/` and emits an error event.
///
/// Idempotent: the first call wins, subsequent calls return `false`. Safe to
/// call from any thread; the hook is fully defensive and never panics.
pub fn install_crash_reporter(data_dir: PathBuf) -> bool {
    if REPORTER_DIR.set(data_dir).is_err() {
        return false;
    }
    let dir = REPORTER_DIR.get().unwrap().join("crash");
    std::panic::set_hook(Box::new(move |info: &std::panic::PanicHookInfo| {
        let msg = panic_message(info);
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        let report = CrashLog::new("panic", msg.clone())
            .with_backtrace(bt)
            .with_context(serde_json::json!({ "location": panic_location(info) }));
        // Best-effort: never let crash reporting take down the process.
        if let Ok(p) = write_crash_log(&dir, &report) {
            let _ = p;
        }
        emit_crash_event(&report);
    }));
    true
}

fn panic_message(info: &std::panic::PanicHookInfo) -> String {
    match info.payload().downcast_ref::<&str>() {
        Some(s) => (*s).to_string(),
        None => match info.payload().downcast_ref::<String>() {
            Some(s) => s.clone(),
            None => "unknown panic".to_string(),
        },
    }
}

fn panic_location(info: &std::panic::PanicHookInfo) -> String {
    match info.location() {
        Some(l) => format!("{}:{}:{}", l.file(), l.line(), l.column()),
        None => "unknown".to_string(),
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn gen_id() -> String {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{n:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_ring_is_bounded_and_lifo() {
        let ring = LogRing::new(3);
        assert!(ring.is_empty());
        ring.push("info", "a");
        ring.push("info", "b");
        ring.push("info", "c");
        ring.push("info", "d"); // evicts "a"
        assert_eq!(ring.len(), 3);
        let snap = ring.snapshot(10);
        assert_eq!(snap.len(), 3);
        assert_eq!(snap[0].line, "d");
        assert_eq!(snap[2].line, "b");
    }

    #[test]
    fn record_log_lands_in_global_ring() {
        record_log("warn", "rc-marker-line-123");
        let snap = recent_logs(50);
        assert!(snap.iter().any(|e| e.line == "rc-marker-line-123"));
    }

    #[test]
    fn crash_log_serialises_roundtrip() {
        let logs = vec![LogEntry {
            ts: 1,
            level: "error".into(),
            line: "boom".into(),
        }];
        let report = CrashLog::new("out_of_memory", "the game ran out of memory")
            .with_logs(logs.clone())
            .with_context(serde_json::json!({ "exit_code": 1 }));
        let text = serde_json::to_string(&report).unwrap();
        let back: CrashLog = serde_json::from_str(&text).unwrap();
        assert_eq!(back.kind, "out_of_memory");
        assert_eq!(back.message, "the game ran out of memory");
        assert_eq!(back.logs.len(), 1);
        assert_eq!(back.context.as_ref().unwrap()["exit_code"], 1);
    }

    #[test]
    fn write_crash_log_persists_file() {
        let dir = tempfile::tempdir().unwrap();
        let report = CrashLog::new("panic", "something panicked").with_backtrace("at main.rs:1");
        let path = write_crash_log(dir.path(), &report).unwrap();
        assert!(path.exists());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("something panicked"));
        assert!(text.contains("panic"));
    }

    #[test]
    fn report_crash_writes_and_is_safe_without_sink() {
        // No event sink is subscribed in tests; emit_crash_event is a no-op.
        let dir = tempfile::tempdir().unwrap();
        let report = CrashLog::new("native_crash", "SIGSEGV");
        let path = report_crash(dir.path(), &report).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn install_crash_reporter_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let first = install_crash_reporter(dir.path().to_path_buf());
        let second = install_crash_reporter(dir.path().to_path_buf());
        assert!(first);
        assert!(!second);
    }
}
