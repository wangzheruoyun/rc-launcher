//! Launch options: everything the user / UI can influence about a launch (task 7).
//!
//! This is the Rust counterpart of FCL's `LaunchOptions` (`FCLCore/game`) plus
//! the Android-specific knobs `FCLauncher` needs (renderer, ABI, app runtime
//! directory, native library directory).
//!
//! Secrets (the Minecraft access token) are **never** printed: [`AccountProfile`]
//! has a hand-written [`std::fmt::Debug`] that redacts them, and
//! [`AccountProfile::secrets`] enumerates the strings the command builder must
//! redact before logging (see [`crate::launch::command::LaunchCommand`]).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{RcError, RcResult};
use crate::game::platform::{Features, Platform};
use crate::launch::render::PerfProfile;
use crate::plugins::RendererPlugin;
use crate::runtime::{Abi, JavaVersion};

/// Default launcher brand reported to the game (`-Dminecraft.launcher.brand`).
pub const LAUNCHER_NAME: &str = "RCLauncher";

/// The `--userType` reported to the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserType {
    /// Microsoft account (modern).
    Msa,
    /// Legacy Mojang account.
    Mojang,
    /// Offline / cracked account (reported as `legacy`, like vanilla does).
    Offline,
}

impl UserType {
    /// The value substituted into `${user_type}`.
    pub fn as_str(self) -> &'static str {
        match self {
            UserType::Msa => "msa",
            UserType::Mojang => "mojang",
            // Vanilla reports offline sessions as `legacy`.
            UserType::Offline => "legacy",
        }
    }
}

/// The player identity handed to the game.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountProfile {
    pub username: String,
    pub uuid: String,
    /// Minecraft access token (`${auth_access_token}`). Redacted in logs.
    pub access_token: String,
    pub user_type: UserType,
    /// Xbox user id (`${auth_xuid}`), Microsoft accounts only.
    #[serde(default)]
    pub xuid: Option<String>,
    /// OAuth client id (`${clientid}`), Microsoft accounts only.
    #[serde(default)]
    pub client_id: Option<String>,
    /// Legacy `${user_properties}` blob (twitch/legacy profiles).
    #[serde(default = "empty_json_object")]
    pub user_properties: String,
}

fn empty_json_object() -> String {
    "{}".to_string()
}

impl AccountProfile {
    /// An offline profile: no real token, `legacy` user type (vanilla parity).
    pub fn offline(username: impl Into<String>, uuid: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            uuid: uuid.into(),
            // Vanilla passes a dummy token for offline sessions; `0` is what
            // every launcher (incl. FCL) uses.
            access_token: "0".to_string(),
            user_type: UserType::Offline,
            xuid: None,
            client_id: None,
            user_properties: empty_json_object(),
        }
    }

    /// A Microsoft profile.
    pub fn microsoft(
        username: impl Into<String>,
        uuid: impl Into<String>,
        access_token: impl Into<String>,
    ) -> Self {
        Self {
            username: username.into(),
            uuid: uuid.into(),
            access_token: access_token.into(),
            user_type: UserType::Msa,
            xuid: None,
            client_id: None,
            user_properties: empty_json_object(),
        }
    }

    /// Build from the `auth` subsystem's account model (task 5).
    pub fn from_account(account: &crate::auth::model::Account) -> Self {
        match account {
            crate::auth::model::Account::Microsoft(m) => Self {
                username: m.username.clone(),
                uuid: m.uuid.clone(),
                access_token: m.access_token.clone(),
                user_type: UserType::Msa,
                xuid: m.xuid.clone(),
                client_id: Some(m.client_id.clone()),
                user_properties: empty_json_object(),
            },
            crate::auth::model::Account::Offline(o) => {
                Self::offline(o.username.clone(), o.uuid.clone())
            }
        }
    }

    /// Strings that must be redacted from any log / crash report.
    pub fn secrets(&self) -> Vec<String> {
        let mut out = Vec::new();
        // A dummy offline token ("0") is not a secret and redacting it would
        // mangle every unrelated `0` in the command line.
        if self.access_token.len() > 8 {
            out.push(self.access_token.clone());
        }
        out
    }

    /// Validate the profile (non-empty name/uuid).
    pub fn validate(&self) -> RcResult<()> {
        if self.username.trim().is_empty() {
            return Err(RcError::Launch("account username is empty".into()));
        }
        if self.uuid.trim().is_empty() {
            return Err(RcError::Launch("account uuid is empty".into()));
        }
        Ok(())
    }
}

impl fmt::Debug for AccountProfile {
    /// Redacts the access token so it can never leak through `{:?}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccountProfile")
            .field("username", &self.username)
            .field("uuid", &self.uuid)
            .field("access_token", &"<redacted>")
            .field("user_type", &self.user_type)
            .field("xuid", &self.xuid.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Heap sizing for the JVM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryOptions {
    /// `-Xms` in MiB (omitted when `None`).
    #[serde(default)]
    pub min_mb: Option<u32>,
    /// `-Xmx` in MiB.
    pub max_mb: u32,
}

impl Default for MemoryOptions {
    fn default() -> Self {
        Self {
            min_mb: None,
            max_mb: 1024,
        }
    }
}

impl MemoryOptions {
    /// `-Xms` / `-Xmx` arguments (min first, like every launcher emits them).
    pub fn to_args(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(min) = self.min_mb {
            out.push(format!("-Xms{}M", min));
        }
        out.push(format!("-Xmx{}M", self.max_mb));
        out
    }

    /// Clamp to what the device can actually give the JVM.
    ///
    /// Robustness (task 19): an `-Xmx` above the device RAM makes the JVM abort
    /// at startup with an unhelpful message, so we clamp to `total - reserve`
    /// and never below a floor of 256 MiB.
    pub fn clamped(mut self, device_total_mb: u32, reserve_mb: u32) -> Self {
        let budget = device_total_mb.saturating_sub(reserve_mb).max(256);
        if self.max_mb > budget {
            self.max_mb = budget;
        }
        if let Some(min) = self.min_mb {
            if min > self.max_mb {
                self.min_mb = Some(self.max_mb);
            }
        }
        self
    }

    /// Validate the pair (`-Xms` must not exceed `-Xmx`, `-Xmx` must be > 0).
    pub fn validate(&self) -> RcResult<()> {
        if self.max_mb == 0 {
            return Err(RcError::Launch("-Xmx must be greater than 0".into()));
        }
        if let Some(min) = self.min_mb {
            if min > self.max_mb {
                return Err(RcError::Launch(format!(
                    "-Xms{}M exceeds -Xmx{}M",
                    min, self.max_mb
                )));
            }
        }
        Ok(())
    }
}

/// Game window / surface size (`${resolution_width}` / `${resolution_height}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

impl Default for WindowSize {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
        }
    }
}

impl WindowSize {
    /// Non-zero size, scaled by `scale` (mimics FCL's resolution scaler).
    pub fn scaled(self, scale: f32) -> Self {
        let w = ((self.width as f32) * scale).round().max(1.0) as u32;
        let h = ((self.height as f32) * scale).round().max(1.0) as u32;
        Self {
            width: w,
            height: h,
        }
    }

    /// `WxH`, the format `-Dcacio.managed.screensize` expects.
    pub fn as_screen_size(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }
}

/// A server to auto-join on launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerAddress {
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
}

/// Quick-play target (modern `--quickPlay*` arguments, MC 1.20+).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuickPlay {
    #[default]
    None,
    /// Open a single-player world by folder name.
    Singleplayer { world: String },
    /// Join a multiplayer server (`host[:port]`).
    Multiplayer { address: String },
    /// Join a realm by id.
    Realms { id: String },
}

/// Which OpenGL(ES) translation stack to run the game on.
///
/// The ids / library names / environment variables mirror the renderers FCL
/// ships (extracted from the FCL APK): `gl4es` (`libgl4es_114.so`), NG-GL4ES
/// (`libng_gl4es.so`), VirGL (`libvgpu.so`), Zink over Mesa (`libOSMesa_8.so`)
/// and ANGLE. The richer, pluggable representation lives in the `plugins`
/// module ([`crate::plugins::RendererPlugin`] / [`crate::plugins::RendererRegistry`]):
/// the launch engine consumes only this enum's `id()` / `gl_libname()` / `env()`
/// contract (derived from the same built-in descriptors via
/// [`Renderer::plugin`]), while the UI / plugin manager enumerates, injects and
/// verifies renderers through the pluggable interface (task 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Renderer {
    /// GL4ES 1.1.4 — the default, most compatible translation layer.
    #[default]
    Gl4es,
    /// NG-GL4ES — newer GL4ES fork with better shader support.
    NgGl4es,
    /// VirGL / vgpu — virtualised Gallium driver.
    VirGl,
    /// Zink on Mesa (desktop GL over Vulkan).
    Zink,
    /// ANGLE — GLES over Vulkan.
    Angle,
}

impl Renderer {
    /// FCL's renderer id (passed to the renderer plugin / `POJAV_RENDERER`).
    pub fn id(self) -> &'static str {
        match self {
            Renderer::Gl4es => "opengles2",
            Renderer::NgGl4es => "opengles2_ng",
            Renderer::VirGl => "opengles2_vgpu",
            Renderer::Zink => "opengles3_desktopgl_zink_kopper",
            Renderer::Angle => "opengles3_angle",
        }
    }

    /// The `.so` LWJGL must dlopen for `-Dorg.lwjgl.opengl.libname`.
    pub fn gl_libname(self) -> &'static str {
        match self {
            Renderer::Gl4es => "libgl4es_114.so",
            Renderer::NgGl4es => "libng_gl4es.so",
            Renderer::VirGl => "libvgpu.so",
            Renderer::Zink => "libOSMesa_8.so",
            Renderer::Angle => "libGLESv2_angle.so",
        }
    }

    /// Parse a renderer id (as persisted by the settings UI).
    pub fn from_id(s: &str) -> Option<Renderer> {
        match s {
            "opengles2" | "gl4es" => Some(Renderer::Gl4es),
            "opengles2_ng" | "ng_gl4es" => Some(Renderer::NgGl4es),
            "opengles2_vgpu" | "virgl" | "vgpu" => Some(Renderer::VirGl),
            "opengles3_desktopgl_zink_kopper" | "zink" => Some(Renderer::Zink),
            "opengles3_angle" | "angle" => Some(Renderer::Angle),
            _ => None,
        }
    }

    /// Renderer-specific environment variables (GL4ES / Mesa / Gallium tuning).
    ///
    /// These are the variables FCL sets for each stack; they are appended to the
    /// process environment by [`crate::launch::env`].
    pub fn env(self) -> Vec<(&'static str, String)> {
        match self {
            Renderer::Gl4es | Renderer::NgGl4es => vec![
                ("LIBGL_ES", "2".into()),
                ("LIBGL_MIPMAP", "3".into()),
                ("LIBGL_NORMALIZE", "1".into()),
                ("LIBGL_NOINTOVLHACK", "1".into()),
                ("LIBGL_NOERROR", "1".into()),
                ("LIBGL_USE_MC_COLOR", "1".into()),
            ],
            Renderer::VirGl => vec![
                ("GALLIUM_DRIVER", "virpipe".into()),
                ("VTEST_SOCKET_NAME", "/tmp/.virgl_test".into()),
                ("MESA_GL_VERSION_OVERRIDE", "4.3".into()),
                ("MESA_GLSL_VERSION_OVERRIDE", "430".into()),
            ],
            Renderer::Zink => vec![
                ("LIB_MESA_NAME", "libOSMesa_8.so".into()),
                ("MESA_LOADER_DRIVER_OVERRIDE", "zink".into()),
                ("GALLIUM_DRIVER", "zink".into()),
                ("MESA_GL_VERSION_OVERRIDE", "4.6".into()),
                ("MESA_GLSL_VERSION_OVERRIDE", "460".into()),
                ("OSMESA_NO_FLUSH_FRONTBUFFER", "1".into()),
            ],
            Renderer::Angle => vec![
                ("LIBGL_ES", "3".into()),
                ("MESA_GL_VERSION_OVERRIDE", "4.6".into()),
                ("MESA_GLSL_VERSION_OVERRIDE", "460".into()),
            ],
        }
    }
    /// The corresponding pluggable [`RendererPlugin`] descriptor (task 9).
    ///
    /// The launch engine keeps consuming this enum's `id()` / `gl_libname()` /
    /// `env()`; the UI / plugin manager consumes the richer, validatable
    /// [`RendererPlugin`] produced here (the same built-in descriptor the
    /// launch-engine contract is derived from).
    pub fn plugin(&self) -> RendererPlugin {
        crate::plugins::renderer_plugin(*self)
    }
}

/// Which prebuilt LWJGL bundle to put on the classpath.
///
/// FCL ships two, both present in the APK asset catalog
/// (`assets/app_runtime/lwjgl/{3.3.3,3.4.1}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LwjglVersion {
    /// LWJGL 3.3.3 — the safe default (works with MC 1.6 → latest).
    #[default]
    V3_3_3,
    /// LWJGL 3.4.1 — newer bundle with the SDL backend.
    V3_4_1,
}

impl LwjglVersion {
    /// Directory name inside `app_runtime/lwjgl/`.
    pub fn as_dir(self) -> &'static str {
        match self {
            LwjglVersion::V3_3_3 => "3.3.3",
            LwjglVersion::V3_4_1 => "3.4.1",
        }
    }

    /// Parse the directory name.
    pub fn from_dir(s: &str) -> Option<LwjglVersion> {
        match s {
            "3.3.3" => Some(LwjglVersion::V3_3_3),
            "3.4.1" => Some(LwjglVersion::V3_4_1),
            _ => None,
        }
    }
}

/// Everything needed to turn a [`crate::game::ResolvedVersion`] into a process.
///
/// Serialisable so the Compose UI can hand a whole launch configuration to the
/// core as JSON over the JNI bridge ([`crate::ffi`]). Only the five fields the
/// launcher cannot guess (`game_dir`, `data_root`, `java_home`, `java_version`,
/// `account`) are required; everything else falls back to the Android defaults
/// used by [`LaunchOptions::new`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchOptions {
    /// Working directory of the game (`${game_directory}`), i.e. `.minecraft`
    /// or the per-instance directory when version isolation is on.
    pub game_dir: PathBuf,
    /// Launcher data root holding `versions/`, `libraries/`, `assets/`
    /// (the same root the task-4 [`crate::game::DependencyResolver`] plans into).
    pub data_root: PathBuf,
    /// JRE home (contains `bin/java`, `lib/`), from the task-6 runtime manager.
    pub java_home: PathBuf,
    /// Java feature release of `java_home`.
    pub java_version: JavaVersion,
    /// ABI of the device (selects the LWJGL natives slice).
    #[serde(default = "default_abi")]
    pub abi: Abi,
    /// Extracted `app_runtime/` directory (LWJGL / caciocavallo / JNA / java).
    #[serde(default)]
    pub app_runtime: Option<PathBuf>,
    /// Which prebuilt LWJGL bundle to use.
    #[serde(default)]
    pub lwjgl_version: LwjglVersion,
    /// The app's `nativeLibraryDir` (holds `libgl4es_114.so`, `libopenal.so`, …).
    #[serde(default)]
    pub native_lib_dir: Option<PathBuf>,
    /// Directory native jars are extracted to (`${natives_directory}`).
    /// Defaults to `<data_root>/versions/<id>/natives-<abi>` when `None`.
    #[serde(default)]
    pub natives_dir: Option<PathBuf>,
    #[serde(default)]
    pub memory: MemoryOptions,
    #[serde(default)]
    pub window: WindowSize,
    pub account: AccountProfile,
    #[serde(default)]
    pub renderer: Renderer,
    /// Graphics performance profile (task 17 perf tuning: GL4ES / Mesa knobs).
    #[serde(default)]
    pub perf_profile: PerfProfile,
    /// Demo mode (`--demo`, gates the `is_demo_user` rule feature).
    #[serde(default)]
    pub demo: bool,
    #[serde(default)]
    pub fullscreen: bool,
    /// Auto-join server (legacy `--server`/`--port`, modern quick-play).
    #[serde(default)]
    pub server: Option<ServerAddress>,
    #[serde(default)]
    pub quick_play: QuickPlay,
    /// Brand reported through `-Dminecraft.launcher.brand`.
    #[serde(default = "default_launcher_name")]
    pub launcher_name: String,
    #[serde(default = "default_launcher_version")]
    pub launcher_version: String,
    /// User-supplied JVM arguments, appended last so they win.
    #[serde(default)]
    pub extra_jvm_args: Vec<String>,
    /// User-supplied game arguments, appended last.
    #[serde(default)]
    pub extra_game_args: Vec<String>,
    /// Extra / overriding environment variables.
    #[serde(default)]
    pub env_overrides: BTreeMap<String, String>,
    /// Enable the caciocavallo AWT bridge (task 18). Required for any version
    /// that touches AWT/Swing on Android.
    #[serde(default = "default_true")]
    pub use_cacio: bool,
    /// Directory holding the live AWT session channels (task 18).
    ///
    /// When set *and* [`Self::use_cacio`] is on, the command builder advertises
    /// `rc.awt.bridge.frames` / `.events` / `.protocol` to the JVM, so the
    /// caciocavallo-side bridge publishes every repaint to the launcher and
    /// consumes the touches Compose sends back
    /// ([`crate::launch::awt_host::AwtHost::attach_transport`] creates and pumps
    /// the channels). Leave it `None` for a purely off-screen AWT: cacio then
    /// still renders dialogs, they are simply never displayed.
    #[serde(default)]
    pub awt_transport_dir: Option<PathBuf>,
    /// `-XX:ActiveProcessorCount=` (FCL sets this to tame the JVM on big.LITTLE).
    #[serde(default)]
    pub processor_count: Option<u32>,
    /// How many log lines to retain in memory for crash diagnosis.
    #[serde(default = "default_log_buffer_lines")]
    pub log_buffer_lines: usize,
    /// Version-isolation flag: informational, callers set `game_dir` themselves.
    #[serde(default)]
    pub isolated: bool,
}

// --- serde defaults (mirror `LaunchOptions::new`) ----------------------------

fn default_abi() -> Abi {
    Abi::Arm64V8a
}

fn default_launcher_name() -> String {
    LAUNCHER_NAME.to_string()
}

fn default_launcher_version() -> String {
    crate::VERSION.to_string()
}

fn default_true() -> bool {
    true
}

fn default_log_buffer_lines() -> usize {
    2048
}

impl LaunchOptions {
    /// Minimal options with sane Android defaults.
    pub fn new(
        game_dir: impl Into<PathBuf>,
        data_root: impl Into<PathBuf>,
        java_home: impl Into<PathBuf>,
        java_version: JavaVersion,
        account: AccountProfile,
    ) -> Self {
        Self {
            game_dir: game_dir.into(),
            data_root: data_root.into(),
            java_home: java_home.into(),
            java_version,
            abi: Abi::Arm64V8a,
            app_runtime: None,
            lwjgl_version: LwjglVersion::default(),
            native_lib_dir: None,
            natives_dir: None,
            memory: MemoryOptions::default(),
            window: WindowSize::default(),
            account,
            renderer: Renderer::default(),
            perf_profile: PerfProfile::Balanced,
            demo: false,
            fullscreen: false,
            server: None,
            quick_play: QuickPlay::None,
            launcher_name: LAUNCHER_NAME.to_string(),
            launcher_version: crate::VERSION.to_string(),
            extra_jvm_args: Vec::new(),
            extra_game_args: Vec::new(),
            env_overrides: BTreeMap::new(),
            use_cacio: true,
            awt_transport_dir: None,
            processor_count: None,
            log_buffer_lines: 2048,
            isolated: false,
        }
    }

    /// `bin/java` inside [`Self::java_home`].
    pub fn java_executable(&self) -> PathBuf {
        self.java_home.join("bin").join("java")
    }

    /// The platform used for rule evaluation (Android presents as Linux/arm64).
    pub fn platform(&self) -> Platform {
        let mut p = Platform::android();
        p.arch = match self.abi {
            Abi::Arm64V8a => crate::game::platform::Arch::Arm64,
            Abi::ArmeabiV7a => crate::game::platform::Arch::Arm,
            Abi::X86 => crate::game::platform::Arch::X86,
            Abi::X86_64 => crate::game::platform::Arch::X86_64,
        };
        p
    }

    /// Rule features derived from the options, used to filter the conditional
    /// argument lists of a modern `version.json`.
    pub fn features(&self) -> Features {
        let mut f = Features::new();
        f.insert("is_demo_user".to_string(), self.demo);
        f.insert(
            "has_custom_resolution".to_string(),
            self.window != WindowSize::default() || self.fullscreen,
        );
        let quick = !matches!(self.quick_play, QuickPlay::None);
        f.insert("has_quick_plays_support".to_string(), quick);
        f.insert(
            "is_quick_play_singleplayer".to_string(),
            matches!(self.quick_play, QuickPlay::Singleplayer { .. }),
        );
        f.insert(
            "is_quick_play_multiplayer".to_string(),
            matches!(self.quick_play, QuickPlay::Multiplayer { .. }),
        );
        f.insert(
            "is_quick_play_realms".to_string(),
            matches!(self.quick_play, QuickPlay::Realms { .. }),
        );
        f
    }

    /// `${natives_directory}` for `version_id`.
    pub fn natives_dir_for(&self, version_id: &str) -> PathBuf {
        self.natives_dir.clone().unwrap_or_else(|| {
            self.data_root
                .join("versions")
                .join(version_id)
                .join(format!("natives-{}", self.abi.as_android_abi()))
        })
    }

    /// `libraries/` root (matches the task-4 download plan layout).
    pub fn libraries_dir(&self) -> PathBuf {
        self.data_root.join("libraries")
    }

    /// `assets/` root.
    pub fn assets_dir(&self) -> PathBuf {
        self.data_root.join("assets")
    }

    /// Client jar for `version_id` (matches the task-4 download plan layout).
    pub fn client_jar_for(&self, version_id: &str) -> PathBuf {
        self.data_root
            .join("versions")
            .join(version_id)
            .join(format!("{}.jar", version_id))
    }

    /// Validate the options that can be checked without touching the disk.
    pub fn validate(&self) -> RcResult<()> {
        self.account.validate()?;
        self.memory.validate()?;
        if self.window.width == 0 || self.window.height == 0 {
            return Err(RcError::Launch("window size must be non-zero".into()));
        }
        if !self.game_dir.is_absolute() {
            return Err(RcError::Launch(format!(
                "game_dir must be absolute: {}",
                self.game_dir.display()
            )));
        }
        if !self.data_root.is_absolute() {
            return Err(RcError::Launch(format!(
                "data_root must be absolute: {}",
                self.data_root.display()
            )));
        }
        Ok(())
    }

    /// Every secret that must be redacted from logs / crash reports.
    pub fn secrets(&self) -> Vec<String> {
        self.account.secrets()
    }
}

/// Helper: stringify a path for an argument value (lossy, never panics).
pub(crate) fn path_str(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> LaunchOptions {
        LaunchOptions::new(
            "/data/mc/.minecraft",
            "/data/mc",
            "/data/jre17",
            JavaVersion::Java17,
            AccountProfile::offline("Steve", "0000-uuid"),
        )
    }

    #[test]
    fn memory_args_and_clamp() {
        let m = MemoryOptions {
            min_mb: Some(512),
            max_mb: 4096,
        };
        assert_eq!(m.to_args(), vec!["-Xms512M", "-Xmx4096M"]);
        // 3 GiB device with 1 GiB reserved for the system + launcher.
        let c = m.clamped(3072, 1024);
        assert_eq!(c.max_mb, 2048);
        assert_eq!(c.min_mb, Some(512));
        // min is pulled down when it would exceed the clamped max.
        let c2 = MemoryOptions {
            min_mb: Some(4096),
            max_mb: 4096,
        }
        .clamped(1500, 1024);
        assert_eq!(c2.max_mb, 476);
        assert_eq!(c2.min_mb, Some(476));
        // never below the floor
        assert_eq!(MemoryOptions::default().clamped(100, 1024).max_mb, 256);
    }

    #[test]
    fn memory_validation() {
        assert!(MemoryOptions {
            min_mb: Some(2048),
            max_mb: 1024
        }
        .validate()
        .is_err());
        assert!(MemoryOptions {
            min_mb: None,
            max_mb: 0
        }
        .validate()
        .is_err());
        assert!(MemoryOptions::default().validate().is_ok());
    }

    #[test]
    fn account_debug_redacts_token() {
        let a = AccountProfile::microsoft("Alex", "uuid-1", "super-secret-token-value");
        let dbg = format!("{:?}", a);
        assert!(!dbg.contains("super-secret-token-value"), "{dbg}");
        assert!(dbg.contains("<redacted>"));
        assert_eq!(a.secrets(), vec!["super-secret-token-value".to_string()]);
        // the offline dummy token is not treated as a secret
        assert!(AccountProfile::offline("S", "u").secrets().is_empty());
    }

    #[test]
    fn account_validation() {
        assert!(AccountProfile::offline("", "u").validate().is_err());
        assert!(AccountProfile::offline("S", "  ").validate().is_err());
        assert!(AccountProfile::offline("S", "u").validate().is_ok());
    }

    #[test]
    fn user_type_strings() {
        assert_eq!(UserType::Msa.as_str(), "msa");
        assert_eq!(UserType::Mojang.as_str(), "mojang");
        assert_eq!(UserType::Offline.as_str(), "legacy");
    }

    #[test]
    fn features_track_options() {
        let mut o = opts();
        let f = o.features();
        assert!(!f["is_demo_user"]);
        assert!(!f["has_custom_resolution"]);
        assert!(!f["has_quick_plays_support"]);

        o.demo = true;
        o.window = WindowSize {
            width: 800,
            height: 600,
        };
        o.quick_play = QuickPlay::Multiplayer {
            address: "mc.example.cn".into(),
        };
        let f = o.features();
        assert!(f["is_demo_user"]);
        assert!(f["has_custom_resolution"]);
        assert!(f["has_quick_plays_support"]);
        assert!(f["is_quick_play_multiplayer"]);
        assert!(!f["is_quick_play_singleplayer"]);
    }

    #[test]
    fn derived_paths_match_download_plan_layout() {
        let o = opts();
        assert_eq!(
            o.client_jar_for("1.20.4"),
            PathBuf::from("/data/mc/versions/1.20.4/1.20.4.jar")
        );
        assert_eq!(o.libraries_dir(), PathBuf::from("/data/mc/libraries"));
        assert_eq!(o.assets_dir(), PathBuf::from("/data/mc/assets"));
        assert_eq!(
            o.natives_dir_for("1.20.4"),
            PathBuf::from("/data/mc/versions/1.20.4/natives-arm64-v8a")
        );
        assert_eq!(o.java_executable(), PathBuf::from("/data/jre17/bin/java"));
    }

    #[test]
    fn options_validation_rejects_relative_dirs() {
        let mut o = opts();
        o.game_dir = PathBuf::from("relative/dir");
        assert!(o.validate().is_err());
        let mut o = opts();
        o.window.width = 0;
        assert!(o.validate().is_err());
        assert!(opts().validate().is_ok());
    }

    #[test]
    fn platform_follows_abi() {
        let mut o = opts();
        o.abi = Abi::X86_64;
        assert_eq!(o.platform().arch, crate::game::platform::Arch::X86_64);
        assert_eq!(o.platform().os, crate::game::platform::OsName::Linux);
    }

    #[test]
    fn renderer_ids_roundtrip_and_env() {
        for r in [
            Renderer::Gl4es,
            Renderer::NgGl4es,
            Renderer::VirGl,
            Renderer::Zink,
            Renderer::Angle,
        ] {
            assert_eq!(Renderer::from_id(r.id()), Some(r));
            assert!(r.gl_libname().starts_with("lib"));
            assert!(!r.env().is_empty());
        }
        assert_eq!(Renderer::from_id("gl4es"), Some(Renderer::Gl4es));
        assert_eq!(Renderer::from_id("nope"), None);
        // GL4ES needs LIBGL_ES=2; Zink drives Mesa through zink.
        assert!(Renderer::Gl4es
            .env()
            .contains(&("LIBGL_ES", "2".to_string())));
        assert!(Renderer::Zink
            .env()
            .contains(&("MESA_LOADER_DRIVER_OVERRIDE", "zink".to_string())));
    }

    #[test]
    fn lwjgl_version_dirs() {
        assert_eq!(LwjglVersion::V3_3_3.as_dir(), "3.3.3");
        assert_eq!(LwjglVersion::from_dir("3.4.1"), Some(LwjglVersion::V3_4_1));
        assert_eq!(LwjglVersion::from_dir("2.9.4"), None);
    }

    #[test]
    fn window_helpers() {
        let w = WindowSize {
            width: 1920,
            height: 1080,
        };
        assert_eq!(w.as_screen_size(), "1920x1080");
        assert_eq!(
            w.scaled(0.5),
            WindowSize {
                width: 960,
                height: 540
            }
        );
        // never collapses to zero
        assert_eq!(w.scaled(0.0).width, 1);
    }

    #[test]
    fn profile_from_auth_account() {
        let acc = crate::auth::model::Account::Offline(crate::auth::model::OfflineAccount {
            uuid: "u1".into(),
            username: "Steve".into(),
        });
        let p = AccountProfile::from_account(&acc);
        assert_eq!(p.username, "Steve");
        assert_eq!(p.user_type, UserType::Offline);
    }

    #[test]
    fn options_deserialize_from_a_minimal_ui_payload() {
        // The Compose UI only has to send what the launcher cannot guess.
        let json = r#"{
            "game_dir": "/data/mc/.minecraft",
            "data_root": "/data/mc",
            "java_home": "/data/mc/app_runtime/java/jre17",
            "java_version": "jre17",
            "account": { "username": "Steve", "uuid": "0-0-0-0",
                         "access_token": "0", "user_type": "offline" }
        }"#;
        let o: LaunchOptions = serde_json::from_str(json).unwrap();
        assert_eq!(o.java_version, JavaVersion::Java17);
        assert_eq!(o.abi, Abi::Arm64V8a);
        assert_eq!(o.memory.max_mb, 1024);
        assert_eq!(o.window, WindowSize::default());
        assert_eq!(o.renderer, Renderer::Gl4es);
        assert_eq!(o.lwjgl_version, LwjglVersion::default());
        assert_eq!(o.launcher_name, LAUNCHER_NAME);
        assert_eq!(o.log_buffer_lines, 2048);
        assert!(o.use_cacio);
        assert!(!o.demo && !o.fullscreen && !o.isolated);
        assert!(o.validate().is_ok());

        // full round-trip keeps every knob
        let mut full = o.clone();
        full.renderer = Renderer::Zink;
        full.memory = MemoryOptions {
            min_mb: Some(512),
            max_mb: 3072,
        };
        full.quick_play = QuickPlay::Singleplayer {
            world: "新世界".into(),
        };
        full.env_overrides.insert("LIBGL_ES".into(), "3".into());
        full.extra_jvm_args = vec!["-Dfoo=bar".into()];
        let text = serde_json::to_string(&full).unwrap();
        let back: LaunchOptions = serde_json::from_str(&text).unwrap();
        assert_eq!(back.renderer, Renderer::Zink);
        assert_eq!(back.memory, full.memory);
        assert_eq!(back.quick_play, full.quick_play);
        assert_eq!(back.env_overrides, full.env_overrides);
        assert_eq!(back.extra_jvm_args, full.extra_jvm_args);

        // a payload missing a required field is rejected (not silently defaulted)
        assert!(serde_json::from_str::<LaunchOptions>(r#"{"game_dir":"/a"}"#).is_err());
    }
}
