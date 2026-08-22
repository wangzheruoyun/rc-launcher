//! Game process spawning & supervision (task 7).
//!
//! The Rust counterpart of FCL's `FCLauncher` + `ProcessListener`: it starts the
//! JVM, streams its stdout/stderr, retains a bounded log ring buffer for crash
//! diagnosis, and reports the exit code / terminating signal.
//!
//! ```text
//!  LaunchCommand ──▶ SpawnSpec ──▶ GameProcess ──▶ GameExit { code, signal, log, crash }
//!                                    │  stdout ─┐
//!                                    │  stderr ─┴──▶ mpsc ──▶ callback (UI log window)
//!                                    └── stop()/kill() (SIGTERM → SIGKILL)
//! ```
//!
//! Robustness decisions that matter on a phone:
//!
//! * **Bounded memory.** Minecraft logs megabytes per session; [`LogBuffer`] is a
//!   ring buffer (`LaunchOptions::log_buffer_lines`), and single pathological
//!   lines are truncated, so a runaway mod cannot OOM the launcher itself.
//! * **Lossy decoding, never a panic.** Game output is *not* guaranteed UTF-8
//!   (native crash dumps, mods printing raw bytes), so lines are decoded lossily
//!   instead of erroring out.
//! * **No secret ever reaches the log.** Every line is redacted with the
//!   command's secrets (the Minecraft access token appears in `argv`).
//! * **No zombie / leaked process.** The child is spawned with `kill_on_drop`, and
//!   [`GameProcess::stop`] escalates SIGTERM → SIGKILL, so the game cannot outlive
//!   the launcher.
//! * **Output is never lost at exit.** After the process exits we keep draining
//!   the pipes briefly — the crash-report lines arrive *last*, and they are the
//!   ones the diagnosis needs.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::error::{RcError, RcResult};
use crate::launch::command::{redact_with, LaunchCommand};
use crate::launch::crash::{diagnose, CrashReport};

/// Default number of log lines retained for crash diagnosis.
pub const DEFAULT_LOG_CAPACITY: usize = 2048;
/// Longest single log line kept (longer ones are truncated).
pub const MAX_LINE_CHARS: usize = 8192;
/// Hard byte cap per read, so output *without* a newline (a broken mod printing
/// a gigabyte-long line, a binary blob) cannot grow the read buffer without
/// bound. UTF-8 worst case is 4 bytes per char.
const MAX_LINE_BYTES: usize = MAX_LINE_CHARS * 4;
/// How long to keep draining the pipes after the process exited.
const DRAIN_AFTER_EXIT: Duration = Duration::from_millis(500);
/// Default grace period for [`GameProcess::stop`] before SIGKILL.
pub const DEFAULT_STOP_GRACE: Duration = Duration::from_secs(5);

/// Which pipe a log line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogStream {
    Stdout,
    Stderr,
}

impl LogStream {
    /// Lowercase name, used as the JSON tag / log prefix.
    pub fn as_str(self) -> &'static str {
        match self {
            LogStream::Stdout => "stdout",
            LogStream::Stderr => "stderr",
        }
    }
}

/// One captured line of game output (already redacted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub stream: LogStream,
    pub text: String,
}

impl LogLine {
    /// A stdout line.
    pub fn out(text: impl Into<String>) -> Self {
        Self {
            stream: LogStream::Stdout,
            text: text.into(),
        }
    }

    /// A stderr line.
    pub fn err(text: impl Into<String>) -> Self {
        Self {
            stream: LogStream::Stderr,
            text: text.into(),
        }
    }

    /// Did it come from stderr?
    pub fn is_error(&self) -> bool {
        self.stream == LogStream::Stderr
    }
}

impl fmt::Display for LogLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.stream.as_str(), self.text)
    }
}

/// A bounded, in-memory tail of the game log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogBuffer {
    capacity: usize,
    lines: VecDeque<LogLine>,
    dropped: usize,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_LOG_CAPACITY)
    }
}

impl LogBuffer {
    /// A ring buffer holding at most `capacity` lines (0 => the default).
    pub fn new(capacity: usize) -> Self {
        let capacity = if capacity == 0 {
            DEFAULT_LOG_CAPACITY
        } else {
            capacity
        };
        Self {
            capacity,
            lines: VecDeque::with_capacity(capacity.min(1024)),
            dropped: 0,
        }
    }

    /// Append a line, evicting the oldest one when full.
    pub fn push(&mut self, line: LogLine) {
        if self.lines.len() == self.capacity {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.lines.push_back(line);
    }

    /// Retained lines, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &LogLine> {
        self.lines.iter()
    }

    /// Retained line texts, oldest first.
    pub fn texts(&self) -> Vec<&str> {
        self.lines.iter().map(|l| l.text.as_str()).collect()
    }

    /// The last `n` lines (what a crash dialog shows).
    pub fn tail(&self, n: usize) -> Vec<&LogLine> {
        let skip = self.lines.len().saturating_sub(n);
        self.lines.iter().skip(skip).collect()
    }

    /// Number of retained lines.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Is the buffer empty?
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Ring capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// How many lines were evicted (the log was longer than the buffer).
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// Render the buffer as text (what [`Self::write_to_file`] writes).
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        if self.dropped > 0 {
            out.push_str(&format!(
                "… {} earlier line(s) dropped (log buffer holds {})\n",
                self.dropped, self.capacity
            ));
        }
        for l in &self.lines {
            out.push_str(&l.text);
            out.push('\n');
        }
        out
    }

    /// Export the retained log (crash reporting / "share log" in the UI).
    pub fn write_to_file(&self, path: &Path) -> RcResult<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(path, self.to_text())?;
        Ok(())
    }
}

/// Everything needed to spawn a child process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    pub env: BTreeMap<String, String>,
    /// Start from an empty environment instead of inheriting the launcher's.
    pub clear_env: bool,
    /// Strings redacted from every captured line.
    pub secrets: Vec<String>,
    /// Ring-buffer size for the captured log.
    pub log_capacity: usize,
}

impl SpawnSpec {
    /// A spec for `program` with `args`, running in the current directory.
    pub fn new(program: impl Into<PathBuf>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            working_dir: PathBuf::from("."),
            env: BTreeMap::new(),
            clear_env: false,
            secrets: Vec::new(),
            log_capacity: DEFAULT_LOG_CAPACITY,
        }
    }

    /// The spec for an assembled [`LaunchCommand`].
    pub fn from_command(cmd: &LaunchCommand, log_capacity: usize) -> Self {
        Self {
            program: cmd.program.clone(),
            args: cmd.args(),
            working_dir: cmd.working_dir.clone(),
            env: cmd.env.as_map().clone(),
            clear_env: false,
            secrets: cmd.secrets().to_vec(),
            log_capacity,
        }
    }
}

/// A running game process.
pub struct GameProcess {
    child: Child,
    pid: u32,
    started: Instant,
    log: LogBuffer,
    secrets: Vec<String>,
    rx: UnboundedReceiver<LogLine>,
    stop_requested: bool,
}

impl fmt::Debug for GameProcess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameProcess")
            .field("pid", &self.pid)
            .field("uptime", &self.uptime())
            .field("log_lines", &self.log.len())
            .field("stop_requested", &self.stop_requested)
            .finish()
    }
}

impl GameProcess {
    /// Spawn the process described by `spec`.
    ///
    /// Must be called from within a Tokio runtime (the pipe readers are spawned
    /// as tasks). Fails *before* spawning when the program or working directory
    /// is missing, so the caller gets an actionable error instead of a bare
    /// `ENOENT`.
    pub fn spawn(spec: &SpawnSpec) -> RcResult<Self> {
        // A path (rather than a bare command name) must exist.
        if spec.program.components().count() > 1 && !spec.program.is_file() {
            return Err(RcError::MissingFile(format!(
                "java executable not found: {}",
                spec.program.display()
            )));
        }
        if !spec.working_dir.is_dir() {
            return Err(RcError::MissingFile(format!(
                "working directory not found: {}",
                spec.working_dir.display()
            )));
        }

        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args);
        cmd.current_dir(&spec.working_dir);
        if spec.clear_env {
            cmd.env_clear();
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // The game must never outlive the launcher process.
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| {
            RcError::Launch(format!("failed to spawn {}: {}", spec.program.display(), e))
        })?;
        let pid = child.id().unwrap_or(0);

        let (tx, rx) = mpsc::unbounded_channel();
        if let Some(out) = child.stdout.take() {
            tokio::spawn(pump(out, LogStream::Stdout, tx.clone()));
        }
        if let Some(err) = child.stderr.take() {
            tokio::spawn(pump(err, LogStream::Stderr, tx.clone()));
        }
        // Drop our own sender so the receiver closes when both readers finish.
        drop(tx);

        Ok(Self {
            child,
            pid,
            started: Instant::now(),
            log: LogBuffer::new(spec.log_capacity),
            secrets: spec.secrets.clone(),
            rx,
            stop_requested: false,
        })
    }

    /// OS process id (0 when it could not be determined).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// How long the process has been running.
    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }

    /// The captured log so far.
    pub fn log(&self) -> &LogBuffer {
        &self.log
    }

    /// Take the captured log out of the process.
    pub fn take_log(&mut self) -> LogBuffer {
        let capacity = self.log.capacity();
        std::mem::replace(&mut self.log, LogBuffer::new(capacity))
    }

    /// Did the launcher ask this process to stop?
    pub fn stop_requested(&self) -> bool {
        self.stop_requested
    }

    /// Is the process still alive? (Non-blocking.)
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Read the next output line, recording it in the log buffer.
    ///
    /// Returns `None` once both pipes are closed.
    pub async fn next_line(&mut self) -> Option<LogLine> {
        let line = self.rx.recv().await?;
        let line = LogLine {
            stream: line.stream,
            text: redact_with(&self.secrets, &line.text),
        };
        self.log.push(line.clone());
        Some(line)
    }

    /// Stream the output to `on_line` until the process exits.
    ///
    /// Every line is redacted, appended to the ring buffer and handed to the
    /// callback (which the UI uses to feed its log window).
    pub async fn wait_with<F>(&mut self, mut on_line: F) -> RcResult<GameExit>
    where
        F: FnMut(&LogLine),
    {
        let started = self.started;
        let status = loop {
            // Disjoint field borrows so `select!` can hold both futures.
            // Both branches are cancel-safe (`Child::wait` and
            // `UnboundedReceiver::recv` may be dropped without losing data).
            let child = &mut self.child;
            let rx = &mut self.rx;
            let log = &mut self.log;
            let secrets = &self.secrets;
            tokio::select! {
                // Prefer draining output over reaping: keeps the log complete.
                biased;
                maybe = rx.recv() => match maybe {
                    Some(line) => record(log, secrets, line, &mut on_line),
                    // Pipes closed: the process is finishing, wait for its status.
                    None => break child.wait().await.map_err(spawn_err)?,
                },
                st = child.wait() => {
                    let st = st.map_err(spawn_err)?;
                    // The crash report is printed *last*: keep draining briefly
                    // so the diagnosis sees it.
                    while let Ok(Some(line)) =
                        tokio::time::timeout(DRAIN_AFTER_EXIT, rx.recv()).await
                    {
                        record(log, secrets, line, &mut on_line);
                    }
                    break st;
                }
            }
        };
        Ok(self.finish(status, started))
    }

    /// Wait for the process without observing individual lines.
    pub async fn wait(&mut self) -> RcResult<GameExit> {
        self.wait_with(|_| {}).await
    }

    /// Ask the game to stop, escalating to SIGKILL after `grace`.
    ///
    /// SIGTERM first so Minecraft can save the world and flush its log; a hung
    /// JVM (frozen GL driver, deadlocked mod) is then killed outright.
    pub async fn stop(&mut self, grace: Duration) -> RcResult<()> {
        self.stop_requested = true;
        // Hosts without signals go straight to `kill()`.
        #[cfg(not(unix))]
        let _ = grace;
        #[cfg(unix)]
        if self.pid != 0 {
            // SAFETY: `kill` on our own child pid; an already-reaped pid simply
            // returns ESRCH, which we ignore.
            unsafe { libc::kill(self.pid as libc::pid_t, libc::SIGTERM) };
            if tokio::time::timeout(grace, self.child.wait()).await.is_ok() {
                return Ok(());
            }
        }
        self.kill().await
    }

    /// Kill the process immediately (SIGKILL).
    pub async fn kill(&mut self) -> RcResult<()> {
        self.stop_requested = true;
        match self.child.kill().await {
            Ok(()) => Ok(()),
            // Already exited: nothing to do.
            Err(_) if matches!(self.child.try_wait(), Ok(Some(_))) => Ok(()),
            Err(e) => Err(RcError::Launch(format!("failed to kill the game: {e}"))),
        }
    }

    /// Build the [`GameExit`] for a finished process.
    fn finish(&mut self, status: ExitStatus, started: Instant) -> GameExit {
        let code = status.code();
        let signal = signal_of(&status);
        let log = self.take_log();
        let crash = {
            let lines = log.texts();
            diagnose(code, signal, lines, self.stop_requested)
        };
        GameExit {
            pid: self.pid,
            code,
            signal,
            duration: started.elapsed(),
            log,
            crash,
        }
    }
}

/// The outcome of a finished game process.
#[derive(Debug)]
pub struct GameExit {
    pub pid: u32,
    /// Exit code (`None` when killed by a signal).
    pub code: Option<i32>,
    /// Terminating signal, when any.
    pub signal: Option<i32>,
    /// How long the game ran.
    pub duration: Duration,
    /// The retained log tail.
    pub log: LogBuffer,
    /// The diagnosis (see [`crate::launch::crash`]).
    pub crash: CrashReport,
}

impl GameExit {
    /// Did the game exit cleanly?
    pub fn is_success(&self) -> bool {
        !self.crash.crashed()
    }

    /// A one-line, log-friendly summary.
    pub fn summary(&self) -> String {
        format!(
            "game (pid {}) ran for {:.1}s: {}",
            self.pid,
            self.duration.as_secs_f32(),
            self.crash.summary()
        )
    }

    /// JSON payload for the UI / FFI (task 10).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "pid": self.pid,
            "exit_code": self.code,
            "signal": self.signal,
            "duration_ms": self.duration.as_millis() as u64,
            "success": self.is_success(),
            "log_lines": self.log.len(),
            "log_dropped": self.log.dropped(),
            "crash": self.crash.to_json(),
        })
    }
}

/// Redact, buffer and forward one line.
fn record<F>(log: &mut LogBuffer, secrets: &[String], line: LogLine, on_line: &mut F)
where
    F: FnMut(&LogLine),
{
    let line = LogLine {
        stream: line.stream,
        text: redact_with(secrets, &line.text),
    };
    on_line(&line);
    log.push(line);
}

/// Read `reader` line by line and forward the lines to `tx`.
async fn pump<R>(reader: R, stream: LogStream, tx: UnboundedSender<LogLine>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = BufReader::new(reader);
    let mut raw: Vec<u8> = Vec::with_capacity(256);
    loop {
        raw.clear();
        // Bounded read: a newline-less flood is emitted as several capped lines
        // (the ring buffer then drops the old ones) instead of growing `raw`.
        let mut limited = (&mut buf).take(MAX_LINE_BYTES as u64);
        match limited.read_until(b'\n', &mut raw).await {
            // EOF
            Ok(0) => break,
            Ok(_) => {
                while matches!(raw.last(), Some(b'\n') | Some(b'\r')) {
                    raw.pop();
                }
                // Game output is not guaranteed UTF-8 (native crash dumps).
                let mut text = String::from_utf8_lossy(&raw).into_owned();
                if text.chars().count() > MAX_LINE_CHARS {
                    text = text.chars().take(MAX_LINE_CHARS).collect::<String>() + "… (truncated)";
                }
                if tx.send(LogLine { stream, text }).is_err() {
                    // Receiver gone: nobody is listening any more.
                    break;
                }
            }
            // A broken pipe is normal when the process dies mid-write.
            Err(_) => break,
        }
    }
}

/// Terminating signal of an exit status (Unix only).
#[cfg(unix)]
fn signal_of(status: &ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

/// Windows / other hosts have no signals.
#[cfg(not(unix))]
fn signal_of(_status: &ExitStatus) -> Option<i32> {
    None
}

fn spawn_err(e: std::io::Error) -> RcError {
    RcError::Launch(format!("failed to wait for the game process: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::crash::CrashCategory;

    /// A `/bin/sh -c <script>` spec running inside `dir`.
    fn sh(script: &str, dir: &Path) -> SpawnSpec {
        let mut spec = SpawnSpec::new("/bin/sh", vec!["-c".into(), script.into()]);
        spec.working_dir = dir.to_path_buf();
        spec
    }

    #[tokio::test]
    async fn captures_both_streams_in_order_with_the_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let spec = sh("echo hello; echo boom 1>&2; exit 3", dir.path());
        let mut p = GameProcess::spawn(&spec).unwrap();
        assert!(p.pid() > 0);

        let mut seen: Vec<String> = Vec::new();
        let exit = p.wait_with(|l| seen.push(l.to_string())).await.unwrap();

        assert_eq!(exit.code, Some(3));
        assert_eq!(exit.signal, None);
        assert!(!exit.is_success());
        assert!(seen.contains(&"[stdout] hello".to_string()), "{seen:?}");
        assert!(seen.contains(&"[stderr] boom".to_string()), "{seen:?}");
        assert_eq!(exit.log.len(), 2);
        assert!(exit.log.iter().any(|l| l.is_error() && l.text == "boom"));
        assert!(exit.summary().contains("exit code 3"));
        assert_eq!(exit.to_json()["exit_code"], 3);
    }

    #[tokio::test]
    async fn clean_exit_is_reported_as_success() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = GameProcess::spawn(&sh("echo done", dir.path())).unwrap();
        let exit = p.wait().await.unwrap();
        assert_eq!(exit.code, Some(0));
        assert!(exit.is_success());
        assert_eq!(exit.crash.category, CrashCategory::CleanExit);
        assert!(exit.duration < Duration::from_secs(60));
    }

    #[tokio::test]
    async fn diagnoses_a_crash_from_the_captured_log() {
        let dir = tempfile::tempdir().unwrap();
        let script = "echo '[main/INFO]: Setting user: Steve'; \
                      echo 'Exception in thread \"main\" java.lang.OutOfMemoryError: Java heap space' 1>&2; \
                      exit 1";
        let mut p = GameProcess::spawn(&sh(script, dir.path())).unwrap();
        let exit = p.wait().await.unwrap();
        assert_eq!(exit.code, Some(1));
        assert_eq!(exit.crash.category, CrashCategory::OutOfMemory);
        assert!(!exit.crash.evidence.is_empty());
        assert_eq!(exit.to_json()["crash"]["category"], "out_of_memory");
    }

    #[tokio::test]
    async fn secrets_are_redacted_from_captured_output() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = sh("echo token=super-secret-token-value", dir.path());
        spec.secrets = vec!["super-secret-token-value".to_string()];
        let mut p = GameProcess::spawn(&spec).unwrap();
        let mut seen = String::new();
        let exit = p.wait_with(|l| seen.push_str(&l.text)).await.unwrap();
        assert_eq!(seen, "token=<redacted>");
        assert!(!exit.log.to_text().contains("super-secret-token-value"));
        assert!(exit.log.to_text().contains("<redacted>"));
    }

    #[tokio::test]
    async fn stop_sends_sigterm_and_is_not_reported_as_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        // Traps nothing: SIGTERM kills it immediately.
        let mut p = GameProcess::spawn(&sh("sleep 30", dir.path())).unwrap();
        assert!(p.is_running());
        p.stop(Duration::from_secs(3)).await.unwrap();
        assert!(p.stop_requested());
        let exit = p.wait().await.unwrap();
        #[cfg(unix)]
        assert_eq!(exit.signal, Some(libc::SIGTERM));
        assert_eq!(exit.crash.category, CrashCategory::UserTerminated);
        assert!(exit.crash.terminated_by_user());
    }

    #[tokio::test]
    async fn stop_escalates_to_sigkill_when_sigterm_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        // A JVM with a frozen GL driver behaves like this: SIGTERM is ignored.
        let mut p = GameProcess::spawn(&sh("trap '' TERM; sleep 30", dir.path())).unwrap();
        // Give the shell a moment to install the trap.
        tokio::time::sleep(Duration::from_millis(150)).await;
        p.stop(Duration::from_millis(300)).await.unwrap();
        let exit = p.wait().await.unwrap();
        #[cfg(unix)]
        assert_eq!(exit.signal, Some(libc::SIGKILL));
        // still an intentional stop, not a scary crash
        assert_eq!(exit.crash.category, CrashCategory::UserTerminated);
    }

    #[tokio::test]
    async fn kill_is_idempotent_after_the_process_exited() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = GameProcess::spawn(&sh("exit 0", dir.path())).unwrap();
        let _ = p.wait().await.unwrap();
        // killing an already reaped child must not error
        p.kill().await.unwrap();
        assert!(!p.is_running());
    }

    #[tokio::test]
    async fn environment_is_overlaid_or_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = sh("echo \"$MY_VAR/$PATH_PRESENT\"", dir.path());
        spec.env.insert("MY_VAR".into(), "42".into());
        spec.env.insert("PATH_PRESENT".into(), "yes".into());
        let mut p = GameProcess::spawn(&spec).unwrap();
        let exit = p.wait().await.unwrap();
        assert_eq!(exit.log.texts(), vec!["42/yes"]);

        // clear_env: only what we pass survives
        let mut spec = sh("echo \"[${HOME:-unset}]\"", dir.path());
        spec.clear_env = true;
        let mut p = GameProcess::spawn(&spec).unwrap();
        let exit = p.wait().await.unwrap();
        assert_eq!(exit.log.texts(), vec!["[unset]"]);
    }

    #[tokio::test]
    async fn survives_non_utf8_and_pathologically_long_output() {
        let dir = tempfile::tempdir().unwrap();
        // invalid UTF-8 byte in the middle of a line
        let mut p = GameProcess::spawn(&sh("printf 'a\\377b\\n'", dir.path())).unwrap();
        let exit = p.wait().await.unwrap();
        assert_eq!(exit.log.len(), 1);
        assert!(exit.log.texts()[0].starts_with('a'));

        // a single 100k-character line is truncated, not buffered forever
        let script = format!("printf '%0{}d\\n' 0", MAX_LINE_CHARS + 5000);
        let mut p = GameProcess::spawn(&sh(&script, dir.path())).unwrap();
        let exit = p.wait().await.unwrap();
        let line = exit.log.texts()[0].to_string();
        assert!(line.ends_with("… (truncated)"), "{}", &line[..40]);
        assert!(line.chars().count() <= MAX_LINE_CHARS + 16);
    }

    #[tokio::test]
    async fn a_newline_less_flood_stays_bounded() {
        let dir = tempfile::tempdir().unwrap();
        // 40 KiB with no newline at all: must not grow a single buffer, must not
        // hang, and must still terminate with the process.
        let script = format!("printf '%0{}d' 0", MAX_LINE_BYTES + 8000);
        let mut p = GameProcess::spawn(&sh(&script, dir.path())).unwrap();
        let exit = p.wait().await.unwrap();
        assert_eq!(exit.code, Some(0));
        assert!(
            exit.log.len() >= 2,
            "flood should be split: {}",
            exit.log.len()
        );
        for line in exit.log.iter() {
            assert!(line.text.chars().count() <= MAX_LINE_CHARS + 16);
        }
    }

    #[tokio::test]
    async fn log_ring_buffer_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = sh("for i in 1 2 3 4 5 6 7 8; do echo line$i; done", dir.path());
        spec.log_capacity = 3;
        let mut p = GameProcess::spawn(&spec).unwrap();
        let mut streamed = 0usize;
        let exit = p.wait_with(|_| streamed += 1).await.unwrap();
        // every line was streamed to the UI ...
        assert_eq!(streamed, 8);
        // ... but only the tail is retained
        assert_eq!(exit.log.len(), 3);
        assert_eq!(exit.log.dropped(), 5);
        assert_eq!(exit.log.texts(), vec!["line6", "line7", "line8"]);
        assert!(exit
            .log
            .to_text()
            .starts_with("… 5 earlier line(s) dropped"));
    }

    #[tokio::test]
    async fn output_after_exit_is_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        // The crash report lines are the last thing a dying JVM prints.
        let script = "echo start; echo '---- Minecraft Crash Report ----' 1>&2; \
                      echo 'java.lang.NullPointerException' 1>&2; exit 255";
        let mut p = GameProcess::spawn(&sh(script, dir.path())).unwrap();
        let exit = p.wait().await.unwrap();
        assert_eq!(exit.code, Some(255));
        assert_eq!(exit.log.len(), 3);
        assert_eq!(exit.crash.category, CrashCategory::GameError);
        assert!(exit.crash.exception.is_some());
    }

    #[tokio::test]
    async fn next_line_streams_manually() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = GameProcess::spawn(&sh("echo a; echo b", dir.path())).unwrap();
        assert_eq!(p.next_line().await.unwrap().text, "a");
        assert_eq!(p.next_line().await.unwrap().text, "b");
        assert!(p.next_line().await.is_none());
        let exit = p.wait().await.unwrap();
        // lines read manually are still in the buffer for diagnosis
        assert_eq!(exit.log.texts(), vec!["a", "b"]);
    }

    #[tokio::test]
    async fn rejects_a_missing_program_or_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let spec = sh("true", Path::new("/definitely/not/here"));
        let err = GameProcess::spawn(&spec).unwrap_err();
        assert!(err.to_string().contains("working directory"), "{err}");

        let mut spec = SpawnSpec::new("/no/such/java", vec![]);
        spec.working_dir = dir.path().to_path_buf();
        let err = GameProcess::spawn(&spec).unwrap_err();
        assert!(
            err.to_string().contains("java executable not found"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn exports_the_log_to_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = GameProcess::spawn(&sh("echo exported", dir.path())).unwrap();
        let exit = p.wait().await.unwrap();
        let out = dir.path().join("logs").join("game.log");
        exit.log.write_to_file(&out).unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert_eq!(text, "exported\n");
    }

    #[test]
    fn log_buffer_basics() {
        let mut b = LogBuffer::new(0);
        assert_eq!(b.capacity(), DEFAULT_LOG_CAPACITY);
        assert!(b.is_empty());
        b.push(LogLine::out("a"));
        b.push(LogLine::err("b"));
        assert_eq!(b.len(), 2);
        assert_eq!(b.tail(1)[0].text, "b");
        assert!(b.tail(10).len() == 2);
        assert_eq!(b.dropped(), 0);
        assert_eq!(b.to_text(), "a\nb\n");
        assert!(LogLine::err("x").is_error());
        assert_eq!(LogStream::Stdout.as_str(), "stdout");
        assert_eq!(LogBuffer::default().capacity(), DEFAULT_LOG_CAPACITY);
    }

    #[test]
    fn spawn_spec_from_launch_command() {
        use crate::launch::options::{AccountProfile, LaunchOptions};
        use crate::launch::{Classpath, CommandBuilder};
        use crate::runtime::JavaVersion;

        let mut o = LaunchOptions::new(
            "/data/mc/.minecraft",
            "/data/mc",
            "/data/jre17",
            JavaVersion::Java17,
            AccountProfile::microsoft("Alex", "u", "a-secret-token-value"),
        );
        o.use_cacio = false;
        let mut v = crate::game::ResolvedVersion::default();
        v.id = "1.20.4".into();
        v.main_class = Some("net.minecraft.client.main.Main".into());
        v.minecraft_arguments = Some("--username ${auth_player_name}".into());
        let cp = Classpath {
            entries: vec![PathBuf::from("/data/mc/versions/1.20.4/1.20.4.jar")],
            ..Default::default()
        };
        let cmd = CommandBuilder::new(&o, &v, &cp).build().unwrap();
        let spec = SpawnSpec::from_command(&cmd, 64);
        assert_eq!(spec.program, PathBuf::from("/data/jre17/bin/java"));
        assert_eq!(spec.working_dir, PathBuf::from("/data/mc/.minecraft"));
        assert!(spec.args.contains(&"Alex".to_string()), "{:?}", spec.args);
        assert_eq!(spec.args.last().unwrap(), "720"); // --width/--height appended
        assert_eq!(spec.log_capacity, 64);
        assert_eq!(spec.secrets, vec!["a-secret-token-value".to_string()]);
        assert_eq!(
            spec.env.get("JAVA_HOME").map(String::as_str),
            Some("/data/jre17")
        );
    }
}
