//! The launch engine (task 7): preflight → command → process → verdict.
//!
//! [`LaunchEngine`] is the single entry point the JNI bridge (task 10) and the
//! Compose UI talk to. It is the Rust equivalent of FCL's
//! `FCLGameLauncher`/`DefaultLauncher` pipeline:
//!
//! ```text
//!  ResolvedVersion (task 4)      LaunchOptions (UI)      JRE home (task 6)
//!            │                          │                      │
//!            └──────────────┬───────────┴──────────────────────-┘
//!                           ▼
//!                 LaunchEngine::prepare()
//!    ┌──────────────────────┴───────────────────────────────────┐
//!    │ 1. validate options (account, heap, absolute paths)      │
//!    │ 2. preflight the JRE (exists + major version matches)    │
//!    │ 3. preflight app_runtime/ (LWJGL + caciocavallo present) │
//!    │ 4. assemble the classpath (+ LWJGL substitution)         │
//!    │ 5. verify every classpath entry exists on disk           │
//!    │ 6. create the runtime directories the JVM writes to      │
//!    │ 7. build the JVM command line (+ env)                    │
//!    └──────────────────────┬───────────────────────────────────┘
//!                           ▼
//!                    PreparedLaunch  ──describe()──▶ log header (redacted)
//!                           ▼
//!             LaunchEngine::spawn() / launch_and_wait()
//!                           ▼
//!         GameProcess ──▶ GameExit { code, signal, log, crash }
//! ```
//!
//! Everything that can fail *before* a JVM is spawned is checked here, with a
//! precise [`RcError`]: on a phone, "the game closed instantly" is unactionable,
//! while "3 classpath entries are missing (re-download the version)" is not.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{RcError, RcResult};
use crate::game::ResolvedVersion;
use crate::launch::awt::AwtBridge;
use crate::launch::classpath::{Classpath, ClasspathBuilder, ClasspathPolicy};
use crate::launch::command::{CommandBuilder, LaunchCommand};
use crate::launch::env::jre_lib_dirs;
use crate::launch::options::LaunchOptions;
use crate::launch::process::{GameExit, GameProcess, LogLine, SpawnSpec};
use crate::launch::render::LwjglNativeBundle;
use crate::launch::runtime_assets::AppRuntime;
use crate::runtime::JavaVersion;

/// Which preflight checks to run before spawning.
///
/// All enabled in production; a caller that only wants the command line (the
/// settings UI previewing arguments, unit tests) uses [`LaunchEngine::dry_run`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreflightChecks {
    /// `<java_home>/bin/java` must exist.
    pub verify_java: bool,
    /// The JRE must be new enough for the version's `javaVersion.majorVersion`.
    pub verify_java_major: bool,
    /// `app_runtime/` must contain the LWJGL / caciocavallo bundles.
    pub verify_app_runtime: bool,
    /// Every classpath entry must exist on disk.
    pub verify_classpath: bool,
    /// Create the directories the JVM writes into.
    pub create_dirs: bool,
}

impl Default for PreflightChecks {
    fn default() -> Self {
        Self {
            verify_java: true,
            verify_java_major: true,
            verify_app_runtime: true,
            verify_classpath: true,
            create_dirs: true,
        }
    }
}

impl PreflightChecks {
    /// No disk access at all (command preview / unit tests).
    pub fn none() -> Self {
        Self {
            verify_java: false,
            verify_java_major: false,
            verify_app_runtime: false,
            verify_classpath: false,
            create_dirs: false,
        }
    }
}

/// A validated, ready-to-spawn launch.
#[derive(Debug, Clone)]
pub struct PreparedLaunch {
    pub version_id: String,
    /// The assembled command line (secrets redacted in `Debug`).
    pub command: LaunchCommand,
    pub classpath: Classpath,
    /// `${natives_directory}` for this launch.
    pub natives_dir: PathBuf,
    /// Non-fatal findings the UI should surface (substituted libraries, dropped
    /// arguments, Java version mismatch, ...).
    pub warnings: Vec<String>,
}

impl PreparedLaunch {
    /// A redacted, human-readable header for the log window / exported log.
    ///
    /// Mirrors what FCL writes at the top of every session log, which is the
    /// first thing anyone needs when diagnosing a report from a user.
    pub fn describe(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "--- {} {} launch report ---\n",
            crate::launch::options::LAUNCHER_NAME,
            crate::VERSION
        ));
        s.push_str(&format!("version:    {}\n", self.version_id));
        s.push_str(&format!("main class: {}\n", self.command.main_class));
        s.push_str(&format!("java:       {}\n", self.command.program.display()));
        s.push_str(&format!(
            "game dir:   {}\n",
            self.command.working_dir.display()
        ));
        s.push_str(&format!("natives:    {}\n", self.natives_dir.display()));
        s.push_str(&format!(
            "classpath:  {} entries ({} substituted, {} collapsed)\n",
            self.classpath.len(),
            self.classpath.substituted.len(),
            self.classpath.collapsed.len()
        ));
        for w in &self.warnings {
            s.push_str(&format!("warning:    {}\n", w));
        }
        s.push_str(&format!("command:    {}\n", self.command.to_shell_string()));
        s
    }

    /// JSON payload for the UI / FFI (task 10) — never contains secrets.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "version_id": self.version_id,
            "program": self.command.program.to_string_lossy(),
            "main_class": self.command.main_class,
            "working_dir": self.command.working_dir.to_string_lossy(),
            "natives_dir": self.natives_dir.to_string_lossy(),
            "jvm_args": self.command.jvm_args.iter().map(|a| self.command.redact(a)).collect::<Vec<_>>(),
            "game_args": self.command.game_args.iter().map(|a| self.command.redact(a)).collect::<Vec<_>>(),
            "env": self.command.env.as_map(),
            "classpath": self.classpath.entries.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
            "substituted": self.classpath.substituted,
            "collapsed": self.classpath.collapsed,
            "warnings": self.warnings,
            "notes": self.command.notes,
            "command_line": self.command.to_shell_string(),
        })
    }
}

/// The launch engine.
#[derive(Debug, Clone)]
pub struct LaunchEngine {
    pub options: LaunchOptions,
    pub checks: PreflightChecks,
    pub classpath_policy: ClasspathPolicy,
}

impl LaunchEngine {
    /// A production engine (all preflight checks on).
    pub fn new(options: LaunchOptions) -> Self {
        Self {
            options,
            checks: PreflightChecks::default(),
            classpath_policy: ClasspathPolicy::default(),
        }
    }

    /// An engine that only builds the command line (no disk access).
    pub fn dry_run(options: LaunchOptions) -> Self {
        Self {
            options,
            checks: PreflightChecks::none(),
            classpath_policy: ClasspathPolicy::default(),
        }
    }

    /// The `app_runtime/` bundle, when configured.
    fn app_runtime(&self) -> Option<AppRuntime> {
        self.options.app_runtime.as_ref().map(AppRuntime::new)
    }

    /// Preflight the JRE: present, and new enough for the version.
    fn preflight_java(
        &self,
        version: &ResolvedVersion,
        warnings: &mut Vec<String>,
    ) -> RcResult<()> {
        let exe = self.options.java_executable();
        if self.checks.verify_java && !exe.is_file() {
            return Err(RcError::MissingFile(format!(
                "java executable not found: {} (install the {} runtime first)",
                exe.display(),
                self.options.java_version.as_jre_dir()
            )));
        }
        let required = version.java_version.as_ref().and_then(|j| j.major_version);
        if let Some(required) = required {
            let have = self.options.java_version.major();
            if have < required {
                let suggestion = JavaVersion::from_major(required)
                    .map(|v| v.as_jre_dir())
                    .unwrap_or("a newer JRE");
                let msg = format!(
                    "version {} requires Java {} but Java {} is selected (install/select {})",
                    version.id, required, have, suggestion
                );
                if self.checks.verify_java_major {
                    return Err(RcError::Launch(msg));
                }
                warnings.push(msg);
            } else if have > required {
                warnings.push(format!(
                    "version {} asks for Java {} but Java {} is selected; older mod loaders may \
                     not support it",
                    version.id, required, have
                ));
            }
        }
        Ok(())
    }

    /// Directories the AWT natives (`libawt.so`, the fake `libawt_xawt.so`, the
    /// frame bridge) are looked for in: the app's `nativeLibraryDir` first, then
    /// the JRE's own `lib/` directories.
    fn awt_native_search_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(nl) = &self.options.native_lib_dir {
            dirs.push(nl.clone());
        }
        dirs.extend(jre_lib_dirs(
            &self.options.java_home,
            self.options.java_version,
            self.options.abi,
        ));
        dirs
    }

    /// Preflight `app_runtime/` (LWJGL + caciocavallo bundles).
    fn preflight_app_runtime(&self, warnings: &mut Vec<String>) -> RcResult<()> {
        match self.app_runtime() {
            Some(rt) => {
                if self.checks.verify_app_runtime {
                    rt.verify(
                        self.options.java_version,
                        self.options.lwjgl_version,
                        self.options.abi,
                    )?;
                    // task 17: the LWJGL natives themselves must be present, not
                    // just the directory — a missing `liblwjgl_opengl.so` would
                    // otherwise crash the JVM with an opaque UnsatisfiedLinkError.
                    LwjglNativeBundle::discover(&rt, self.options.lwjgl_version, self.options.abi)?;
                    // task 18: the AWT/Swing bridge must be complete as well. A
                    // half-extracted caciocavallo bundle only surfaces much
                    // later, as an `AWTError` / `ClassNotFoundException` the
                    // first time something touches Swing (a Forge installer, the
                    // crash dialog, font metrics) — by then the user has already
                    // waited for a full launch.
                    if self.options.use_cacio {
                        let bridge = AwtBridge::discover(
                            self.options.java_version,
                            self.options.window,
                            &rt,
                            &self.awt_native_search_dirs(),
                        )?;
                        warnings.extend(bridge.warnings());
                    }
                }
            }
            None => warnings.push(
                "no app_runtime/ configured: the version's own (desktop) LWJGL will be used, \
                 which cannot load on Android"
                    .to_string(),
            ),
        }
        Ok(())
    }

    /// Assemble the classpath for `version`.
    pub fn build_classpath(&self, version: &ResolvedVersion) -> RcResult<Classpath> {
        let o = &self.options;
        let mut builder = ClasspathBuilder::new(
            o.libraries_dir(),
            o.platform(),
            o.features(),
            o.abi,
            o.java_version,
        );
        builder.policy = self.classpath_policy.clone();
        builder.use_cacio = o.use_cacio;
        if let Some(rt) = self.app_runtime() {
            builder = builder.with_app_runtime(rt, o.lwjgl_version);
        }
        builder.build(version, &o.client_jar_for(&version.id))
    }

    /// Create the directories the JVM writes into.
    ///
    /// The JVM does not create these itself and dies with an opaque error when
    /// they are missing (`-XX:ErrorFile`, `-Djna.tmpdir`, the Mesa shader cache
    /// and the working directory).
    fn create_runtime_dirs(&self, natives_dir: &Path) -> RcResult<()> {
        let o = &self.options;
        for dir in [
            o.game_dir.clone(),
            natives_dir.to_path_buf(),
            o.data_root.join("logs"),
            o.data_root.join("tmp").join("jna"),
            o.data_root.join("cache"),
        ] {
            std::fs::create_dir_all(&dir)
                .map_err(|e| RcError::Launch(format!("cannot create {}: {}", dir.display(), e)))?;
        }
        Ok(())
    }

    /// Run every preflight check and build the command line.
    pub fn prepare(&self, version: &ResolvedVersion) -> RcResult<PreparedLaunch> {
        let o = &self.options;
        o.validate()?;
        let mut warnings: Vec<String> = Vec::new();

        self.preflight_java(version, &mut warnings)?;
        self.preflight_app_runtime(&mut warnings)?;

        let classpath = self.build_classpath(version)?;
        if self.checks.verify_classpath {
            classpath.verify_present()?;
        }
        if !classpath.native_jars.is_empty() {
            warnings.push(format!(
                "{} manifest native jar(s) ignored: the prebuilt Android LWJGL bundle provides \
                 the natives",
                classpath.native_jars.len()
            ));
        }
        if !classpath.collapsed.is_empty() {
            warnings.push(format!(
                "{} duplicate librar(y/ies) collapsed to the highest version",
                classpath.collapsed.len()
            ));
        }

        let natives_dir = o.natives_dir_for(&version.id);
        if self.checks.create_dirs {
            self.create_runtime_dirs(&natives_dir)?;
        }

        let command = CommandBuilder::new(o, version, &classpath).build()?;
        warnings.extend(command.notes.iter().cloned());

        Ok(PreparedLaunch {
            version_id: version.id.clone(),
            command,
            classpath,
            natives_dir,
            warnings,
        })
    }

    /// Spawn the game for an already prepared launch.
    ///
    /// Must be called from within a Tokio runtime.
    pub fn spawn(&self, prepared: &PreparedLaunch) -> RcResult<GameProcess> {
        let spec = SpawnSpec::from_command(&prepared.command, self.options.log_buffer_lines);
        GameProcess::spawn(&spec)
    }

    /// Preflight, build and spawn in one go.
    pub fn launch(&self, version: &ResolvedVersion) -> RcResult<(PreparedLaunch, GameProcess)> {
        let prepared = self.prepare(version)?;
        let process = self.spawn(&prepared)?;
        Ok((prepared, process))
    }

    /// Launch and supervise the game until it exits.
    ///
    /// `on_line` receives the launch header first (as a stdout line, exactly like
    /// FCL prints it into the log window), then every line of game output.
    /// Launch and supervise the game until it exits.
    ///
    /// `on_line` receives the launch header first (as a stdout line, exactly like
    /// FCL prints it into the log window), then every line of game output.
    ///
    /// Every line is also captured into the process-wide log ring
    /// ([`crate::robust::reporter`], task 19) so a crash report always has the
    /// lead-up, and when the game crashes a crash log is persisted + emitted.
    pub async fn launch_and_wait<F>(
        &self,
        version: &ResolvedVersion,
        mut on_line: F,
    ) -> RcResult<GameExit>
    where
        F: FnMut(&LogLine),
    {
        let (prepared, mut process) = self.launch(version)?;
        for line in prepared.describe().lines() {
            on_line(&LogLine::out(line));
        }
        // Capture every game log line into the process-wide ring so a crash
        // report always has the lead-up (task 19).
        let on_line = move |line: &LogLine| {
            crate::robust::reporter::record_log(line.stream.as_str(), &line.text);
            on_line(line);
        };
        let exit = process.wait_with(on_line).await?;
        if exit.crash.crashed() {
            // Best-effort: record + persist + emit the crash (task 19). A write
            // failure must not turn a crash into a second failure.
            let _ = self.report_crash(&exit);
        }
        Ok(exit)
    }

    /// Persist + emit a crash report for a finished (crashed) game, using the
    /// captured log and the diagnosis from [`crate::launch::crash`].
    ///
    /// Best-effort: a write failure simply returns the error (the caller may
    /// ignore it). Writes under `<data_root>/crash/` and pushes an `error` event
    /// onto the global bus so the Compose UI can surface it (task 19).
    pub fn report_crash(&self, exit: &GameExit) -> RcResult<PathBuf> {
        let logs: Vec<crate::robust::reporter::LogEntry> = exit
            .log
            .iter()
            .map(|l| crate::robust::reporter::LogEntry {
                ts: 0,
                level: l.stream.as_str().to_string(),
                line: l.text.clone(),
            })
            .collect();
        let report = crate::robust::reporter::CrashLog::new(
            exit.crash.category.id(),
            exit.crash.category.summary(),
        )
        .with_logs(logs)
        .with_context(serde_json::json!({
            "exit_code": exit.code,
            "signal": exit.signal,
            "evidence": exit.crash.evidence,
            "exception": exit.crash.exception,
        }));
        let dir = self.options.data_root.join("crash");
        crate::robust::reporter::report_crash(&dir, &report)
    }

    /// Stop a running game, escalating SIGTERM → SIGKILL after `grace`.
    pub async fn stop(&self, process: &mut GameProcess, grace: Duration) -> RcResult<()> {
        process.stop(grace).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::version::{JavaVersion as ManifestJava, VersionArguments};
    use crate::launch::options::{AccountProfile, LwjglVersion};
    use crate::launch::LogStream;
    use crate::runtime::Abi;
    use std::fs;
    use std::path::Path;

    /// A realistic on-device tree: `app_runtime/` (LWJGL 3.3.3 + caciocavallo17 +
    /// a fake `jre17`), `libraries/` and `versions/<id>/<id>.jar`, using the file
    /// names catalogued from the real FCL APK.
    ///
    /// `java_script` becomes `app_runtime/java/jre17/bin/java`, so the whole
    /// pipeline can be exercised end-to-end without a real JVM.
    fn install(java_script: &str) -> (tempfile::TempDir, LaunchOptions) {
        let td = tempfile::tempdir().unwrap();
        let root = td.path().to_path_buf();
        let app_runtime = root.join("app_runtime");

        // --- fake JRE ---------------------------------------------------------
        let jre = app_runtime.join("java").join("jre17");
        fs::create_dir_all(jre.join("bin")).unwrap();
        fs::create_dir_all(jre.join("lib").join("server")).unwrap();
        let java = jre.join("bin").join("java");
        fs::write(&java, java_script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&java, fs::Permissions::from_mode(0o755)).unwrap();
        }

        // --- prebuilt LWJGL bundle + natives ---------------------------------
        let lwjgl = app_runtime.join("lwjgl").join("3.3.3");
        fs::create_dir_all(lwjgl.join("natives").join("arm64-v8a")).unwrap();
        for jar in [
            "lwjgl.jar",
            "lwjgl-3.3.3-merged-modules.jar",
            "lwjgl-openal.jar",
        ] {
            fs::write(lwjgl.join(jar), b"jar").unwrap();
        }
        fs::write(
            lwjgl.join("natives").join("arm64-v8a").join("liblwjgl.so"),
            b"so",
        )
        .unwrap();
        fs::write(
            lwjgl
                .join("natives")
                .join("arm64-v8a")
                .join("liblwjgl_opengl.so"),
            b"so",
        )
        .unwrap();

        // --- caciocavallo17 AWT bridge ---------------------------------------
        let cacio = app_runtime.join("caciocavallo17");
        fs::create_dir_all(&cacio).unwrap();
        for jar in [
            "cacio-shared-1.19.1-SNAPSHOT.jar",
            "cacio-tta-1.19.1-SNAPSHOT.jar",
            "cacio-agent.jar",
        ] {
            fs::write(cacio.join(jar), b"jar").unwrap();
        }

        // --- libraries + client jar ------------------------------------------
        let lib = root
            .join("libraries")
            .join("com")
            .join("mojang")
            .join("patchy")
            .join("1.3.9");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("patchy-1.3.9.jar"), b"jar").unwrap();
        let vdir = root.join("versions").join("1.20.4");
        fs::create_dir_all(&vdir).unwrap();
        fs::write(vdir.join("1.20.4.jar"), b"jar").unwrap();
        let game_dir = root.join(".minecraft");
        fs::create_dir_all(&game_dir).unwrap();

        let mut o = LaunchOptions::new(
            &game_dir,
            &root,
            &jre,
            JavaVersion::Java17,
            AccountProfile::offline("Steve", "0-0-0-0"),
        );
        o.app_runtime = Some(app_runtime);
        o.abi = Abi::Arm64V8a;
        o.lwjgl_version = LwjglVersion::V3_3_3;
        o.native_lib_dir = Some(root.join("nativeLibraryDir"));
        (td, o)
    }

    /// A version.json equivalent for 1.20.4 (vanilla-shaped).
    fn version() -> ResolvedVersion {
        let mut v = ResolvedVersion::default();
        v.id = "1.20.4".into();
        v.kind = Some("release".into());
        v.main_class = Some("net.minecraft.client.main.Main".into());
        v.java_version = Some(ManifestJava {
            major_version: Some(17),
            name: Some("java-runtime-gamma".into()),
        });
        v.libraries = serde_json::from_value(serde_json::json!([
            { "name": "com.mojang:patchy:1.3.9" },
            { "name": "org.lwjgl:lwjgl:3.3.3" },
            { "name": "org.lwjgl:lwjgl:3.3.3", "natives": { "linux": "natives-linux" } },
            // a *non*-LWJGL native library: not substituted, so it stays a
            // native jar the engine has to report on (vanilla ships this one)
            { "name": "com.mojang:text2speech:1.11.3", "natives": { "linux": "natives-linux" } }
        ]))
        .unwrap();
        v.arguments = Some(VersionArguments {
            jvm: serde_json::json!([
                "-Djava.library.path=${natives_directory}",
                "-cp",
                "${classpath}"
            ])
            .as_array()
            .unwrap()
            .clone(),
            game: serde_json::json!([
                "--username",
                "${auth_player_name}",
                "--version",
                "${version_name}",
                "--uuid",
                "${auth_uuid}",
                "--accessToken",
                "${auth_access_token}"
            ])
            .as_array()
            .unwrap()
            .clone(),
        });
        v
    }

    const OK_JAVA: &str = "#!/bin/sh\necho \"ARGS: $*\"\necho \"CWD: $(pwd)\"\nexit 0\n";

    #[test]
    fn prepare_assembles_a_launchable_command() {
        let (_td, o) = install(OK_JAVA);
        let engine = LaunchEngine::new(o.clone());
        let p = engine.prepare(&version()).unwrap();

        assert_eq!(p.version_id, "1.20.4");
        assert_eq!(p.command.program, o.java_home.join("bin").join("java"));
        // vanilla lib + prebuilt LWJGL + cacio jars + client jar (LWJGL dropped)
        let cp: Vec<String> = p
            .classpath
            .entries
            .iter()
            .map(|e| e.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(cp.contains(&"patchy-1.3.9.jar".to_string()), "{cp:?}");
        assert!(cp.contains(&"lwjgl.jar".to_string()), "{cp:?}");
        assert!(
            cp.contains(&"cacio-shared-1.19.1-SNAPSHOT.jar".to_string()),
            "{cp:?}"
        );
        assert_eq!(cp.last().unwrap(), "1.20.4.jar");
        assert_eq!(
            p.classpath.substituted,
            vec![
                "org.lwjgl:lwjgl:3.3.3".to_string(),
                "org.lwjgl:lwjgl:3.3.3".to_string()
            ],
            "both the LWJGL jar and its natives entry are replaced by the bundle"
        );
        // the cacio agent was found, so no warning about it
        assert!(p.command.jvm_args.iter().any(|a| a.contains("-javaagent:")));
        assert!(
            !p.warnings.iter().any(|w| w.contains("cacio-agent.jar")),
            "{:?}",
            p.warnings
        );
        // native jars of the manifest are ignored in favour of the bundle
        assert!(
            p.warnings.iter().any(|w| w.contains("native jar")),
            "{:?}",
            p.warnings
        );
        p.command.validate().unwrap();
    }

    #[test]
    fn prepare_creates_the_directories_the_jvm_writes_to() {
        let (_td, o) = install(OK_JAVA);
        let engine = LaunchEngine::new(o.clone());
        let p = engine.prepare(&version()).unwrap();
        assert!(p.natives_dir.is_dir(), "{}", p.natives_dir.display());
        assert!(o.data_root.join("logs").is_dir());
        assert!(o.data_root.join("tmp").join("jna").is_dir());
        assert!(o.data_root.join("cache").is_dir());
    }

    #[test]
    fn describe_and_json_are_informative_and_redacted() {
        let (_td, mut o) = install(OK_JAVA);
        o.account = AccountProfile::microsoft("Alex", "uuid-1", "super-secret-token-value");
        let p = LaunchEngine::new(o).prepare(&version()).unwrap();

        let text = p.describe();
        assert!(text.contains("launch report"));
        assert!(text.contains("version:    1.20.4"));
        assert!(text.contains("main class: net.minecraft.client.main.Main"));
        assert!(text.contains("classpath:"));
        assert!(!text.contains("super-secret-token-value"), "{text}");
        assert!(text.contains("<redacted>"));

        let j = p.to_json();
        assert_eq!(j["version_id"], "1.20.4");
        assert_eq!(j["main_class"], "net.minecraft.client.main.Main");
        assert!(!j.to_string().contains("super-secret-token-value"));
        assert!(j["classpath"].as_array().unwrap().len() >= 3);
        assert!(j["env"]["JAVA_HOME"].is_string());
        assert!(j["command_line"].as_str().unwrap().contains("java"));
    }

    #[test]
    fn missing_java_is_reported_before_spawning() {
        let (_td, mut o) = install(OK_JAVA);
        o.java_home = o.data_root.join("no-such-jre");
        let err = LaunchEngine::new(o).prepare(&version()).unwrap_err();
        assert!(
            err.to_string().contains("java executable not found"),
            "{err}"
        );
    }

    #[test]
    fn java_major_version_mismatch_is_fatal_but_can_be_downgraded() {
        let (_td, o) = install(OK_JAVA);
        let mut v = version();
        v.java_version = Some(ManifestJava {
            major_version: Some(21),
            name: None,
        });
        let err = LaunchEngine::new(o.clone()).prepare(&v).unwrap_err();
        assert!(err.to_string().contains("requires Java 21"), "{err}");
        assert!(err.to_string().contains("jre21"), "{err}");

        // ... and only a warning when the caller opted out of the check
        let mut engine = LaunchEngine::new(o.clone());
        engine.checks.verify_java_major = false;
        let p = engine.prepare(&v).unwrap();
        assert!(p.warnings.iter().any(|w| w.contains("requires Java 21")));

        // a *newer* JRE than requested is only a warning
        let mut v8 = version();
        v8.java_version = Some(ManifestJava {
            major_version: Some(8),
            name: None,
        });
        let p = LaunchEngine::new(o).prepare(&v8).unwrap();
        assert!(
            p.warnings.iter().any(|w| w.contains("asks for Java 8")),
            "{:?}",
            p.warnings
        );
    }

    #[test]
    fn missing_classpath_entry_is_reported_with_the_path() {
        let (_td, o) = install(OK_JAVA);
        fs::remove_file(
            o.data_root
                .join("versions")
                .join("1.20.4")
                .join("1.20.4.jar"),
        )
        .unwrap();
        let err = LaunchEngine::new(o.clone())
            .prepare(&version())
            .unwrap_err();
        assert!(err.to_string().contains("1.20.4.jar"), "{err}");
        assert!(err.to_string().contains("re-download"), "{err}");

        // the check can be skipped (offline command preview)
        let mut engine = LaunchEngine::new(o);
        engine.checks.verify_classpath = false;
        assert!(engine.prepare(&version()).is_ok());
    }

    #[test]
    fn app_runtime_problems_are_reported() {
        // broken bundle: no LWJGL jars
        let (_td, o) = install(OK_JAVA);
        let lwjgl = o.app_runtime.clone().unwrap().join("lwjgl").join("3.3.3");
        for e in fs::read_dir(&lwjgl).unwrap() {
            let p = e.unwrap().path();
            if p.is_file() {
                fs::remove_file(p).unwrap();
            }
        }
        let err = LaunchEngine::new(o.clone())
            .prepare(&version())
            .unwrap_err();
        assert!(err.to_string().contains("no LWJGL jars"), "{err}");

        // no bundle configured at all: a warning, because the desktop LWJGL of
        // the manifest cannot load on Android
        let (_td2, mut o2) = install(OK_JAVA);
        o2.app_runtime = None;
        let mut engine = LaunchEngine::new(o2);
        engine.checks.verify_classpath = false;
        let p = engine.prepare(&version()).unwrap();
        assert!(
            p.warnings.iter().any(|w| w.contains("no app_runtime/")),
            "{:?}",
            p.warnings
        );
    }

    #[test]
    fn dry_run_never_touches_the_disk() {
        let o = LaunchOptions::new(
            "/nonexistent/.minecraft",
            "/nonexistent",
            "/nonexistent/jre17",
            JavaVersion::Java17,
            AccountProfile::offline("Steve", "u"),
        );
        let engine = LaunchEngine::dry_run(o);
        let p = engine.prepare(&version()).unwrap();
        assert!(p.command.jvm_args.iter().any(|a| a == "-cp"));
        assert!(!Path::new("/nonexistent").exists());
        assert!(!PreflightChecks::none().verify_java);
    }

    // ---- end-to-end: really spawn the "JVM" -------------------------------

    #[cfg(unix)]
    #[tokio::test]
    async fn launches_supervises_and_reports_a_clean_exit() {
        let (_td, o) = install(OK_JAVA);
        let engine = LaunchEngine::new(o.clone());
        let mut lines: Vec<String> = Vec::new();
        let exit = engine
            .launch_and_wait(&version(), |l| lines.push(l.text.clone()))
            .await
            .unwrap();

        assert!(exit.is_success(), "{}", exit.summary());
        assert_eq!(exit.code, Some(0));
        // the launch header is streamed first ...
        assert!(lines[0].contains("launch report"), "{:?}", &lines[..3]);
        // ... then the game output: our fake JVM echoed the real argv
        let out = lines.join("\n");
        assert!(out.contains("ARGS: "), "{out}");
        assert!(out.contains("net.minecraft.client.main.Main"), "{out}");
        assert!(out.contains("--username Steve"), "{out}");
        assert!(out.contains("-Xmx1024M"), "{out}");
        // ... and it ran in the instance directory
        assert!(
            out.contains(&format!("CWD: {}", o.game_dir.display())),
            "{out}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn diagnoses_a_crashing_game_end_to_end() {
        let script = "#!/bin/sh\n\
             echo '[main/INFO]: Setting user: Steve'\n\
             echo 'Exception in thread \"main\" java.lang.OutOfMemoryError: Java heap space' 1>&2\n\
             exit 1\n";
        let (_td, o) = install(script);
        let engine = LaunchEngine::new(o.clone());
        let (prepared, mut process) = engine.launch(&version()).unwrap();
        assert!(process.pid() > 0);
        let exit = process.wait().await.unwrap();

        assert!(!exit.is_success());
        assert_eq!(exit.code, Some(1));
        assert_eq!(
            exit.crash.category,
            crate::launch::CrashCategory::OutOfMemory
        );
        assert!(exit.crash.evidence[0].contains("OutOfMemoryError"));
        assert!(exit.crash.category.advice_zh().contains("内存"));
        assert!(exit.log.iter().any(|l| l.stream == LogStream::Stderr));

        // the exported log is what a bug report should carry
        let log_file = o.data_root.join("logs").join("latest.log");
        exit.log.write_to_file(&log_file).unwrap();
        let text = fs::read_to_string(&log_file).unwrap();
        assert!(text.contains("OutOfMemoryError"));
        assert!(prepared.describe().contains("1.20.4"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stopping_a_running_game_is_not_a_crash() {
        let (_td, o) = install("#!/bin/sh\nsleep 30\n");
        let engine = LaunchEngine::new(o);
        let (_prepared, mut process) = engine.launch(&version()).unwrap();
        assert!(process.is_running());
        engine
            .stop(&mut process, Duration::from_secs(3))
            .await
            .unwrap();
        let exit = process.wait().await.unwrap();
        assert!(exit.crash.terminated_by_user());
        assert!(!exit.is_success());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_non_executable_java_fails_with_a_clear_error() {
        let (_td, o) = install(OK_JAVA);
        let java = o.java_home.join("bin").join("java");
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&java, fs::Permissions::from_mode(0o644)).unwrap();
        }
        let engine = LaunchEngine::new(o);
        let prepared = engine.prepare(&version()).unwrap();
        let err = engine.spawn(&prepared).unwrap_err();
        assert!(err.to_string().contains("failed to spawn"), "{err}");
    }
}
