//! Command assembly: a resolved version + options -> a concrete JVM command
//! line (task 7).
//!
//! This is the heart of the launch engine and the Rust counterpart of FCLCore's
//! `DefaultLauncher.java` plus `FCLauncher`/`jre_launcher.c`: it decides the
//! exact argument vector the game JVM is started with.
//!
//! ```text
//! <java_home>/bin/java
//!   <base jvm args>        heap, GC, encoding, java.library.path, LWJGL/renderer,
//!                          caciocavallo AWT bridge, launcher brand, error file
//!   <manifest jvm args>    version.json `arguments.jvm` (rule-filtered, templated)
//!   -cp <classpath>        assembled by `launch::classpath` (always ours)
//!   <main class>           version.json `mainClass`
//!   <game args>            `arguments.game` / legacy `minecraftArguments`
//!                          + window / server / quick-play / demo extras
//! ```
//!
//! Design rules (robustness first — a wrong command line means an unhelpful JVM
//! crash on a phone, where there is no console to debug it):
//!
//! * **Never emit an unresolved `${...}`** — [`crate::launch::args::prune_unresolved`]
//!   drops such arguments (with their flag) and every drop is recorded in
//!   [`LaunchCommand::notes`] so the UI can explain what happened.
//! * **The launcher owns the classpath and the native search path.** A vanilla
//!   manifest asks for `-Djava.library.path=${natives_directory}` and its own
//!   `-cp`; on Android the JVM also needs the prebuilt LWJGL natives, the app's
//!   `nativeLibraryDir` and the JRE's own `lib/` dirs, so those manifest
//!   arguments are dropped in favour of ours.
//! * **User arguments win.** [`LaunchOptions::extra_jvm_args`] /
//!   [`LaunchOptions::extra_game_args`] are appended last (HotSpot honours the
//!   last `-Xmx` / `-D`), except for a user `-cp`, which would silently break
//!   the launch and is therefore dropped with a note.
//! * **Secrets never reach a log.** [`LaunchCommand`] carries the strings that
//!   must be redacted and its [`std::fmt::Debug`] / [`LaunchCommand::to_shell_string`]
//!   redact them.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::{RcError, RcResult};
use crate::game::ResolvedVersion;
use crate::launch::args::{
    flatten_arguments, has_flag, prune_unresolved, split_legacy_arguments, Substitutions,
};
use crate::launch::awt::{AwtBridge, AwtTransport, CacioBundle};
use crate::launch::classpath::Classpath;
use crate::launch::env::{build_env, join_paths, library_path, LaunchEnv, PATH_SEP};
use crate::launch::options::{path_str as path, LaunchOptions, QuickPlay};
use crate::launch::runtime_assets::AppRuntime;
use crate::runtime::JavaVersion;

// The AWT-bridge constants now live with the bridge itself (task 18,
// `launch::awt`); they stay re-exported here because the command line is where
// they are consumed.
pub use crate::launch::awt::{
    CACIO17_GRAPHICS_ENV, CACIO17_MODULE_FLAGS, CACIO17_TOOLKIT, CACIO8_GRAPHICS_ENV,
    CACIO8_TOOLKIT,
};

/// Manifest JVM arguments the launcher always replaces with its own value.
const REPLACED_JVM_PREFIXES: &[&str] = &[
    // Ours also contains the LWJGL / renderer / JRE / system native dirs.
    "-Djava.library.path=",
    // Windows-only heap dump path (`MojangTricksIntelDriversForPerformance`).
    "-XX:HeapDumpPath=",
];

/// Flags that take the classpath; the launcher always supplies its own.
const CLASSPATH_FLAGS: &[&str] = &["-cp", "-classpath", "--class-path"];

/// A fully assembled, ready-to-spawn game command line.
#[derive(Clone, PartialEq, Eq)]
pub struct LaunchCommand {
    /// The JVM binary (`<java_home>/bin/java`).
    pub program: PathBuf,
    /// JVM arguments, `-cp <classpath>` included (always last of the JVM block).
    pub jvm_args: Vec<String>,
    /// `mainClass` of the resolved version.
    pub main_class: String,
    /// Game arguments (everything after the main class).
    pub game_args: Vec<String>,
    /// Working directory of the process (`${game_directory}`).
    pub working_dir: PathBuf,
    /// Environment handed to the process (see [`crate::launch::env`]).
    pub env: LaunchEnv,
    /// Version id this command launches.
    pub version_id: String,
    /// Diagnostics: arguments dropped / replaced and why.
    pub notes: Vec<String>,
    /// Strings that must be redacted from logs / crash reports.
    secrets: Vec<String>,
}

impl LaunchCommand {
    /// The complete argument vector after `program`.
    pub fn args(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.jvm_args.len() + 1 + self.game_args.len());
        out.extend(self.jvm_args.iter().cloned());
        out.push(self.main_class.clone());
        out.extend(self.game_args.iter().cloned());
        out
    }

    /// Strings that must never appear in a log.
    pub fn secrets(&self) -> &[String] {
        &self.secrets
    }

    /// Replace every secret in `s` with `<redacted>`.
    pub fn redact(&self, s: &str) -> String {
        redact_with(&self.secrets, s)
    }

    /// A copy-pasteable, **redacted** shell command line (for the log header).
    pub fn to_shell_string(&self) -> String {
        let mut parts = Vec::with_capacity(self.jvm_args.len() + self.game_args.len() + 2);
        parts.push(shell_quote(&self.program.to_string_lossy()));
        for a in self.args() {
            parts.push(shell_quote(&self.redact(&a)));
        }
        parts.join(" ")
    }

    /// Sanity-check the assembled command (cheap, no disk access).
    pub fn validate(&self) -> RcResult<()> {
        if self.main_class.trim().is_empty() {
            return Err(RcError::Launch("main class is empty".into()));
        }
        if !has_classpath_flag(&self.jvm_args) {
            return Err(RcError::Launch("command has no -cp".into()));
        }
        for a in self.args() {
            if a.contains("${") {
                return Err(RcError::Launch(format!(
                    "unresolved placeholder in argument: {a}"
                )));
            }
        }
        Ok(())
    }
}

impl fmt::Debug for LaunchCommand {
    /// Redacts secrets so a `{:?}` of the command is safe to log.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LaunchCommand")
            .field("program", &self.program)
            .field("version_id", &self.version_id)
            .field(
                "jvm_args",
                &self
                    .jvm_args
                    .iter()
                    .map(|a| self.redact(a))
                    .collect::<Vec<_>>(),
            )
            .field("main_class", &self.main_class)
            .field(
                "game_args",
                &self
                    .game_args
                    .iter()
                    .map(|a| self.redact(a))
                    .collect::<Vec<_>>(),
            )
            .field("working_dir", &self.working_dir)
            .field("env", &self.env.len())
            .field("notes", &self.notes)
            .finish()
    }
}

/// Replace every non-empty entry of `secrets` inside `s` with `<redacted>`.
pub fn redact_with(secrets: &[String], s: &str) -> String {
    let mut out = s.to_string();
    for secret in secrets {
        if secret.len() >= 8 && out.contains(secret.as_str()) {
            out = out.replace(secret.as_str(), "<redacted>");
        }
    }
    out
}

/// Quote `s` for a POSIX shell when it contains anything non-trivial.
fn shell_quote(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-+=/:,@%".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// Does `args` already carry a classpath flag?
fn has_classpath_flag(args: &[String]) -> bool {
    args.iter().any(|a| CLASSPATH_FLAGS.contains(&a.as_str()))
}

/// The `-Dkey` part of a `-Dkey=value` argument.
fn property_key(arg: &str) -> Option<&str> {
    if !arg.starts_with("-D") {
        return None;
    }
    Some(match arg.find('=') {
        Some(i) => &arg[..i],
        None => arg,
    })
}

/// Builds a [`LaunchCommand`] from a resolved version.
#[derive(Debug, Clone)]
pub struct CommandBuilder<'a> {
    pub options: &'a LaunchOptions,
    pub version: &'a ResolvedVersion,
    pub classpath: &'a Classpath,
}

impl<'a> CommandBuilder<'a> {
    /// A builder for `version` with `options` and an assembled `classpath`.
    pub fn new(
        options: &'a LaunchOptions,
        version: &'a ResolvedVersion,
        classpath: &'a Classpath,
    ) -> Self {
        Self {
            options,
            version,
            classpath,
        }
    }

    /// The `app_runtime/` bundle, when the options point at one.
    fn app_runtime(&self) -> Option<AppRuntime> {
        self.options.app_runtime.as_ref().map(AppRuntime::new)
    }

    /// The task-18 AWT/Swing bridge for this launch.
    ///
    /// `use_cacio == false` yields a *headless* bridge (`-Djava.awt.headless=true`),
    /// which is the right choice for modern vanilla versions: no AWT window is
    /// ever created, so nothing has to be emulated.
    pub fn awt_bridge(&self) -> AwtBridge {
        let o = &self.options;
        if !o.use_cacio {
            return AwtBridge::headless();
        }
        let mut bridge = AwtBridge::for_java(o.java_version, o.window);
        if let Some(rt) = self.app_runtime() {
            let bundle = CacioBundle::scan(&rt, bridge.backend);
            bridge = bridge.with_bundle(bundle);
        }
        // A live session: tell the JVM-side bridge where the channels are. Only
        // when the launcher actually hosts them (`awt_transport_dir`), because a
        // FIFO with no reader would block the game's first repaint.
        if let Some(dir) = &o.awt_transport_dir {
            bridge = bridge.with_transport(AwtTransport::in_dir(dir));
        }
        bridge
    }

    /// The placeholder table for `${...}` expansion.
    ///
    /// Deliberately *incomplete*: placeholders we cannot fill (an offline
    /// account has no `${auth_xuid}` / `${clientid}`, a non-quick-play launch has
    /// no `${quickPlayPath}`) are left undefined so
    /// [`prune_unresolved`] drops the argument instead of handing the game a
    /// literal `${...}`.
    pub fn substitutions(&self, classpath: &str, natives_dir: &Path) -> Substitutions {
        let o = self.options;
        let v = self.version;
        let mut s = Substitutions::new();

        // --- identity -------------------------------------------------------
        s.set("auth_player_name", &o.account.username)
            .set("auth_uuid", &o.account.uuid)
            .set("auth_access_token", &o.account.access_token)
            // Legacy (MC <= 1.5) session token format.
            .set(
                "auth_session",
                format!("token:{}:{}", o.account.access_token, o.account.uuid),
            )
            .set("user_type", o.account.user_type.as_str())
            .set("user_properties", &o.account.user_properties)
            .set("profile_name", &o.account.username);
        if let Some(xuid) = &o.account.xuid {
            s.set("auth_xuid", xuid);
        }
        if let Some(cid) = &o.account.client_id {
            s.set("clientid", cid);
        }

        // --- version / launcher --------------------------------------------
        s.set("version_name", &v.id)
            .set(
                "version_type",
                v.kind.clone().unwrap_or_else(|| o.launcher_name.clone()),
            )
            .set("launcher_name", &o.launcher_name)
            .set("launcher_version", &o.launcher_version);

        // --- paths ----------------------------------------------------------
        let assets_root = o.assets_dir();
        let assets_index = v
            .asset_index
            .as_ref()
            .map(|a| a.id.clone())
            .or_else(|| v.assets.clone())
            .unwrap_or_else(|| "legacy".to_string());
        s.set("game_directory", path(&o.game_dir))
            .set("assets_root", path(&assets_root))
            .set("assets_index_name", &assets_index)
            // Pre-1.6 versions read loose assets from ${game_assets}.
            .set(
                "game_assets",
                path(&assets_root.join("virtual").join(&assets_index)),
            )
            .set("natives_directory", path(natives_dir))
            .set("classpath", classpath)
            .set("classpath_separator", PATH_SEP)
            // Forge / NeoForge read the library root from a property.
            .set("library_directory", path(&o.libraries_dir()))
            .set("primary_jar", path(&o.client_jar_for(&v.id)))
            .set("primary_jar_name", format!("{}.jar", v.id));

        // --- window ---------------------------------------------------------
        s.set("resolution_width", o.window.width.to_string())
            .set("resolution_height", o.window.height.to_string());

        // --- quick play (MC 1.20+) -----------------------------------------
        match &o.quick_play {
            QuickPlay::None => {}
            QuickPlay::Singleplayer { world } => {
                s.set("quickPlaySingleplayer", world);
            }
            QuickPlay::Multiplayer { address } => {
                s.set("quickPlayMultiplayer", address);
            }
            QuickPlay::Realms { id } => {
                s.set("quickPlayRealms", id);
            }
        }
        if !matches!(o.quick_play, QuickPlay::None) {
            // Quick-play log written by the game (must be inside the instance).
            s.set(
                "quickPlayPath",
                path(&o.game_dir.join("quickPlay").join("log.json")),
            );
        }
        s
    }

    /// The base JVM arguments the launcher always passes.
    ///
    /// Mirrors what FCL's `FCLauncher` sets up before `JNI_CreateJavaVM`, with
    /// the additions a *diagnosable* launch needs (`-XX:ErrorFile`,
    /// `-XX:-OmitStackTraceInFastThrow`).
    pub fn base_jvm_args(&self, java_library_path: &str, notes: &mut Vec<String>) -> Vec<String> {
        let o = self.options;
        let java = o.java_version;
        let mut a: Vec<String> = Vec::new();

        // --- heap & GC ------------------------------------------------------
        a.extend(o.memory.to_args());
        // G1 with a short pause target keeps frame pacing smooth on phones.
        a.push("-XX:+UseG1GC".into());
        a.push("-XX:MaxGCPauseMillis=50".into());
        // Full stack traces: the crash diagnosis (see `launch::crash`) is only as
        // good as the exception text the JVM prints.
        a.push("-XX:-OmitStackTraceInFastThrow".into());
        if let Some(n) = o.processor_count {
            // big.LITTLE phones report 8 cores but can sustain far fewer; the
            // JVM sizes its GC / JIT thread pools from this.
            a.push(format!("-XX:ActiveProcessorCount={}", n.max(1)));
        }
        // A native JVM crash must land somewhere we can read back.
        a.push(format!(
            "-XX:ErrorFile={}",
            path(&o.data_root.join("logs").join("hs_err_pid%p.log"))
        ));

        // --- encoding (Chinese paths / chat must not mojibake) --------------
        a.push("-Dfile.encoding=UTF-8".into());
        if java == JavaVersion::Java8 {
            a.push("-Dsun.stdout.encoding=UTF-8".into());
            a.push("-Dsun.stderr.encoding=UTF-8".into());
            a.push("-Dsun.jnu.encoding=UTF-8".into());
        } else {
            a.push("-Dstdout.encoding=UTF-8".into());
            a.push("-Dstderr.encoding=UTF-8".into());
        }

        // --- launcher identity ---------------------------------------------
        a.push(format!("-Dminecraft.launcher.brand={}", o.launcher_name));
        a.push(format!(
            "-Dminecraft.launcher.version={}",
            o.launcher_version
        ));

        // --- native / library paths ----------------------------------------
        a.push(format!("-Djava.library.path={}", java_library_path));
        a.push(format!(
            "-Djna.tmpdir={}",
            path(&o.data_root.join("tmp").join("jna"))
        ));
        if let Some(nl) = &o.native_lib_dir {
            // JNA must load its dispatcher from the app's nativeLibraryDir: the
            // bundled desktop build inside jna.jar cannot run on Android.
            a.push(format!("-Djna.boot.library.path={}", path(nl)));
            a.push("-Djna.nosys=false".into());
        }
        a.push(format!("-Duser.home={}", path(&o.game_dir)));

        // --- LWJGL / renderer ----------------------------------------------
        a.push(format!(
            "-Dorg.lwjgl.opengl.libname={}",
            o.renderer.gl_libname()
        ));
        a.push("-Dorg.lwjgl.vulkan.libname=libvulkan.so".into());
        // LWJGL's bundled jemalloc is not built for Android; use bionic malloc.
        a.push("-Dorg.lwjgl.system.allocator=system".into());
        // Keep LWJGL's parameter checks on: GL misuse then raises a Java
        // exception we can classify instead of a SIGSEGV we can only guess at.
        a.push("-Dorg.lwjgl.util.NoChecks=false".into());

        // --- mod loader quirks ---------------------------------------------
        // Forge's early splash screen needs a desktop AWT window: never on Android.
        a.push("-Dfml.earlyprogresswindow=false".into());
        // Fabric loader must not fork a GUI error window.
        a.push("-Dloader.disable_forked_guis=true".into());
        // Log4Shell mitigation for the many old versions still in the wild.
        a.push("-Dlog4j2.formatMsgNoLookups=true".into());

        // --- AWT / Swing compatibility layer (task 18) ----------------------
        // `launch::awt` owns the whole bridge: which caciocavallo backend the
        // Java version needs, whether its jars are on disk, the boot classpath /
        // java agent, the module flags and the font plumbing. Anything missing
        // becomes a note instead of a hard failure (the engine's preflight is
        // what refuses to launch).
        a.extend(self.awt_bridge().jvm_args(notes));

        // --- log4j configuration from the manifest -------------------------
        if let Some(arg) = self.logging_argument(notes) {
            a.push(arg);
        }
        a
    }

    /// `-Dlog4j.configurationFile=<file>` from the version's `logging.client`
    /// block, when the file was actually downloaded (task 4 places it in
    /// `<data_root>/assets/log_configs/<id>.xml`).
    fn logging_argument(&self, notes: &mut Vec<String>) -> Option<String> {
        let client = self.version.logging.as_ref()?.get("client")?;
        let file_id = client.get("file")?.get("id")?.as_str()?;
        let template = client
            .get("argument")
            .and_then(|v| v.as_str())
            .unwrap_or("-Dlog4j.configurationFile=${path}");
        let file = self
            .options
            .data_root
            .join("assets")
            .join("log_configs")
            .join(file_id);
        if !file.is_file() {
            notes.push(format!(
                "log4j configuration {} not downloaded: using the game's built-in logging",
                file.display()
            ));
            return None;
        }
        let mut subs = Substitutions::new();
        subs.set("path", path(&file));
        Some(subs.apply(template))
    }

    /// The game arguments (everything after the main class).
    fn game_args(&self, subs: &Substitutions, notes: &mut Vec<String>) -> RcResult<Vec<String>> {
        let o = self.options;
        let platform = o.platform();
        let features = o.features();

        // Modern `arguments.game`, else legacy `minecraftArguments`.
        let templates: Vec<String> =
            match (&self.version.arguments, &self.version.minecraft_arguments) {
                (Some(a), _) if !a.game.is_empty() => {
                    flatten_arguments(&a.game, &platform, &features)?
                }
                (_, Some(legacy)) if !legacy.trim().is_empty() => split_legacy_arguments(legacy),
                _ => {
                    return Err(RcError::Launch(format!(
                        "version {} declares neither `arguments.game` nor `minecraftArguments`",
                        self.version.id
                    )))
                }
            };
        // Does this version understand the modern quick-play flags?
        //
        // Checked against the *raw* manifest, not `templates`: the quick-play
        // entries are gated on the `has_quick_plays_support` feature, which is
        // only true once the user picked a target. Using the filtered list would
        // make us fall back to `--server/--port`, which Minecraft 1.20+ rejects.
        let quick_play_capable = match &self.version.arguments {
            Some(a) => a
                .game
                .iter()
                .any(|v| v.to_string().contains("quickPlayMultiplayer")),
            None => false,
        };

        let pruned = prune_unresolved(&templates, subs);
        for d in &pruned.dropped {
            notes.push(format!("dropped game argument: {d}"));
        }
        let mut args = pruned.args;

        // Window: only when the version does not template it itself.
        if o.fullscreen {
            if !has_flag(&args, "--fullscreen") {
                args.push("--fullscreen".into());
            }
        } else if !has_flag(&args, "--width") && !has_flag(&args, "--height") {
            args.push("--width".into());
            args.push(o.window.width.to_string());
            args.push("--height".into());
            args.push(o.window.height.to_string());
        }

        // Demo mode.
        if o.demo && !has_flag(&args, "--demo") {
            args.push("--demo".into());
        }

        // Auto-join a server: 1.20+ replaced `--server/--port` with quick play.
        if let Some(server) = &o.server {
            let host = server.host.trim();
            if host.is_empty() {
                notes.push("ignored empty server address".to_string());
            } else if quick_play_capable {
                if !has_flag(&args, "--quickPlayMultiplayer") {
                    let address = match server.port {
                        Some(p) if p != 25565 => format!("{}:{}", host, p),
                        _ => host.to_string(),
                    };
                    args.push("--quickPlayMultiplayer".into());
                    args.push(address);
                }
            } else if !has_flag(&args, "--server") {
                args.push("--server".into());
                args.push(host.to_string());
                if let Some(port) = server.port {
                    args.push("--port".into());
                    args.push(port.to_string());
                }
            }
        }

        // User extras last so they can override anything above.
        args.extend(o.extra_game_args.iter().cloned());
        Ok(args)
    }

    /// Assemble the full command line.
    pub fn build(&self) -> RcResult<LaunchCommand> {
        let o = self.options;
        o.validate()?;
        let v = self.version;
        if v.id.trim().is_empty() {
            return Err(RcError::Launch("resolved version has no id".into()));
        }
        let main_class = v
            .main_class
            .clone()
            .filter(|m| !m.trim().is_empty())
            .ok_or_else(|| RcError::Launch(format!("version {} declares no mainClass", v.id)))?;
        if self.classpath.is_empty() {
            return Err(RcError::Launch(format!(
                "classpath for version {} is empty",
                v.id
            )));
        }

        let mut notes: Vec<String> = Vec::new();
        let classpath = self.classpath.join(PATH_SEP);
        let natives_dir = o.natives_dir_for(&v.id);
        let lib_dirs = library_path(o, &v.id, &self.classpath.native_dirs);
        let java_library_path = join_paths(&lib_dirs);
        let subs = self.substitutions(&classpath, &natives_dir);

        // 1) our own base arguments
        let mut jvm_args = self.base_jvm_args(&java_library_path, &mut notes);

        // 2) the manifest's `arguments.jvm`, rule-filtered and templated
        let manifest_jvm: Vec<String> = match &v.arguments {
            Some(a) if !a.jvm.is_empty() => {
                flatten_arguments(&a.jvm, &o.platform(), &o.features())?
            }
            _ => Vec::new(),
        };
        let pruned = prune_unresolved(&manifest_jvm, &subs);
        for d in &pruned.dropped {
            notes.push(format!("dropped jvm argument: {d}"));
        }
        let mut skip_value = false;
        for arg in pruned.args {
            if skip_value {
                skip_value = false;
                continue;
            }
            if CLASSPATH_FLAGS.contains(&arg.as_str()) {
                // The launcher owns the classpath (LWJGL substitution etc.).
                skip_value = true;
                notes.push("replaced the manifest classpath with the assembled one".to_string());
                continue;
            }
            if let Some(prefix) = REPLACED_JVM_PREFIXES.iter().find(|p| arg.starts_with(**p)) {
                notes.push(format!("replaced manifest jvm argument {}…", prefix));
                continue;
            }
            // Our own value for the same `-Dkey` wins (we know Android).
            if let Some(key) = property_key(&arg) {
                let dup = format!("{}=", key);
                if jvm_args
                    .iter()
                    .any(|e| e == key || e.starts_with(dup.as_str()))
                {
                    continue;
                }
            }
            jvm_args.push(arg);
        }

        // 3) user JVM arguments (last => they win), minus a classpath override
        let mut skip_value = false;
        for arg in &o.extra_jvm_args {
            if skip_value {
                skip_value = false;
                continue;
            }
            if CLASSPATH_FLAGS.contains(&arg.as_str()) {
                skip_value = true;
                notes.push(
                    "ignored user -cp: the launcher assembles the classpath itself".to_string(),
                );
                continue;
            }
            jvm_args.push(arg.clone());
        }

        // 4) the classpath, always last of the JVM block
        jvm_args.push("-cp".into());
        jvm_args.push(classpath);

        let game_args = self.game_args(&subs, &mut notes)?;
        let cmd = LaunchCommand {
            program: o.java_executable(),
            jvm_args,
            main_class,
            game_args,
            working_dir: o.game_dir.clone(),
            env: build_env(o, &v.id, &self.classpath.native_dirs),
            version_id: v.id.clone(),
            notes,
            secrets: o.secrets(),
        };
        cmd.validate()?;
        Ok(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::version::{AssetIndexRef, VersionArguments};
    use crate::launch::options::{
        AccountProfile, LwjglVersion, MemoryOptions, Renderer, ServerAddress, WindowSize,
    };
    use crate::runtime::Abi;

    fn opts() -> LaunchOptions {
        let mut o = LaunchOptions::new(
            "/data/mc/.minecraft",
            "/data/mc",
            "/data/jre17",
            JavaVersion::Java17,
            AccountProfile::offline("Steve", "0-0-0-0"),
        );
        o.native_lib_dir = Some(PathBuf::from("/data/app/lib/arm64"));
        o.abi = Abi::Arm64V8a;
        o.lwjgl_version = LwjglVersion::V3_3_3;
        o.memory = MemoryOptions {
            min_mb: Some(256),
            max_mb: 2048,
        };
        o
    }

    /// A modern (1.13+) manifest with rule-gated argument lists.
    fn modern() -> ResolvedVersion {
        let mut v = ResolvedVersion::default();
        v.id = "1.20.4".into();
        v.kind = Some("release".into());
        v.main_class = Some("net.minecraft.client.main.Main".into());
        v.asset_index = Some(AssetIndexRef {
            id: "12".into(),
            sha1: None,
            size: None,
            total_size: None,
            url: "https://example/12.json".into(),
        });
        v.arguments = Some(VersionArguments {
            jvm: serde_json::json!([
                "-Djava.library.path=${natives_directory}",
                "-Djna.tmpdir=${natives_directory}",
                "-Dminecraft.launcher.brand=${launcher_name}",
                "-cp",
                "${classpath}",
                { "rules": [{ "action": "allow", "os": { "name": "windows" } }],
                  "value": "-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump" }
            ])
            .as_array()
            .unwrap()
            .clone(),
            game: serde_json::json!([
                "--username", "${auth_player_name}",
                "--version", "${version_name}",
                "--gameDir", "${game_directory}",
                "--assetsDir", "${assets_root}",
                "--assetIndex", "${assets_index_name}",
                "--uuid", "${auth_uuid}",
                "--accessToken", "${auth_access_token}",
                "--clientId", "${clientid}",
                "--xuid", "${auth_xuid}",
                "--userType", "${user_type}",
                "--versionType", "${version_type}",
                { "rules": [{ "action": "allow", "features": { "is_demo_user": true } }],
                  "value": "--demo" },
                { "rules": [{ "action": "allow", "features": { "has_quick_plays_support": true } }],
                  "value": ["--quickPlayPath", "${quickPlayPath}"] },
                { "rules": [{ "action": "allow", "features": { "is_quick_play_multiplayer": true } }],
                  "value": ["--quickPlayMultiplayer", "${quickPlayMultiplayer}"] }
            ])
            .as_array()
            .unwrap()
            .clone(),
        });
        v
    }

    /// A legacy (<= 1.12) manifest with a flat `minecraftArguments`.
    fn legacy() -> ResolvedVersion {
        let mut v = ResolvedVersion::default();
        v.id = "1.7.10".into();
        v.main_class = Some("net.minecraft.client.main.Main".into());
        v.assets = Some("legacy".into());
        v.minecraft_arguments = Some(
            "--username ${auth_player_name} --version ${version_name} --gameDir ${game_directory} \
             --assetsDir ${game_assets} --session ${auth_session} --userProperties ${user_properties}"
                .into(),
        );
        v
    }

    fn cp() -> Classpath {
        Classpath {
            entries: vec![
                PathBuf::from("/data/mc/libraries/a/a.jar"),
                PathBuf::from("/data/mc/versions/1.20.4/1.20.4.jar"),
            ],
            native_jars: vec![],
            native_dirs: vec![PathBuf::from("/data/rt/lwjgl/3.3.3/natives/arm64-v8a")],
            substituted: vec!["org.lwjgl:lwjgl:3.3.3".into()],
            collapsed: vec![],
        }
    }

    fn build(o: &LaunchOptions, v: &ResolvedVersion, c: &Classpath) -> LaunchCommand {
        CommandBuilder::new(o, v, c).build().expect("build")
    }

    #[test]
    fn assembles_program_main_class_and_order() {
        let (o, v, c) = (opts(), modern(), cp());
        let cmd = build(&o, &v, &c);
        assert_eq!(cmd.program, PathBuf::from("/data/jre17/bin/java"));
        assert_eq!(cmd.main_class, "net.minecraft.client.main.Main");
        assert_eq!(cmd.working_dir, PathBuf::from("/data/mc/.minecraft"));
        // -cp is the *last* JVM argument pair, immediately before the main class.
        let args = cmd.args();
        let cp_idx = args.iter().position(|a| a == "-cp").unwrap();
        assert_eq!(args[cp_idx + 1], c.join(":"));
        assert_eq!(args[cp_idx + 2], cmd.main_class);
        // exactly one classpath flag
        assert_eq!(args.iter().filter(|a| *a == "-cp").count(), 1);
        cmd.validate().unwrap();
    }

    #[test]
    fn launcher_owns_library_path_and_classpath() {
        let (o, v, c) = (opts(), modern(), cp());
        let cmd = build(&o, &v, &c);
        let lib_path: Vec<&String> = cmd
            .jvm_args
            .iter()
            .filter(|a| a.starts_with("-Djava.library.path="))
            .collect();
        assert_eq!(lib_path.len(), 1, "{:?}", cmd.jvm_args);
        // ours contains the natives dir *and* the prebuilt LWJGL + JRE dirs
        let p = lib_path[0];
        assert!(p.contains("/data/mc/versions/1.20.4/natives-arm64-v8a"));
        assert!(p.contains("/data/rt/lwjgl/3.3.3/natives/arm64-v8a"));
        assert!(p.contains("/data/jre17/lib/server"));
        assert!(p.contains("/data/app/lib/arm64"));
        // the manifest's own `-cp ${classpath}` pair was dropped, not duplicated
        assert!(cmd
            .notes
            .iter()
            .any(|n| n.contains("replaced the manifest classpath")));
        // a manifest property we do not own survives
        assert!(cmd.jvm_args.iter().any(|a| a.starts_with("-Djna.tmpdir=")));
        // windows-only heap dump path is rule-filtered away
        assert!(!cmd.jvm_args.iter().any(|a| a.contains("HeapDumpPath")));
    }

    #[test]
    fn heap_gc_and_diagnostics_flags_are_present() {
        let mut o = opts();
        o.processor_count = Some(4);
        let cmd = build(&o, &modern(), &cp());
        for expected in [
            "-Xms256M",
            "-Xmx2048M",
            "-XX:+UseG1GC",
            "-XX:-OmitStackTraceInFastThrow",
            "-XX:ActiveProcessorCount=4",
            "-Dfile.encoding=UTF-8",
            "-Dlog4j2.formatMsgNoLookups=true",
            "-Dfml.earlyprogresswindow=false",
            "-Dorg.lwjgl.system.allocator=system",
        ] {
            assert!(
                cmd.jvm_args.iter().any(|a| a == expected),
                "missing {expected} in {:?}",
                cmd.jvm_args
            );
        }
        assert!(cmd
            .jvm_args
            .iter()
            .any(|a| a.starts_with("-XX:ErrorFile=/data/mc/logs/hs_err_pid")));
        assert!(cmd
            .jvm_args
            .iter()
            .any(|a| a == "-Dorg.lwjgl.opengl.libname=libgl4es_114.so"));
    }

    #[test]
    fn renderer_selects_the_gl_library() {
        let mut o = opts();
        o.renderer = Renderer::Zink;
        let cmd = build(&o, &modern(), &cp());
        assert!(cmd
            .jvm_args
            .iter()
            .any(|a| a == "-Dorg.lwjgl.opengl.libname=libOSMesa_8.so"));
        assert_eq!(
            cmd.env.get("POJAV_RENDERER"),
            Some("opengles3_desktopgl_zink_kopper")
        );
    }

    #[test]
    fn cacio_flags_follow_the_java_version() {
        // Java 17+: module flags + the modern toolkit
        let cmd = build(&opts(), &modern(), &cp());
        assert!(cmd
            .jvm_args
            .iter()
            .any(|a| a == &format!("-Dawt.toolkit={}", CACIO17_TOOLKIT)));
        assert!(cmd
            .jvm_args
            .iter()
            .any(|a| a == "--add-opens=java.desktop/sun.font=ALL-UNNAMED"));
        assert!(cmd
            .jvm_args
            .iter()
            .any(|a| a == "-Dcacio.managed.screensize=1280x720"));
        // no agent jar on disk => a note, never a silent failure
        assert!(cmd.notes.iter().any(|n| n.contains("cacio-agent.jar")));

        // Java 8: the original toolkit, no module flags
        let mut o = opts();
        o.java_version = JavaVersion::Java8;
        o.java_home = PathBuf::from("/data/jre8");
        let cmd8 = build(&o, &modern(), &cp());
        assert!(cmd8
            .jvm_args
            .iter()
            .any(|a| a == &format!("-Dawt.toolkit={}", CACIO8_TOOLKIT)));
        assert!(!cmd8.jvm_args.iter().any(|a| a.starts_with("--add-opens")));
        assert!(cmd8
            .jvm_args
            .iter()
            .any(|a| a == "-Dsun.stdout.encoding=UTF-8"));

        // disabled => no AWT bridge at all
        let mut o = opts();
        o.use_cacio = false;
        let cmd_no = build(&o, &modern(), &cp());
        assert!(!cmd_no.jvm_args.iter().any(|a| a.contains("cacio")));
        assert!(!cmd_no.jvm_args.iter().any(|a| a.contains("awt.toolkit")));
    }

    #[test]
    fn a_live_awt_session_advertises_its_channels() {
        // No transport directory => the JVM is not told about any channel (a
        // FIFO nobody reads would block the game's first repaint).
        let cmd = build(&opts(), &modern(), &cp());
        assert!(!cmd.jvm_args.iter().any(|a| a.contains("rc.awt.bridge")));

        // With one, all three properties are handed over.
        let mut o = opts();
        o.awt_transport_dir = Some(PathBuf::from("/data/mc/awt"));
        let cmd = build(&o, &modern(), &cp());
        assert!(cmd
            .jvm_args
            .iter()
            .any(|a| a == "-Drc.awt.bridge.protocol=rcaf1"));
        assert!(cmd
            .jvm_args
            .iter()
            .any(|a| a == "-Drc.awt.bridge.frames=/data/mc/awt/awt-frames.rcaf"));
        assert!(cmd
            .jvm_args
            .iter()
            .any(|a| a == "-Drc.awt.bridge.events=/data/mc/awt/awt-events.rcae"));

        // A headless launch never advertises them, even with a directory set.
        o.use_cacio = false;
        let cmd = build(&o, &modern(), &cp());
        assert!(!cmd.jvm_args.iter().any(|a| a.contains("rc.awt.bridge")));
    }

    #[test]
    fn offline_account_drops_xuid_and_client_id_with_their_flags() {
        let cmd = build(&opts(), &modern(), &cp());
        let joined = cmd.game_args.join(" ");
        assert!(!joined.contains("--xuid"), "{joined}");
        assert!(!joined.contains("--clientId"), "{joined}");
        assert!(!joined.contains("${"), "{joined}");
        assert!(joined.contains("--username Steve"));
        assert!(joined.contains("--userType legacy"));
        assert!(joined.contains("--versionType release"));
        assert!(cmd.notes.iter().any(|n| n.contains("auth_xuid")));
    }

    #[test]
    fn microsoft_account_keeps_xuid_and_client_id() {
        let mut o = opts();
        let mut acc = AccountProfile::microsoft("Alex", "uuid-1", "a-very-secret-token");
        acc.xuid = Some("xuid-9".into());
        acc.client_id = Some("client-9".into());
        o.account = acc;
        let cmd = build(&o, &modern(), &cp());
        let joined = cmd.game_args.join(" ");
        assert!(joined.contains("--xuid xuid-9"), "{joined}");
        assert!(joined.contains("--clientId client-9"), "{joined}");
        assert!(joined.contains("--userType msa"));
    }

    #[test]
    fn access_token_is_never_logged() {
        let mut o = opts();
        o.account = AccountProfile::microsoft("Alex", "uuid-1", "super-secret-token-value");
        let cmd = build(&o, &modern(), &cp());
        // it *is* in the real argv ...
        assert!(cmd
            .game_args
            .iter()
            .any(|a| a == "super-secret-token-value"));
        // ... but never in anything printable
        let shell = cmd.to_shell_string();
        assert!(!shell.contains("super-secret-token-value"), "{shell}");
        assert!(shell.contains("<redacted>"));
        let dbg = format!("{:?}", cmd);
        assert!(!dbg.contains("super-secret-token-value"), "{dbg}");
        assert_eq!(
            cmd.redact("token=super-secret-token-value"),
            "token=<redacted>"
        );
    }

    #[test]
    fn legacy_manifest_uses_minecraft_arguments_and_virtual_assets() {
        let cmd = build(&opts(), &legacy(), &cp());
        let joined = cmd.game_args.join(" ");
        assert!(joined.contains("--username Steve"));
        assert!(joined.contains("--session token:0:0-0-0-0"), "{joined}");
        assert!(joined.contains("--userProperties {}"), "{joined}");
        // pre-1.6 loose assets live in assets/virtual/<index>
        assert!(
            joined.contains("--assetsDir /data/mc/assets/virtual/legacy"),
            "{joined}"
        );
        // no `arguments.jvm` => the launcher still supplies -cp + library path
        assert!(cmd.jvm_args.iter().any(|a| a == "-cp"));
        assert!(cmd
            .jvm_args
            .iter()
            .any(|a| a.starts_with("-Djava.library.path=")));
    }

    #[test]
    fn window_fullscreen_and_demo_extras() {
        // default: width/height appended (the manifest does not template them)
        let cmd = build(&opts(), &modern(), &cp());
        let joined = cmd.game_args.join(" ");
        assert!(joined.contains("--width 1280 --height 720"), "{joined}");

        // fullscreen wins over an explicit size
        let mut o = opts();
        o.fullscreen = true;
        let cmd = build(&o, &modern(), &cp());
        let joined = cmd.game_args.join(" ");
        assert!(joined.contains("--fullscreen"));
        assert!(!joined.contains("--width"));

        // demo mode is rule-gated in the manifest and must appear exactly once
        let mut o = opts();
        o.demo = true;
        let cmd = build(&o, &modern(), &cp());
        assert_eq!(
            cmd.game_args.iter().filter(|a| *a == "--demo").count(),
            1,
            "{:?}",
            cmd.game_args
        );
    }

    #[test]
    fn custom_window_size_marks_the_rule_feature() {
        let mut o = opts();
        o.window = WindowSize {
            width: 854,
            height: 480,
        };
        let cmd = build(&o, &modern(), &cp());
        assert!(cmd.game_args.join(" ").contains("--width 854 --height 480"));
        assert!(cmd
            .jvm_args
            .iter()
            .any(|a| a == "-Dcacio.managed.screensize=854x480"));
    }

    #[test]
    fn server_uses_quick_play_on_modern_and_server_flags_on_legacy() {
        let mut o = opts();
        o.server = Some(ServerAddress {
            host: "mc.example.cn".into(),
            port: Some(25566),
        });
        // modern manifest understands quick play
        let cmd = build(&o, &modern(), &cp());
        let joined = cmd.game_args.join(" ");
        assert!(
            joined.contains("--quickPlayMultiplayer mc.example.cn:25566"),
            "{joined}"
        );
        assert!(!joined.contains("--server"));

        // legacy manifest gets --server/--port
        let cmd = build(&o, &legacy(), &cp());
        let joined = cmd.game_args.join(" ");
        assert!(
            joined.contains("--server mc.example.cn --port 25566"),
            "{joined}"
        );

        // default port is left implicit for quick play
        let mut o2 = opts();
        o2.server = Some(ServerAddress {
            host: "mc.example.cn".into(),
            port: Some(25565),
        });
        let cmd = build(&o2, &modern(), &cp());
        assert!(cmd
            .game_args
            .join(" ")
            .contains("--quickPlayMultiplayer mc.example.cn"));

        // an empty host is ignored with a note instead of producing `--server `
        let mut o3 = opts();
        o3.server = Some(ServerAddress {
            host: "   ".into(),
            port: None,
        });
        let cmd = build(&o3, &legacy(), &cp());
        assert!(!cmd.game_args.iter().any(|a| a == "--server"));
        assert!(cmd.notes.iter().any(|n| n.contains("empty server address")));
    }

    #[test]
    fn quick_play_singleplayer_resolves_its_placeholders() {
        let mut o = opts();
        o.quick_play = QuickPlay::Multiplayer {
            address: "hypixel.cn".into(),
        };
        let cmd = build(&o, &modern(), &cp());
        let joined = cmd.game_args.join(" ");
        assert!(
            joined.contains("--quickPlayMultiplayer hypixel.cn"),
            "{joined}"
        );
        assert!(
            joined.contains("--quickPlayPath /data/mc/.minecraft/quickPlay/log.json"),
            "{joined}"
        );
    }

    #[test]
    fn user_arguments_are_appended_last_and_cannot_break_the_classpath() {
        let mut o = opts();
        o.extra_jvm_args = vec![
            "-Xmx3072M".into(),
            "-cp".into(),
            "/tmp/evil.jar".into(),
            "-Dmy.flag=1".into(),
        ];
        o.extra_game_args = vec!["--myFlag".into()];
        let cmd = build(&o, &modern(), &cp());
        // user -Xmx comes after ours (HotSpot honours the last one)
        let ours = cmd.jvm_args.iter().position(|a| a == "-Xmx2048M").unwrap();
        let theirs = cmd.jvm_args.iter().position(|a| a == "-Xmx3072M").unwrap();
        assert!(theirs > ours);
        assert!(cmd.jvm_args.iter().any(|a| a == "-Dmy.flag=1"));
        // the user's -cp was dropped, ours is intact
        assert!(!cmd.jvm_args.iter().any(|a| a == "/tmp/evil.jar"));
        assert_eq!(cmd.jvm_args.iter().filter(|a| *a == "-cp").count(), 1);
        assert!(cmd.notes.iter().any(|n| n.contains("ignored user -cp")));
        assert_eq!(cmd.game_args.last().unwrap(), "--myFlag");
    }

    #[test]
    fn log4j_configuration_is_used_only_when_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let mut o = opts();
        o.data_root = dir.path().to_path_buf();
        let mut v = modern();
        v.logging = Some(serde_json::json!({
            "client": {
                "argument": "-Dlog4j.configurationFile=${path}",
                "file": { "id": "client-1.12.xml", "url": "https://example/x.xml" },
                "type": "log4j2-xml"
            }
        }));
        // not downloaded yet => a note, no bogus argument
        let cmd = build(&o, &v, &cp());
        assert!(!cmd
            .jvm_args
            .iter()
            .any(|a| a.contains("log4j.configurationFile")));
        assert!(cmd.notes.iter().any(|n| n.contains("log4j configuration")));

        // downloaded => the argument points at the real file
        let cfg_dir = dir.path().join("assets").join("log_configs");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(cfg_dir.join("client-1.12.xml"), b"<Configuration/>").unwrap();
        let cmd = build(&o, &v, &cp());
        let expected = format!(
            "-Dlog4j.configurationFile={}",
            cfg_dir.join("client-1.12.xml").display()
        );
        assert!(
            cmd.jvm_args.iter().any(|a| a == &expected),
            "{:?}",
            cmd.jvm_args
        );
    }

    #[test]
    fn rejects_unlaunchable_versions() {
        let (o, c) = (opts(), cp());
        // no mainClass
        let mut v = modern();
        v.main_class = None;
        let err = CommandBuilder::new(&o, &v, &c).build().unwrap_err();
        assert!(err.to_string().contains("mainClass"), "{err}");

        // no id
        let mut v = modern();
        v.id = String::new();
        assert!(CommandBuilder::new(&o, &v, &c).build().is_err());

        // no arguments at all
        let mut v = modern();
        v.arguments = None;
        v.minecraft_arguments = None;
        let err = CommandBuilder::new(&o, &v, &c).build().unwrap_err();
        assert!(err.to_string().contains("minecraftArguments"), "{err}");

        // empty classpath
        let empty = Classpath::default();
        let err = CommandBuilder::new(&o, &modern(), &empty)
            .build()
            .unwrap_err();
        assert!(err.to_string().contains("classpath"), "{err}");

        // invalid options bubble up (validation happens before anything else)
        let mut bad = opts();
        bad.memory.max_mb = 0;
        assert!(CommandBuilder::new(&bad, &modern(), &c).build().is_err());
    }

    #[test]
    fn validate_catches_unresolved_placeholders() {
        let mut cmd = build(&opts(), &modern(), &cp());
        cmd.game_args.push("${still_here}".into());
        let err = cmd.validate().unwrap_err();
        assert!(err.to_string().contains("unresolved placeholder"), "{err}");

        let mut cmd = build(&opts(), &modern(), &cp());
        cmd.main_class = "  ".into();
        assert!(cmd.validate().is_err());

        let mut cmd = build(&opts(), &modern(), &cp());
        cmd.jvm_args.retain(|a| a != "-cp");
        assert!(cmd.validate().unwrap_err().to_string().contains("-cp"));
    }

    #[test]
    fn shell_string_quotes_paths_with_spaces() {
        let mut o = opts();
        o.game_dir = PathBuf::from("/data/mc/my instance");
        let cmd = build(&o, &modern(), &cp());
        let s = cmd.to_shell_string();
        assert!(s.contains("'/data/mc/my instance'"), "{s}");
        assert!(s.starts_with("/data/jre17/bin/java "));
    }

    #[test]
    fn environment_is_attached_to_the_command() {
        let cmd = build(&opts(), &modern(), &cp());
        assert_eq!(cmd.env.get("JAVA_HOME"), Some("/data/jre17"));
        assert!(cmd
            .env
            .get("LD_LIBRARY_PATH")
            .unwrap()
            .contains("/data/rt/lwjgl/3.3.3/natives/arm64-v8a"));
        assert_eq!(cmd.env.get("HOME"), Some("/data/mc/.minecraft"));
        assert_eq!(cmd.version_id, "1.20.4");
    }

    #[test]
    fn redaction_helpers_ignore_short_and_missing_secrets() {
        assert_eq!(redact_with(&["0".to_string()], "exit 0"), "exit 0");
        assert_eq!(redact_with(&[], "nothing"), "nothing");
        assert_eq!(
            redact_with(&["abcdefghij".to_string()], "x abcdefghij y"),
            "x <redacted> y"
        );
    }

    #[test]
    fn property_key_parsing() {
        assert_eq!(property_key("-Dfoo=bar"), Some("-Dfoo"));
        assert_eq!(property_key("-Dfoo"), Some("-Dfoo"));
        assert_eq!(property_key("-Xmx1G"), None);
    }

    #[test]
    fn forge_style_module_path_arguments_are_substituted() {
        // Forge / NeoForge template the library root and the path separator, and
        // pass a module path the launcher must not touch.
        let mut v = modern();
        v.main_class = Some("cpw.mods.bootstraplauncher.BootstrapLauncher".into());
        v.arguments.as_mut().unwrap().jvm = serde_json::json!([
            "-p",
            "${library_directory}/cpw/mods/bootstraplauncher/1.1.2/bootstraplauncher-1.1.2.jar${classpath_separator}${library_directory}/cpw/mods/securejarhandler/2.1.4/securejarhandler-2.1.4.jar",
            "--add-modules", "ALL-MODULE-PATH",
            "-DignoreList=bootstraplauncher,securejarhandler,${version_name}.jar",
            "-DlibraryDirectory=${library_directory}",
            "-cp", "${classpath}"
        ])
        .as_array()
        .unwrap()
        .clone();
        let cmd = build(&opts(), &v, &cp());
        let joined = cmd.jvm_args.join(" ");
        assert!(
            joined.contains("-p /data/mc/libraries/cpw/mods/bootstraplauncher"),
            "{joined}"
        );
        assert!(
            joined.contains(
                "bootstraplauncher-1.1.2.jar:/data/mc/libraries/cpw/mods/securejarhandler"
            ),
            "the ${{classpath_separator}} must expand to ':' — {joined}"
        );
        assert!(joined.contains("-DignoreList=bootstraplauncher,securejarhandler,1.20.4.jar"));
        assert!(joined.contains("-DlibraryDirectory=/data/mc/libraries"));
        assert!(cmd.jvm_args.iter().any(|a| a == "--add-modules"));
        // still exactly one classpath, ours, and no unresolved placeholder
        assert_eq!(cmd.jvm_args.iter().filter(|a| *a == "-cp").count(), 1);
        cmd.validate().unwrap();
    }

    #[test]
    fn launcher_identity_is_not_duplicated_by_the_manifest() {
        let cmd = build(&opts(), &modern(), &cp());
        // the manifest also declares -Dminecraft.launcher.brand=${launcher_name}
        assert_eq!(
            cmd.jvm_args
                .iter()
                .filter(|a| a.starts_with("-Dminecraft.launcher.brand="))
                .count(),
            1,
            "{:?}",
            cmd.jvm_args
        );
        assert!(cmd
            .jvm_args
            .iter()
            .any(|a| a == "-Dminecraft.launcher.brand=RCLauncher"));
    }

    #[test]
    fn explicit_natives_dir_override_is_honoured() {
        let mut o = opts();
        o.natives_dir = Some(PathBuf::from("/data/custom/natives"));
        let cmd = build(&o, &modern(), &cp());
        assert!(cmd
            .jvm_args
            .iter()
            .any(|a| a.starts_with("-Djava.library.path=/data/custom/natives:")));
        // The manifest also declares `-Djna.tmpdir=${natives_directory}`; ours
        // wins (a single, always-writable temp dir the engine creates) and the
        // property is never passed twice.
        assert_eq!(
            cmd.jvm_args
                .iter()
                .filter(|a| a.starts_with("-Djna.tmpdir="))
                .count(),
            1
        );
        assert!(cmd
            .jvm_args
            .iter()
            .any(|a| a == "-Djna.tmpdir=/data/mc/tmp/jna"));
        // ... and the override reaches the game arguments' natives placeholder
        let subs = CommandBuilder::new(&o, &modern(), &cp())
            .substitutions("cp", &o.natives_dir_for("1.20.4"));
        assert_eq!(subs.get("natives_directory"), Some("/data/custom/natives"));
    }
}
