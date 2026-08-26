//! Crash diagnosis (task 7).
//!
//! When the game dies on a phone there is no console to inspect: the user sees
//! the launcher come back and nothing else. FCL solves this with
//! `JVMCrashActivity` + `LogExporter`; this module is the equivalent *analysis*
//! step, and it runs entirely in the core so the Compose UI only has to render a
//! verdict.
//!
//! [`diagnose`] takes the process outcome (exit code / terminating signal) plus
//! the captured log lines and classifies it into a [`CrashCategory`] with:
//!
//! * the log lines that justify the verdict ([`CrashReport::evidence`]) — no
//!   guessing, the user can always see *why*,
//! * any `hs_err_pid*.log` the JVM wrote (we pass `-XX:ErrorFile=` so those land
//!   inside the launcher data dir),
//! * the first Java exception, and
//! * actionable advice in English and Chinese (i18n, task 20, keys off
//!   [`CrashCategory::id`]).
//!
//! Two deliberate design decisions:
//!
//! 1. **A clean exit is never a crash.** Mods log stack traces during a perfectly
//!    healthy session, so a `0` exit with no signal is always [`CrashCategory::CleanExit`].
//! 2. **Rule order is priority order.** The first (most specific) rule that
//!    matches anywhere in the log wins, so `OutOfMemoryError` beats the generic
//!    `Exception in thread "main"` that follows it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Maximum length of a single evidence line kept in a report.
const MAX_EVIDENCE_LEN: usize = 400;
/// Maximum number of evidence lines kept (a crash log can be huge).
const MAX_EVIDENCE_LINES: usize = 12;

/// What killed the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrashCategory {
    /// Exited with code 0 — a normal quit.
    CleanExit,
    /// We (or the user) asked the process to stop.
    UserTerminated,
    /// SIGKILL from outside: Android's low-memory killer or a swipe-away.
    KilledBySystem,
    /// Java heap / native memory exhaustion.
    OutOfMemory,
    /// The class files are newer than the selected JRE (or vice versa).
    UnsupportedJavaVersion,
    /// A native library could not be loaded (`UnsatisfiedLinkError`, `dlopen`).
    MissingNativeLibrary,
    /// The GL/GLES translation layer or driver failed.
    GraphicsFailure,
    /// The JVM itself crashed (SIGSEGV / `hs_err_pid*.log`).
    NativeCrash,
    /// A jar / asset on disk is truncated or corrupt.
    CorruptedFile,
    /// `mainClass` (or a core game class) is not on the classpath.
    MissingMainClass,
    /// The session / access token was rejected.
    AuthenticationFailure,
    /// Out of storage.
    DiskFull,
    /// Filesystem permissions (Android scoped storage, read-only mount).
    PermissionDenied,
    /// A mod / mod loader refused to start (missing dependency, mixin, ...).
    ModLoaderFailure,
    /// The game threw an unhandled exception we cannot classify further.
    GameError,
    /// Crashed, but nothing in the log explains it.
    Unknown,
}
/// Triage severity for a crash, used by the Compose UI to pick a colour, by
/// the auto-remediation logic to decide whether a retry/quick fix is worth
/// attempting, and by the telemetry gate to decide whether a report should be
/// uploaded (task 19). Distinct from [`CrashCategory`] (what broke) - two very
/// different failures can share the same severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrashSeverity {
    /// The game never reached a playable state and needs a config change or a
    /// re-download (wrong JRE, missing native lib, corrupt jar, ...).
    Fatal,
    /// An environment problem the user can usually fix without redownloading
    /// (free disk, re-auth, lower memory, switch renderer).
    Recoverable,
    /// The user deliberately stopped the game - not an error at all.
    UserAction,
    /// Informational; nothing went wrong (clean exit).
    Info,
}

impl CrashSeverity {
    /// Stable id for the FFI / UI (task 10, task 20).
    pub fn id(self) -> &'static str {
        match self {
            CrashSeverity::Fatal => "fatal",
            CrashSeverity::Recoverable => "recoverable",
            CrashSeverity::UserAction => "user_action",
            CrashSeverity::Info => "info",
        }
    }

    /// Whether the launcher should offer an automatic remediation (retry with
    /// different renderer / lower memory / re-verify files) rather than just
    /// showing the verdict.
    pub fn auto_remediable(self) -> bool {
        matches!(self, CrashSeverity::Recoverable)
    }
}

/// Read one crash message out of the compiled-in i18n catalogues (task 20).
///
/// The resource files are the single source of truth for user-facing copy, so a
/// wording fix in `i18n/*.properties` reaches the core *and* the Compose UI with
/// no code change. Falls back to the base (Chinese) catalogue and finally to an
/// empty string, so a missing key can never panic a crash *report*.
fn catalog_str(language: crate::i18n::Language, key: &str) -> &'static str {
    crate::i18n::catalog::lookup_exact(language, key)
        .or_else(|| crate::i18n::catalog::lookup_exact(crate::i18n::Language::BASE, key))
        .unwrap_or("")
}

impl CrashCategory {
    /// Stable id used as the i18n key by the UI (task 20).
    pub fn id(self) -> &'static str {
        match self {
            CrashCategory::CleanExit => "clean_exit",
            CrashCategory::UserTerminated => "user_terminated",
            CrashCategory::KilledBySystem => "killed_by_system",
            CrashCategory::OutOfMemory => "out_of_memory",
            CrashCategory::UnsupportedJavaVersion => "unsupported_java_version",
            CrashCategory::MissingNativeLibrary => "missing_native_library",
            CrashCategory::GraphicsFailure => "graphics_failure",
            CrashCategory::NativeCrash => "native_crash",
            CrashCategory::CorruptedFile => "corrupted_file",
            CrashCategory::MissingMainClass => "missing_main_class",
            CrashCategory::AuthenticationFailure => "authentication_failure",
            CrashCategory::DiskFull => "disk_full",
            CrashCategory::PermissionDenied => "permission_denied",
            CrashCategory::ModLoaderFailure => "mod_loader_failure",
            CrashCategory::GameError => "game_error",
            CrashCategory::Unknown => "unknown",
        }
    }

    /// One-line English description.
    ///
    /// Sourced from the i18n resource files (`i18n/en.properties`, key
    /// `crash.<id>.summary`) so the classifier, the UI and the translators all
    /// read the *same* copy (task 20). Use [`localized_summary`](Self::localized_summary)
    /// for the user's language.
    pub fn summary(self) -> &'static str {
        catalog_str(
            crate::i18n::Language::En,
            &crate::i18n::crash_summary_key(self.id()),
        )
    }

    /// Actionable English advice (i18n key `crash.<id>.advice` in `en.properties`).
    pub fn advice(self) -> &'static str {
        catalog_str(
            crate::i18n::Language::En,
            &crate::i18n::crash_advice_key(self.id()),
        )
    }

    /// Actionable Chinese (zh-CN) advice — the launcher's primary audience
    /// (i18n key `crash.<id>.advice` in the base catalogue `zh-CN.properties`).
    pub fn advice_zh(self) -> &'static str {
        catalog_str(
            crate::i18n::Language::ZhCn,
            &crate::i18n::crash_advice_key(self.id()),
        )
    }

    /// Localised one-line summary via the i18n catalogue (task 20).
    ///
    /// This is a *thin view* over `i18n/<tag>.properties` (keyed off
    /// [`CrashCategory::id`]): the English/Chinese copy in those files is byte
    /// identical to [`summary`](Self::summary) / the zh entries, so a wording
    /// change reaches the classifier and the UI with no code change.
    pub fn localized_summary(self, language: crate::i18n::Language) -> String {
        crate::i18n::t_in(language, &crate::i18n::crash_summary_key(self.id()))
    }

    /// Localised actionable advice via the i18n catalogue (task 20).
    pub fn localized_advice(self, language: crate::i18n::Language) -> String {
        crate::i18n::t_in(language, &crate::i18n::crash_advice_key(self.id()))
    }
    /// How serious this category is, for UI triage and automatic remediation.
    ///
    /// Mirrors FCL's `JVMCrashActivity` severity tiers but is computed in the
    /// core so the Compose UI only has to render it (task 7 / task 18).
    pub fn severity(self) -> CrashSeverity {
        use CrashSeverity::*;
        match self {
            CrashCategory::CleanExit => Info,
            CrashCategory::UserTerminated => UserAction,
            CrashCategory::KilledBySystem => Recoverable,
            CrashCategory::OutOfMemory => Recoverable,
            CrashCategory::UnsupportedJavaVersion => Fatal,
            CrashCategory::MissingNativeLibrary => Fatal,
            CrashCategory::GraphicsFailure => Recoverable,
            CrashCategory::NativeCrash => Fatal,
            CrashCategory::CorruptedFile => Fatal,
            CrashCategory::MissingMainClass => Fatal,
            CrashCategory::AuthenticationFailure => Recoverable,
            CrashCategory::DiskFull => Recoverable,
            CrashCategory::PermissionDenied => Recoverable,
            CrashCategory::ModLoaderFailure => Recoverable,
            CrashCategory::GameError => Recoverable,
            CrashCategory::Unknown => Recoverable,
        }
    }

    /// True for categories the user can usually fix without re-downloading the
    /// version (adjust memory, switch renderer, re-auth, free disk, grant
    /// storage). Used to decide whether to surface a "quick fix" button.
    pub fn is_user_recoverable(self) -> bool {
        matches!(
            self.severity(),
            CrashSeverity::Recoverable | CrashSeverity::UserAction | CrashSeverity::Info
        )
    }
}

/// One classification rule: a category plus the (lowercase) needles that imply it.
#[derive(Debug, Clone, Copy)]
pub struct CrashRule {
    pub category: CrashCategory,
    pub patterns: &'static [&'static str],
}

/// The rule table, **in priority order** (first match wins).
///
/// Every needle is matched case-insensitively as a substring, which is far more
/// robust against version-to-version log churn than anchored regexes.
pub const RULES: &[CrashRule] = &[
    CrashRule {
        category: CrashCategory::OutOfMemory,
        patterns: &[
            "java.lang.outofmemoryerror",
            "could not reserve enough space for object heap",
            "there is insufficient memory for the java runtime environment",
            "native memory allocation (mmap) failed",
            "out of memory: java heap space",
            "gc overhead limit exceeded",
            "direct buffer memory",
            "cannot allocate memory",
            "unable to create new native thread",
            "metaspace",
        ],
    },
    CrashRule {
        category: CrashCategory::UnsupportedJavaVersion,
        patterns: &[
            "unsupportedclassversionerror",
            "unsupported class file major version",
            "has been compiled by a more recent version of the java runtime",
            "unrecognized option:",
            "unrecognized vm option",
            "requires java",
        ],
    },
    CrashRule {
        category: CrashCategory::MissingNativeLibrary,
        patterns: &[
            "unsatisfiedlinkerror",
            "no lwjgl in java.library.path",
            "failed to locate library",
            "dlopen failed",
            "cannot load library",
            "library not found",
            "cannot open shared object file",
            "could not load library",
            "unable to load native library",
            ".so not found",
            "library \\\"lib",
        ],
    },
    CrashRule {
        category: CrashCategory::GraphicsFailure,
        patterns: &[
            "glfw error",
            "failed to create window",
            "pixel format not accelerated",
            "no opengl context",
            "eglmakecurrent",
            "eglinitialize",
            "egl_bad",
            "libgl error",
            "opengl 1.2 or higher",
            "opengl 3.2 or higher",
            "failed to initialize graphics",
            "org.lwjgl.glfw.glfwexception",
            "renderer initialization failed",
            "eglcreatesurface",
            "eglcreatecontext",
            "eglchooseconfig",
            "could not create egl",
            "could not initialize egl",
            "failed to create egl",
            "opengl context creation failed",
            "no egl display",
            "swiftshader",
            "geteglerror",
        ],
    },
    CrashRule {
        category: CrashCategory::NativeCrash,
        patterns: &[
            "a fatal error has been detected by the java runtime environment",
            "sigsegv",
            "sigbus",
            "sigill",
            "sigabrt",
            "exception_access_violation",
            "problematic frame",
            "received signal",
            "fatal signal",
            "fatal error detected",
            "core dumped",
            "unexpected termination",
        ],
    },
    CrashRule {
        category: CrashCategory::CorruptedFile,
        patterns: &[
            "zipexception",
            "invalid or corrupt jarfile",
            "error in opening zip file",
            "unexpected end of zlib input stream",
            "premature end of file",
            "invalid checksum",
        ],
    },
    CrashRule {
        category: CrashCategory::MissingMainClass,
        patterns: &[
            "could not find or load main class",
            "classnotfoundexception: net.minecraft",
            "noclassdeffounderror: net/minecraft/client/main/main",
        ],
    },
    CrashRule {
        category: CrashCategory::AuthenticationFailure,
        patterns: &[
            "invalid session",
            "failed to verify username",
            "authentication failed",
            "invalid access token",
            "the authentication is invalid",
            "insufficientprivilegesexception",
        ],
    },
    CrashRule {
        category: CrashCategory::DiskFull,
        patterns: &["no space left on device", "enospc", "disk quota exceeded"],
    },
    CrashRule {
        category: CrashCategory::PermissionDenied,
        patterns: &["permission denied", "eacces", "read-only file system"],
    },
    CrashRule {
        category: CrashCategory::ModLoaderFailure,
        patterns: &[
            "mixin apply failed",
            "mixinapplyerror",
            "mixintransformererror",
            "loaderexception",
            "modresolutionexception",
            "incompatible mods found",
            "duplicate mods",
            "missing or unsupported mandatory dependencies",
            "failed to load mods",
            "the game will now exit: incompatible",
            "nosuchmethoderror",
            "noclassdeffounderror",
        ],
    },
    CrashRule {
        category: CrashCategory::GameError,
        patterns: &[
            "exception in thread \"main\"",
            "minecraft crash report",
            "unexpected error",
            "reportedexception",
        ],
    },
];

/// Signal numbers we can name (Linux/Android).
fn signal_name(sig: i32) -> Option<&'static str> {
    Some(match sig {
        2 => "SIGINT",
        4 => "SIGILL",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        9 => "SIGKILL",
        11 => "SIGSEGV",
        13 => "SIGPIPE",
        15 => "SIGTERM",
        _ => return None,
    })
}

/// The verdict for one finished game process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashReport {
    pub category: CrashCategory,
    /// Process exit code (`None` when killed by a signal).
    pub exit_code: Option<i32>,
    /// Terminating signal, if any.
    pub signal: Option<i32>,
    /// Human-readable signal name (`SIGSEGV`, ...).
    pub signal_name: Option<String>,
    /// Log lines that justify the verdict.
    pub evidence: Vec<String>,
    /// The first Java exception seen, if any.
    pub exception: Option<String>,
    /// `hs_err_pid*.log` files referenced by the log.
    pub hs_err_files: Vec<PathBuf>,
}

impl CrashReport {
    /// A clean exit report (code 0, nothing to explain).
    pub fn clean() -> Self {
        Self {
            category: CrashCategory::CleanExit,
            exit_code: Some(0),
            signal: None,
            signal_name: None,
            evidence: Vec::new(),
            exception: None,
            hs_err_files: Vec::new(),
        }
    }

    /// Did the process fail (non-zero exit or a signal)?
    pub fn crashed(&self) -> bool {
        self.category != CrashCategory::CleanExit
    }

    /// Did the *launcher* stop it on purpose?
    pub fn terminated_by_user(&self) -> bool {
        self.category == CrashCategory::UserTerminated
    }

    /// A one-line, log-friendly summary.
    pub fn summary(&self) -> String {
        let outcome = match (self.exit_code, &self.signal_name, self.signal) {
            (_, Some(name), _) => format!("killed by {name}"),
            (_, None, Some(sig)) => format!("killed by signal {sig}"),
            (Some(code), _, _) => format!("exit code {code}"),
            (None, None, None) => "unknown outcome".to_string(),
        };
        format!("{} ({})", self.category.summary(), outcome)
    }

    /// Locale-aware JSON payload (task 20).
    ///
    /// Same shape as [`to_json`](Self::to_json) plus a `language` tag and the
    /// `summary_localized` / `advice_localized` fields the Compose UI renders,
    /// so the user reads the verdict in *their* language while the raw English
    /// fields stay in the log / bug report.
    pub fn to_json_in(&self, language: crate::i18n::Language) -> serde_json::Value {
        let mut v = self.to_json();
        if let Some(obj) = v.as_object_mut() {
            obj.insert("language".into(), language.tag().into());
            obj.insert(
                "summary_localized".into(),
                self.category.localized_summary(language).into(),
            );
            obj.insert(
                "advice_localized".into(),
                self.category.localized_advice(language).into(),
            );
        }
        v
    }

    /// JSON payload for the UI / FFI (task 10).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "category": self.category.id(),
            "summary": self.category.summary(),
            "advice": self.category.advice(),
            "advice_zh": self.category.advice_zh(),
            "severity": self.category.severity().id(),
            "user_recoverable": self.category.is_user_recoverable(),
            "auto_remediable": self.category.severity().auto_remediable(),
            "crashed": self.crashed(),
            "exit_code": self.exit_code,
            "signal": self.signal,
            "signal_name": self.signal_name,
            "evidence": self.evidence,
            "exception": self.exception,
            "hs_err_files": self.hs_err_files
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>(),
        })
    }
}

/// Classify a finished process.
///
/// * `code` — exit code, `None` when the process was signalled.
/// * `signal` — terminating signal, when known.
/// * `lines` — the captured log lines (stdout + stderr, in order).
/// * `requested_stop` — true when the launcher asked the process to stop, so a
///   non-zero exit is reported as [`CrashCategory::UserTerminated`] instead of a
///   scary crash.
pub fn diagnose<'a, I>(
    code: Option<i32>,
    signal: Option<i32>,
    lines: I,
    requested_stop: bool,
) -> CrashReport
where
    I: IntoIterator<Item = &'a str>,
{
    let mut report = CrashReport {
        category: CrashCategory::Unknown,
        exit_code: code,
        signal,
        signal_name: signal.and_then(signal_name).map(|s| s.to_string()),
        evidence: Vec::new(),
        exception: None,
        hs_err_files: Vec::new(),
    };

    // Scan the log once: best (lowest) rule index wins.
    let mut best: Option<usize> = None;
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if report.exception.is_none() {
            if let Some(exc) = extract_exception(line, &lower) {
                report.exception = Some(truncate(&exc));
            }
        }
        for hs in extract_hs_err_paths(line) {
            if !report.hs_err_files.contains(&hs) {
                report.hs_err_files.push(hs);
            }
        }
        for (idx, rule) in RULES.iter().enumerate() {
            if rule.patterns.iter().any(|p| lower.contains(p)) {
                if best.is_none_or(|b| idx < b) {
                    best = Some(idx);
                }
                // Compare the *stored* (truncated) form: a repeated 10k-char
                // stack line must not be kept twice.
                let evidence = truncate(line);
                if report.evidence.len() < MAX_EVIDENCE_LINES
                    && !report.evidence.contains(&evidence)
                {
                    report.evidence.push(evidence);
                }
                break;
            }
        }
    }

    // A clean exit is never a crash, no matter what the mods logged.
    let clean = signal.is_none() && code == Some(0);
    report.category = if clean {
        CrashCategory::CleanExit
    } else if requested_stop {
        CrashCategory::UserTerminated
    } else {
        match signal {
            Some(15) | Some(2) => CrashCategory::UserTerminated,
            Some(9) => CrashCategory::KilledBySystem,
            Some(4) | Some(6) | Some(7) | Some(8) | Some(11) => {
                // A native signal is authoritative unless the log names a cause
                // that explains it better (e.g. an OOM before the abort).
                match best.map(|b| RULES[b].category) {
                    Some(CrashCategory::OutOfMemory) => CrashCategory::OutOfMemory,
                    Some(CrashCategory::GraphicsFailure) => CrashCategory::GraphicsFailure,
                    Some(CrashCategory::MissingNativeLibrary) => {
                        CrashCategory::MissingNativeLibrary
                    }
                    _ => CrashCategory::NativeCrash,
                }
            }
            _ => best
                .map(|b| RULES[b].category)
                .unwrap_or(CrashCategory::Unknown),
        }
    };
    if clean {
        // Keep the log evidence out of a "nothing happened" report.
        report.evidence.clear();
        report.exception = None;
    }
    report
}

/// The first Java exception on a line (`java.lang.Foo: msg`, `Caused by: ...`).
fn extract_exception(line: &str, lower: &str) -> Option<String> {
    const MARKERS: &[&str] = &["exception in thread", "caused by:"];
    if MARKERS.iter().any(|m| lower.contains(m)) {
        return Some(line.to_string());
    }
    // `...Error: message` / `...Exception: message` anywhere on the line.
    for token in line.split_whitespace() {
        let name = token.trim_end_matches(':');
        if (name.ends_with("Exception") || name.ends_with("Error")) && name.contains('.') {
            return Some(line.to_string());
        }
    }
    None
}

/// Every `hs_err_pid*.log` path mentioned on a line.
fn extract_hs_err_paths(line: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for token in line.split(|c: char| c.is_whitespace() || c == '"' || c == '\'') {
        let t = token.trim_end_matches(['.', ',', ')']);
        if t.contains("hs_err_pid") && t.ends_with(".log") {
            out.push(PathBuf::from(t));
        }
    }
    out
}

/// Clamp a line to [`MAX_EVIDENCE_LEN`] characters (never splits a char).
fn truncate(s: &str) -> String {
    if s.chars().count() <= MAX_EVIDENCE_LEN {
        return s.to_string();
    }
    let cut: String = s.chars().take(MAX_EVIDENCE_LEN).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diag(code: Option<i32>, signal: Option<i32>, log: &[&str]) -> CrashReport {
        diagnose(code, signal, log.iter().copied(), false)
    }

    #[test]
    fn clean_exit_is_never_a_crash() {
        // even when mods logged scary things during a healthy session
        let r = diag(
            Some(0),
            None,
            &[
                "[main/INFO]: Loading 42 mods",
                "java.lang.NoClassDefFoundError: some/optional/Class",
                "[main/INFO]: Stopping!",
            ],
        );
        assert_eq!(r.category, CrashCategory::CleanExit);
        assert!(!r.crashed());
        assert!(r.evidence.is_empty());
        assert!(r.exception.is_none());
        assert_eq!(r.summary(), "the game exited normally (exit code 0)");
        assert_eq!(CrashReport::clean(), diag(Some(0), None, &[]));
    }

    #[test]
    fn detects_out_of_memory_and_keeps_evidence() {
        let r = diag(
            Some(1),
            None,
            &[
                "[main/INFO]: Setting user: Steve",
                "Exception in thread \"main\" java.lang.OutOfMemoryError: Java heap space",
                "\tat net.minecraft.client.main.Main.main(Main.java:1)",
            ],
        );
        assert_eq!(r.category, CrashCategory::OutOfMemory);
        assert!(r.crashed());
        assert_eq!(r.evidence.len(), 1);
        assert!(r.evidence[0].contains("OutOfMemoryError"));
        assert!(r.exception.as_deref().unwrap().contains("OutOfMemoryError"));
        assert!(r.category.advice().contains("-Xmx"));
        assert!(r.category.advice_zh().contains("内存"));
    }

    #[test]
    fn heap_reservation_failure_is_out_of_memory() {
        let r = diag(
            Some(1),
            None,
            &[
                "Error occurred during initialization of VM",
                "Could not reserve enough space for object heap",
            ],
        );
        assert_eq!(r.category, CrashCategory::OutOfMemory);
    }

    #[test]
    fn rule_priority_beats_log_order() {
        // the generic "Minecraft Crash Report" appears *first* ...
        let r = diag(
            Some(255),
            None,
            &[
                "---- Minecraft Crash Report ----",
                "Description: Initializing game",
                "java.lang.OutOfMemoryError: Metaspace",
            ],
        );
        // ... but the specific OOM rule still wins
        assert_eq!(r.category, CrashCategory::OutOfMemory);
        assert!(r.evidence.len() >= 2);
    }

    #[test]
    fn detects_wrong_java_version() {
        let r = diag(
            Some(1),
            None,
            &[
                "java.lang.UnsupportedClassVersionError: net/minecraft/client/main/Main has been \
               compiled by a more recent version of the Java Runtime (class file version 65.0)",
            ],
        );
        assert_eq!(r.category, CrashCategory::UnsupportedJavaVersion);
        assert!(r.category.advice_zh().contains("Java"));

        let r = diag(Some(1), None, &["Unrecognized VM option 'UseShenandoahGC'"]);
        assert_eq!(r.category, CrashCategory::UnsupportedJavaVersion);
    }

    #[test]
    fn detects_missing_native_library() {
        let r = diag(
            Some(1),
            None,
            &["java.lang.UnsatisfiedLinkError: no lwjgl in java.library.path"],
        );
        assert_eq!(r.category, CrashCategory::MissingNativeLibrary);

        let r = diag(
            Some(1),
            None,
            &["dlopen failed: library \"libgl4es_114.so\" not found"],
        );
        assert_eq!(r.category, CrashCategory::MissingNativeLibrary);
    }

    #[test]
    fn detects_graphics_failures() {
        for line in [
            "GLFW error 65542: EGL: Failed to initialize EGL: Success",
            "Failed to create window",
            "org.lwjgl.glfw.GLFWException: something",
            "libGL error: failed to load driver: zink",
        ] {
            let r = diag(Some(1), None, &[line]);
            assert_eq!(r.category, CrashCategory::GraphicsFailure, "{line}");
        }
    }

    #[test]
    fn native_crash_from_signal_and_hs_err_file() {
        let r = diag(
            None,
            Some(11),
            &[
                "# A fatal error has been detected by the Java Runtime Environment:",
                "#  SIGSEGV (0xb) at pc=0x0000007f, pid=1234, tid=1235",
                "# An error report file with more information is saved as:",
                "# /data/mc/logs/hs_err_pid1234.log",
            ],
        );
        assert_eq!(r.category, CrashCategory::NativeCrash);
        assert_eq!(r.signal, Some(11));
        assert_eq!(r.signal_name.as_deref(), Some("SIGSEGV"));
        assert_eq!(
            r.hs_err_files,
            vec![PathBuf::from("/data/mc/logs/hs_err_pid1234.log")]
        );
        assert!(r.summary().contains("killed by SIGSEGV"));
    }

    #[test]
    fn native_signal_yields_to_a_better_explanation() {
        // the JVM aborted, but the log says why
        let r = diagnose(
            None,
            Some(6),
            [
                "There is insufficient memory for the Java Runtime Environment to continue.",
                "# An error report file ... hs_err_pid77.log",
            ],
            false,
        );
        assert_eq!(r.category, CrashCategory::OutOfMemory);

        // ... and a bare abort stays a native crash
        let r = diag(None, Some(6), &["random noise"]);
        assert_eq!(r.category, CrashCategory::NativeCrash);
    }

    #[test]
    fn distinguishes_system_kill_from_user_stop() {
        assert_eq!(
            diag(None, Some(9), &[]).category,
            CrashCategory::KilledBySystem
        );
        assert_eq!(
            diag(None, Some(15), &[]).category,
            CrashCategory::UserTerminated
        );
        // an explicit launcher stop is never reported as a crash cause
        let r = diagnose(Some(143), None, ["java.lang.OutOfMemoryError"], true);
        assert_eq!(r.category, CrashCategory::UserTerminated);
        assert!(r.terminated_by_user());
        assert!(diag(None, Some(9), &[])
            .category
            .advice_zh()
            .contains("内存"));
    }

    #[test]
    fn detects_mod_loader_failures() {
        for line in [
            "net.fabricmc.loader.impl.FormattedException: ModResolutionException: unmet dependency",
            "Mixin apply failed sodium.mixins.json:MixinFoo",
            "Incompatible mods found!",
            "java.lang.NoSuchMethodError: net.minecraft.Foo.bar()V",
        ] {
            assert_eq!(
                diag(Some(1), None, &[line]).category,
                CrashCategory::ModLoaderFailure,
                "{line}"
            );
        }
    }

    #[test]
    fn detects_corrupt_files_and_missing_main_class() {
        let r = diag(
            Some(1),
            None,
            &["java.util.zip.ZipException: error in opening zip file"],
        );
        assert_eq!(r.category, CrashCategory::CorruptedFile);

        let r = diag(
            Some(1),
            None,
            &["Error: Could not find or load main class net.minecraft.client.main.Main"],
        );
        assert_eq!(r.category, CrashCategory::MissingMainClass);
    }

    #[test]
    fn detects_environment_problems() {
        assert_eq!(
            diag(
                Some(1),
                None,
                &["java.io.IOException: No space left on device"]
            )
            .category,
            CrashCategory::DiskFull
        );
        assert_eq!(
            diag(
                Some(1),
                None,
                &["/data/mc/logs/latest.log: Permission denied"]
            )
            .category,
            CrashCategory::PermissionDenied
        );
        assert_eq!(
            diag(
                Some(1),
                None,
                &["Invalid session (Try restarting your game)"]
            )
            .category,
            CrashCategory::AuthenticationFailure
        );
    }

    #[test]
    fn unknown_when_the_log_explains_nothing() {
        let r = diag(Some(1), None, &["[main/INFO]: bye"]);
        assert_eq!(r.category, CrashCategory::Unknown);
        assert!(r.evidence.is_empty());
        assert!(r.crashed());
        assert!(r.summary().contains("exit code 1"));
    }

    #[test]
    fn evidence_is_capped_deduplicated_and_truncated() {
        let long = format!("java.lang.OutOfMemoryError: {}", "x".repeat(1000));
        let mut log: Vec<String> = vec![long.clone(), long.clone()];
        for i in 0..40 {
            log.push(format!("Mixin apply failed pack{i}.json"));
        }
        let refs: Vec<&str> = log.iter().map(|s| s.as_str()).collect();
        let r = diagnose(Some(1), None, refs, false);
        assert_eq!(r.category, CrashCategory::OutOfMemory);
        assert!(r.evidence.len() <= MAX_EVIDENCE_LINES);
        // duplicates collapsed
        assert_eq!(
            r.evidence
                .iter()
                .filter(|e| e.starts_with("java.lang.OutOfMemoryError"))
                .count(),
            1
        );
        // long line truncated with an ellipsis
        assert!(r.evidence[0].chars().count() <= MAX_EVIDENCE_LEN + 1);
        assert!(r.evidence[0].ends_with('…'));
    }

    #[test]
    fn handles_multibyte_and_empty_logs_without_panicking() {
        let r = diag(
            Some(1),
            None,
            &["", "   ", "内存不足：java.lang.OutOfMemoryError"],
        );
        assert_eq!(r.category, CrashCategory::OutOfMemory);
        let long: String = "中".repeat(1000);
        let r = diagnose(Some(1), None, [long.as_str()], false);
        assert_eq!(r.category, CrashCategory::Unknown);
    }

    #[test]
    fn json_payload_is_complete_and_ids_are_unique() {
        let r = diag(None, Some(11), &["# SIGSEGV", "# /tmp/hs_err_pid5.log"]);
        let j = r.to_json();
        assert_eq!(j["category"], "native_crash");
        assert_eq!(j["crashed"], true);
        assert_eq!(j["signal"], 11);
        assert_eq!(j["signal_name"], "SIGSEGV");
        assert!(j["advice_zh"].as_str().unwrap().contains("虚拟机"));
        assert_eq!(j["hs_err_files"][0], "/tmp/hs_err_pid5.log");
        assert!(!j["evidence"].as_array().unwrap().is_empty());

        // ids/summaries are unique per category (they are i18n keys)
        let all = [
            CrashCategory::CleanExit,
            CrashCategory::UserTerminated,
            CrashCategory::KilledBySystem,
            CrashCategory::OutOfMemory,
            CrashCategory::UnsupportedJavaVersion,
            CrashCategory::MissingNativeLibrary,
            CrashCategory::GraphicsFailure,
            CrashCategory::NativeCrash,
            CrashCategory::CorruptedFile,
            CrashCategory::MissingMainClass,
            CrashCategory::AuthenticationFailure,
            CrashCategory::DiskFull,
            CrashCategory::PermissionDenied,
            CrashCategory::ModLoaderFailure,
            CrashCategory::GameError,
            CrashCategory::Unknown,
        ];
        let mut ids: Vec<&str> = all.iter().map(|c| c.id()).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n);
        for c in all {
            assert!(!c.advice().is_empty() && !c.advice_zh().is_empty());
            // report round-trips through serde (FFI boundary)
            let mut r = CrashReport::clean();
            r.category = c;
            let s = serde_json::to_string(&r).unwrap();
            assert_eq!(serde_json::from_str::<CrashReport>(&s).unwrap(), r);
        }
    }

    #[test]
    fn signal_names_cover_the_common_ones() {
        assert_eq!(signal_name(9), Some("SIGKILL"));
        assert_eq!(signal_name(11), Some("SIGSEGV"));
        assert_eq!(signal_name(64), None);
        let r = diag(None, Some(64), &[]);
        assert!(r.summary().contains("signal 64"));
    }

    #[test]
    fn hs_err_extraction_ignores_unrelated_paths() {
        assert!(extract_hs_err_paths("no crash here /tmp/latest.log").is_empty());
        assert_eq!(
            extract_hs_err_paths("saved as \"/a/hs_err_pid1.log\"."),
            vec![PathBuf::from("/a/hs_err_pid1.log")]
        );
    }
    #[test]
    fn severity_triage_is_stable_and_consistent() {
        let all = [
            CrashCategory::CleanExit,
            CrashCategory::UserTerminated,
            CrashCategory::KilledBySystem,
            CrashCategory::OutOfMemory,
            CrashCategory::UnsupportedJavaVersion,
            CrashCategory::MissingNativeLibrary,
            CrashCategory::GraphicsFailure,
            CrashCategory::NativeCrash,
            CrashCategory::CorruptedFile,
            CrashCategory::MissingMainClass,
            CrashCategory::AuthenticationFailure,
            CrashCategory::DiskFull,
            CrashCategory::PermissionDenied,
            CrashCategory::ModLoaderFailure,
            CrashCategory::GameError,
            CrashCategory::Unknown,
        ];
        // severity id is the *tier* and is intentionally shared across categories,
        // so we only assert it is non-empty and stable per category.
        for c in all {
            assert!(!c.severity().id().is_empty());
            // fatal => not user recoverable
            if c.severity() == CrashSeverity::Fatal {
                assert!(!c.is_user_recoverable(), "{:?}", c);
            }
            // the recoverable flag mirrors severity (auto-remediable or softer)
            let expect = matches!(
                c.severity(),
                CrashSeverity::Recoverable | CrashSeverity::UserAction | CrashSeverity::Info
            );
            assert_eq!(c.is_user_recoverable(), expect, "{:?}", c);
        }
    }

    #[test]
    fn severity_classifies_fatal_vs_recoverable() {
        assert_eq!(CrashCategory::NativeCrash.severity(), CrashSeverity::Fatal);
        assert_eq!(
            CrashCategory::UnsupportedJavaVersion.severity(),
            CrashSeverity::Fatal
        );
        assert_eq!(
            CrashCategory::MissingMainClass.severity(),
            CrashSeverity::Fatal
        );
        assert_eq!(CrashCategory::CleanExit.severity(), CrashSeverity::Info);
        assert_eq!(
            CrashCategory::UserTerminated.severity(),
            CrashSeverity::UserAction
        );
        assert_eq!(
            CrashCategory::OutOfMemory.severity(),
            CrashSeverity::Recoverable
        );
        assert!(CrashCategory::GraphicsFailure.is_user_recoverable());
        assert!(!CrashCategory::NativeCrash.is_user_recoverable());
        assert!(CrashSeverity::Recoverable.auto_remediable());
        assert!(!CrashSeverity::Fatal.auto_remediable());
    }

    #[test]
    fn new_graphics_patterns_map_to_graphics_failure() {
        for msg in [
            "EGLNativeContext: eglCreateContext failed",
            "Could not initialize EGL: eglChooseConfig returned null",
            "OpenGL context creation failed",
            "No EGL Display found",
            "libGL error: swiftshader could not initialise",
            "getEGLError",
        ] {
            let r = diag(Some(1), None, &[msg]);
            assert_eq!(r.category, CrashCategory::GraphicsFailure, "{msg}");
        }
    }

    #[test]
    fn new_oom_and_native_patterns_map_correctly() {
        assert_eq!(
            diag(
                Some(1),
                None,
                &["java.lang.OutOfMemoryError: GC overhead limit exceeded"]
            )
            .category,
            CrashCategory::OutOfMemory
        );
        assert_eq!(
            diag(Some(1), None, &["FATAL SIGNAL 11 (SIGSEGV) at 0x0"]).category,
            CrashCategory::NativeCrash
        );
        assert_eq!(
            diag(
                Some(1),
                None,
                &["java.lang.UnsatisfiedLinkError: dlopen failed: library \"libopenal.so\" not found"]
            )
            .category,
            CrashCategory::MissingNativeLibrary
        );
        assert_eq!(
            diag(Some(1), None, &["cannot allocate memory for thread-local"]).category,
            CrashCategory::OutOfMemory
        );
    }
}
