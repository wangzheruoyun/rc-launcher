//! Launch engine (task 7).
//!
//! Turns a resolved version (task 4) + a provisioned JRE (task 6) + the user's
//! options into a *running, supervised, diagnosable* Minecraft process on
//! Android. The module boundaries mirror FCL's `FCLCore/launch` + `FCLauncher`
//! (and its `Terracotta` launch engine):
//!
//! | module           | responsibility |
//! |------------------|----------------|
//! | [`options`]      | everything the UI can influence (account, heap, window, renderer, ABI, paths) |
//! | [`runtime_assets`] | the on-device `app_runtime/` layout (LWJGL, caciocavallo, JNA) |
//! | [`awt`]          | AWT/Swing compatibility (caciocavallo) + the Canvas Compose draws (task 18) |
//! | [`fakefx`]       | the live AWT session: frame/event transport + the surface Compose polls (task 18) |
//! | [`awt_host`]     | the session owner: named-pipe channels + the pump threads that feed it (task 18) |
//! | [`classpath`]    | rule-filtered classpath + LWJGL substitution + duplicate collapsing |
//! | [`args`]         | `${...}` templating, rule-gated argument lists, pruning |
//! | [`env`]          | `LD_LIBRARY_PATH` / `java.library.path` / renderer environment |
//! | [`command`]      | the assembled JVM command line (JVM args, `-cp`, main class, game args) |
//! | [`process`]      | spawning, output streaming, bounded log buffer, exit code / signal |
//! | [`crash`]        | crash classification with evidence and actionable advice |
//! | [`engine`]       | the orchestrator ([`LaunchEngine`]): preflight → command → process → verdict |
//!
//! ```no_run
//! use rc_launcher::launch::{LaunchEngine, LaunchOptions, AccountProfile};
//! use rc_launcher::runtime::JavaVersion;
//!
//! # async fn demo(version: &rc_launcher::game::ResolvedVersion) -> rc_launcher::error::RcResult<()> {
//! let options = LaunchOptions::new(
//!     "/data/data/com.rc.launcher/files/.minecraft",
//!     "/data/data/com.rc.launcher/files",
//!     "/data/data/com.rc.launcher/files/app_runtime/java/jre17",
//!     JavaVersion::Java17,
//!     AccountProfile::offline("Steve", "0-0-0-0"),
//! );
//! let engine = LaunchEngine::new(options);
//! let exit = engine.launch_and_wait(version, |line| println!("{line}")).await?;
//! if !exit.is_success() {
//!     eprintln!("{}", exit.crash.summary());          // e.g. "the game ran out of memory"
//!     eprintln!("{}", exit.crash.category.advice_zh()); // localised, actionable advice
//! }
//! # Ok(())
//! # }
//! ```

pub mod args;
pub mod awt;
pub mod awt_host;
pub mod classpath;
pub mod command;
pub mod crash;
pub mod engine;
pub mod env;
pub mod fakefx;
pub mod options;
pub mod process;
pub mod render;
pub mod runtime_assets;

pub use args::{
    flatten_arguments, has_flag, prune_unresolved, rules_allow, split_legacy_arguments, PrunedArgs,
    Substitutions,
};
pub use awt::{
    cursor_type, decode_control_reply, encode_control_reply, modifier_mask_for_vk, now_millis,
    vk_for_key, AwtBackend, AwtBridge, AwtCanvas, AwtControl, AwtControlKind, AwtEvent,
    AwtEventRecord, AwtFrame, AwtInputTranslator, AwtNativeLib, AwtNativeSet, AwtReplyKind,
    AwtTransport, CacioArtifact, CacioBundle, CacioRole, CanvasStats, CursorKind, Damage,
    MouseButton, PixelFormat, Placement, PointerPhase, Rect, ScaleMode, Viewport,
    AWT_EVENTS_CHANNEL, AWT_FRAMES_CHANNEL, AWT_NATIVES, AWT_PROP_EVENTS, AWT_PROP_FRAMES,
    AWT_PROP_PROTOCOL, AWT_TRANSPORT_PROTOCOL, CACIO17_MODULE_FLAGS, CONTROL_CHUNK_BYTES,
    CONTROL_EVENT_ID, CONTROL_HEADER_LEN, CONTROL_MAGIC, CONTROL_VERSION, EVENT_RECORD_LEN,
    FRAME_HEADER_LEN, MAX_CANVAS_DIM, MAX_CONTROL_TEXT, MAX_REPLY_TEXT,
};
pub use awt_host::{
    AwtHost, LinkState, LinkStats, PollFd, CHANNEL_MODE, DEFAULT_FLUSH_INTERVAL,
    DEFAULT_POLL_INTERVAL,
};
pub use classpath::{Classpath, ClasspathBuilder, ClasspathPolicy};
pub use command::{CommandBuilder, LaunchCommand};
pub use crash::{diagnose, CrashCategory, CrashReport, CrashSeverity};
pub use engine::{LaunchEngine, PreflightChecks, PreparedLaunch};
pub use env::{build_env, jre_lib_dirs, library_path, LaunchEnv, PATH_SEP};
pub use fakefx::{
    AwtControlState, AwtEventWriter, AwtFrameStream, AwtSession, AwtSessionConfig, AwtWindowInfo,
    FrameRead, ImeCaret, SessionStats, DEFAULT_CLICK_SLOP, DEFAULT_MAX_PENDING_CONTROLS,
    DEFAULT_MAX_PENDING_EVENTS, MAX_FRAME_BYTES, MAX_PENDING_CLIPBOARD_REQUESTS,
    MAX_TRACKED_WINDOWS,
};
pub use options::{
    AccountProfile, LaunchOptions, LwjglVersion, MemoryOptions, QuickPlay, Renderer, ServerAddress,
    UserType, WindowSize, LAUNCHER_NAME,
};
pub use process::{
    GameExit, GameProcess, LogBuffer, LogLine, LogStream, SpawnSpec, DEFAULT_STOP_GRACE,
};
pub use render::{
    gl_translation_env, renderer_native_manifest, LwjglNativeBundle, LwjglNativeLib, PerfProfile,
    RenderIntegration, RendererNativeBundle, RendererNativeLib, LWJGL_3_3_3_NATIVES,
    LWJGL_3_4_1_NATIVES,
};
pub use runtime_assets::AppRuntime;
