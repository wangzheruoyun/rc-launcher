//! Discord Rich Presence bridge (task 5).
//!
//! FCL bundles `libdiscord-rpc.so` inside its APK (`lib/arm64-v8a/`, see
//! `FCL_NATIVE_LIBRARIES.md`) and uses it to show a "now playing" card in the
//! user's Discord profile while Minecraft runs. RC shipped without this, so this
//! module re-implements the bridge in the Rust core:
//!
//! The *same* `libdiscord-rpc.so` is also a **compatibility shim for game mods**:
//! Pokémon / Cobblemon-style mods render their own Discord Rich Presence from
//! inside the game JVM, and they `System.loadLibrary("discord-rpc")` it via the
//! app's `nativeLibraryDir` (which is on the game JVM's `java.library.path` /
//! `LD_LIBRARY_PATH` — see [`crate::launch::env::library_path`] and the
//! `COMPAT_NATIVE_LIBS` preflight in [`crate::launch::engine`]). The launcher's
//! own bridge runs in the *launcher* process and never touches the game JVM, so
//! the two usages are fully independent and cannot conflict.
//!
//! * it **lazily `dlopen`s** the native `libdiscord-rpc.so` (no link-time
//!   dependency, so the crate still builds when the `.so` is absent — e.g. on
//!   the host test target or when the user opts out);
//! * it exposes a small, safe, **always-available** API
//!   ([`configure`], [`update_presence`], [`clear_presence`], [`shutdown`],
//!   [`status`]) that degrades to a no-op with a structured status when the
//!   native library cannot be loaded;
//! * it pumps the library's callback loop on a dedicated background thread;
//! * the JNI/C-ABI layers ([`crate::ffi`], [`crate::capi`]) forward the UI
//!   settings (a toggle + custom application id) and the launch engine hooks
//!   [`notify_game_launch`] / [`clear_presence`] so the presence is set on game
//!   start and cleared on process exit.
//!
//! The whole surface is panic-free: every call returns a [`DiscordStateInfo`]
//! JSON snapshot, and the launch-engine hooks swallow any error so a missing or
//! broken Discord library can never take down a Minecraft session.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::event;

/// Default Discord application id used when the UI does not supply one.
///
/// Replace this with RC Launcher's own application registered at
/// <https://discord.com/developers/applications> before shipping; the UI can
/// always override it through [`configure`].
pub const DEFAULT_APPLICATION_ID: &str = "1095782954287534630";

/// Base file name of the native Discord RPC library bundled in the APK.
pub const DEFAULT_LIBRARY_NAME: &str = "libdiscord-rpc.so";

// === Native FFI declarations (mirror discord-rpc.h) =========================

#[repr(C)]
struct DiscordUserFFI {
    user_id: *const c_char,
    username: *const c_char,
    discriminator: *const c_char,
    avatar: *const c_char,
}

type DiscordReadyCb = Option<unsafe extern "C" fn(*const DiscordUserFFI)>;
type DiscordDisconnectedCb = Option<unsafe extern "C" fn(c_int, *const c_char)>;
type DiscordErroredCb = Option<unsafe extern "C" fn(c_int, *const c_char)>;
type DiscordJoinGameCb = Option<unsafe extern "C" fn(*const c_char)>;
type DiscordSpectateGameCb = Option<unsafe extern "C" fn(*const c_char)>;
type DiscordJoinRequestCb = Option<unsafe extern "C" fn(*const DiscordUserFFI)>;

#[repr(C)]
struct DiscordEventHandlersFFI {
    ready: DiscordReadyCb,
    disconnected: DiscordDisconnectedCb,
    errored: DiscordErroredCb,
    join_game: DiscordJoinGameCb,
    spectate_game: DiscordSpectateGameCb,
    join_request: DiscordJoinRequestCb,
}

#[repr(C)]
struct DiscordRichPresenceFFI {
    state: *const c_char,
    details: *const c_char,
    start_timestamp: i64,
    end_timestamp: i64,
    large_image_key: *const c_char,
    large_image_text: *const c_char,
    small_image_key: *const c_char,
    small_image_text: *const c_char,
    party_id: *const c_char,
    party_size: i32,
    party_max: i32,
    match_secret: *const c_char,
    join_secret: *const c_char,
    spectate_secret: *const c_char,
    instance: i8,
}

type SymInitialize =
    unsafe extern "C" fn(*const c_char, *const DiscordEventHandlersFFI, c_int, *const c_char);
type SymShutdown = unsafe extern "C" fn();
type SymRunCallbacks = unsafe extern "C" fn();
type SymUpdatePresence = unsafe extern "C" fn(*const DiscordRichPresenceFFI);
type SymClearPresence = unsafe extern "C" fn();

// === C callbacks that forward Discord lifecycle into the event bus ==========

unsafe extern "C" fn discord_ready(_user: *const DiscordUserFFI) {
    event::publish_lifecycle("discord", "ready", "Discord Rich Presence connected");
}

unsafe extern "C" fn discord_disconnected(code: c_int, msg: *const c_char) {
    let text = msg_to_string(msg);
    event::publish_lifecycle(
        "discord",
        "disconnected",
        format!("Discord Rich Presence disconnected (code {code}): {text}"),
    );
}

unsafe extern "C" fn discord_errored(code: c_int, msg: *const c_char) {
    let text = msg_to_string(msg);
    event::publish_lifecycle(
        "discord",
        "error",
        format!("Discord Rich Presence error (code {code}): {text}"),
    );
}

fn msg_to_string(msg: *const c_char) -> String {
    if msg.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(msg) }
        .to_string_lossy()
        .into_owned()
}

// === Platform-specific dynamic loading ======================================

#[cfg(unix)]
fn dlopen_lib(path: &str) -> Option<*mut c_void> {
    let cpath = CString::new(path).ok()?;
    // SAFETY: `cpath` is a NUL-terminated C string that outlives the call. The
    // returned handle (or NULL) is owned by the caller and closed with
    // `libc::dlclose`.
    unsafe {
        let h = libc::dlopen(cpath.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if h.is_null() {
            None
        } else {
            Some(h)
        }
    }
}

#[cfg(not(unix))]
fn dlopen_lib(_path: &str) -> Option<*mut c_void> {
    // Discord RPC is only wired up on the Android/Linux target.
    None
}

#[cfg(unix)]
unsafe fn load_sym<T: Copy>(handle: *mut c_void, name: &str) -> Option<T> {
    let cname = CString::new(name).ok()?;
    // SAFETY: `cname` is NUL-terminated and outlives the call; `dlsym` returns a
    // function pointer of the requested symbol or NULL. The bit-pattern of a
    // `*mut c_void` is reinterpreted as the (pointer-sized) function pointer `T`
    // via `transmute_copy`, which is sound for fn-pointer types.
    let ptr = libc::dlsym(handle, cname.as_ptr());
    if ptr.is_null() {
        None
    } else {
        Some(std::mem::transmute_copy::<*mut c_void, T>(&ptr))
    }
}

#[cfg(unix)]
unsafe fn dlclose_lib(handle: *mut c_void) {
    libc::dlclose(handle);
}

// === Public configuration / presence model =================================

/// Whether Discord Rich Presence is enabled and which application id it uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscordConfig {
    /// Master on/off switch surfaced as a UI toggle.
    pub enabled: bool,
    /// Discord application id (the "now playing" integration id).
    pub application_id: String,
    /// Optional explicit path to `libdiscord-rpc.so`; when `None` the default
    /// library name is searched on the system/APP native-library path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_path: Option<String>,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            application_id: DEFAULT_APPLICATION_ID.to_string(),
            library_path: None,
        }
    }
}

impl DiscordConfig {
    /// Parse a JSON configuration (used by the FFI layer).
    pub fn from_json(s: &str) -> serde_json::Result<Self> {
        serde_json::from_str(s)
    }
}

/// A Discord Rich Presence payload. Every field is optional so the caller can
/// send a partial update; [`update_presence`] always sends the full struct
/// (Discord replaces the whole presence on each call).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RichPresence {
    /// First line, e.g. "In main menu" / "Playing singleplayer".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Second line, e.g. "Minecraft 1.20.4".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    /// Unix epoch **seconds** the activity started (shows "elapsed" timer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_timestamp: Option<u64>,
    /// Unix epoch seconds the activity ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_timestamp: Option<u64>,
    /// Large image key (asset registered on the Discord application).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub large_image_key: Option<String>,
    /// Hover text for the large image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub large_image_text: Option<String>,
    /// Small image key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_image_key: Option<String>,
    /// Hover text for the small image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_image_text: Option<String>,
    /// Party id for multiplayer sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub party_id: Option<String>,
    /// Current party size (multiplayer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub party_size: Option<u32>,
    /// Max party size (multiplayer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub party_max: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spectate_secret: Option<String>,
    /// Whether this activity is an instance (sent to a specific channel).
    #[serde(default)]
    pub instance: bool,
}

impl RichPresence {
    /// Parse a JSON presence (used by the FFI layer).
    pub fn from_json(s: &str) -> serde_json::Result<Self> {
        serde_json::from_str(s)
    }
}

/// A snapshot of the Discord bridge, returned by every public call so the UI
/// can render the current state without a separate query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscordStateInfo {
    /// Whether the feature is switched on in settings.
    pub enabled: bool,
    /// Whether the native library is loaded and initialised.
    pub connected: bool,
    /// The application id currently in use.
    pub application_id: String,
    /// One of: `"disabled"` | `"connected"` | `"unavailable"` | `"error"`.
    pub status: String,
    /// Human readable detail / last error (empty when healthy).
    pub detail: String,
}

impl DiscordStateInfo {
    /// Serialise to the JSON string the FFI layer returns to Kotlin/C.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

// === Internal connection state =============================================

/// An active native connection: owns the `dlopen` handle (so the symbols stay
/// valid), the resolved function pointers, the heap-pinned event handlers the
/// library points at, and the pump thread that drives `Discord_RunCallbacks`.
struct DiscordConnection {
    handle: *mut c_void,
    sym_shutdown: SymShutdown,
    sym_update_presence: SymUpdatePresence,
    sym_clear_presence: SymClearPresence,
    #[allow(dead_code)]
    handlers: Box<DiscordEventHandlersFFI>,
    stop: std::sync::Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

// SAFETY: the raw `handle` is only dereferenced inside `connect` (to resolve
// symbols) and `Drop` (to `dlclose`), both of which run while the global
// `STATE` mutex is held, so there is never concurrent access to the connection.
// The pump thread only ever calls the resolved function pointers, never the
// handle. Therefore treating the connection as `Send + Sync` is sound.
unsafe impl Send for DiscordConnection {}
unsafe impl Sync for DiscordConnection {}

impl Drop for DiscordConnection {
    fn drop(&mut self) {
        // Stop the pump thread first so it issues no more `RunCallbacks`.
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        // Tell the library to tear down its own IPC/worker thread.
        unsafe {
            (self.sym_shutdown)();
        }
        // Finally release the native library.
        #[cfg(unix)]
        unsafe {
            dlclose_lib(self.handle);
        }
    }
}

/// Process-wide Discord state.
struct DiscordState {
    config: DiscordConfig,
    connection: Option<DiscordConnection>,
    last_error: Option<String>,
}

impl Default for DiscordState {
    fn default() -> Self {
        Self {
            config: DiscordConfig::default(),
            connection: None,
            last_error: None,
        }
    }
}

static STATE: OnceLock<Mutex<DiscordState>> = OnceLock::new();

fn state_lock() -> MutexGuard<'static, DiscordState> {
    STATE
        .get_or_init(|| Mutex::new(DiscordState::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Connect to the native library and spawn the callback pump.
fn connect(path: &str, app_id: &str) -> Result<DiscordConnection, String> {
    #[cfg(unix)]
    {
        let handle = dlopen_lib(path).ok_or_else(|| {
            format!("could not load native library '{path}' (is libdiscord-rpc.so bundled?)")
        })?;

        let sym_initialize: SymInitialize = unsafe { load_sym(handle, "Discord_Initialize") }
            .ok_or_else(|| "missing symbol Discord_Initialize".to_string())?;
        let sym_shutdown: SymShutdown = unsafe { load_sym(handle, "Discord_Shutdown") }
            .ok_or_else(|| "missing symbol Discord_Shutdown".to_string())?;
        let sym_run_callbacks: SymRunCallbacks =
            unsafe { load_sym(handle, "Discord_RunCallbacks") }
                .ok_or_else(|| "missing symbol Discord_RunCallbacks".to_string())?;
        let sym_update_presence: SymUpdatePresence =
            unsafe { load_sym(handle, "Discord_UpdatePresence") }
                .ok_or_else(|| "missing symbol Discord_UpdatePresence".to_string())?;
        let sym_clear_presence: SymClearPresence =
            unsafe { load_sym(handle, "Discord_ClearPresence") }
                .ok_or_else(|| "missing symbol Discord_ClearPresence".to_string())?;

        let handlers = Box::new(DiscordEventHandlersFFI {
            ready: Some(discord_ready),
            disconnected: Some(discord_disconnected),
            errored: Some(discord_errored),
            join_game: None,
            spectate_game: None,
            join_request: None,
        });

        let app_id_c = match CString::new(app_id) {
            Ok(c) => c,
            Err(_) => {
                #[cfg(unix)]
                unsafe {
                    dlclose_lib(handle);
                }
                return Err("application id contains a NUL byte".to_string());
            }
        };

        // SAFETY: `app_id_c` and `handlers` outlive the synchronous call; the
        // library copies the strings/handlers internally.
        unsafe {
            sym_initialize(app_id_c.as_ptr(), &*handlers, 1, std::ptr::null());
        }

        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let run = sym_run_callbacks;
        let stop_thread = std::sync::Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !stop_thread.load(Ordering::Relaxed) {
                unsafe {
                    run();
                }
                std::thread::sleep(Duration::from_millis(1000));
            }
        });

        Ok(DiscordConnection {
            handle,
            sym_shutdown,
            sym_update_presence,
            sym_clear_presence,
            handlers,
            stop,
            thread: Some(thread),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (path, app_id);
        Err("Discord Rich Presence is only supported on the Android/Linux target".to_string())
    }
}

fn build_info(st: &DiscordState) -> DiscordStateInfo {
    let status = if !st.config.enabled {
        "disabled"
    } else if st.connection.is_some() {
        "connected"
    } else {
        "unavailable"
    };
    DiscordStateInfo {
        enabled: st.config.enabled,
        connected: st.connection.is_some(),
        application_id: st.config.application_id.clone(),
        status: status.to_string(),
        detail: st.last_error.clone().unwrap_or_default(),
    }
}

// === Public API ============================================================

/// (Re)configure the bridge from a [`DiscordConfig`].
///
/// * `enabled = false` tears down any active connection and reports
///   `status = "disabled"`;
/// * `enabled = true` loads the native library (or reports `unavailable`),
///   initialises it and starts the callback pump.
pub fn configure(cfg: &DiscordConfig) -> DiscordStateInfo {
    let mut st = state_lock();
    st.config = cfg.clone();
    st.last_error = None;

    if !cfg.enabled {
        // Dropping the old connection (if any) tears it down safely.
        st.connection = None;
        let info = build_info(&st);
        drop(st);
        event::publish_lifecycle("discord", "disabled", "Discord Rich Presence disabled");
        return info;
    }

    // Tear down a previous connection before (re)initialising.
    st.connection = None;
    let path = cfg
        .library_path
        .clone()
        .unwrap_or_else(|| DEFAULT_LIBRARY_NAME.to_string());
    let app_id = cfg.application_id.clone();

    match connect(&path, &app_id) {
        Ok(conn) => {
            st.connection = Some(conn);
            let info = build_info(&st);
            drop(st);
            event::publish_lifecycle(
                "discord",
                "connected",
                format!("Discord Rich Presence connected (app {app_id})"),
            );
            info
        }
        Err(e) => {
            st.last_error = Some(e.clone());
            let info = build_info(&st);
            drop(st);
            event::publish_lifecycle(
                "discord",
                "error",
                format!("Discord Rich Presence unavailable: {e}"),
            );
            info
        }
    }
}

/// Update the rich presence. No-op (returns current status) when not connected.
pub fn update_presence(p: &RichPresence) -> DiscordStateInfo {
    let st = state_lock();
    if let Some(conn) = st.connection.as_ref() {
        update_presence_ffi(conn, p);
    }
    let info = build_info(&st);
    drop(st);
    info
}

/// Clear the rich presence (the "now playing" card disappears) without
/// disconnecting. Called by the launch engine when the game process exits.
pub fn clear_presence() -> DiscordStateInfo {
    let st = state_lock();
    if let Some(conn) = st.connection.as_ref() {
        unsafe {
            (conn.sym_clear_presence)();
        }
    }
    let info = build_info(&st);
    drop(st);
    info
}

/// Fully disconnect from Discord and release the native library. Keeps the
/// `enabled` setting so a later [`configure`]/`set_enabled(true)` reconnects.
pub fn shutdown() -> DiscordStateInfo {
    let conn = {
        let mut st = state_lock();
        st.connection.take()
    };
    drop(conn);
    event::publish_lifecycle("discord", "shutdown", "Discord Rich Presence disconnected");
    status()
}

/// Current bridge state as a JSON snapshot.
pub fn status() -> DiscordStateInfo {
    let st = state_lock();
    build_info(&st)
}

/// Whether the native `libdiscord-rpc.so` can be loaded at all. Lets the UI
/// grey out the toggle when the library is missing from the build.
pub fn is_available() -> bool {
    let path = DEFAULT_LIBRARY_NAME;
    dlopen_lib(path).is_some()
}

/// Toggle the bridge on/off, preserving the current application id.
pub fn set_enabled(enabled: bool) -> DiscordStateInfo {
    let mut cfg = {
        let st = state_lock();
        st.config.clone()
    };
    cfg.enabled = enabled;
    configure(&cfg)
}

/// Set the Discord application id, preserving the current enabled state and
/// (re)initialising the connection if it is enabled.
pub fn set_application_id(app_id: &str) -> DiscordStateInfo {
    let mut cfg = {
        let st = state_lock();
        st.config.clone()
    };
    cfg.application_id = app_id.to_string();
    configure(&cfg)
}

/// Convenience used by the launch engine: announce the game on Discord the
/// moment the JVM process spawns. Builds a sensible default presence
/// ("Playing" / "Minecraft <version>") with an elapsed-time timestamp. No-op
/// when the bridge is disabled or the native library is unavailable.
pub fn notify_game_launch(version_id: &str) -> DiscordStateInfo {
    let enabled = {
        let st = state_lock();
        st.config.enabled
    };
    if !enabled {
        return status();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let presence = RichPresence {
        state: Some("Playing".to_string()),
        details: Some(format!("Minecraft {version_id}")),
        start_timestamp: Some(now),
        end_timestamp: None,
        large_image_key: Some("minecraft".to_string()),
        large_image_text: Some("RC Launcher".to_string()),
        small_image_key: None,
        small_image_text: None,
        party_id: None,
        party_size: None,
        party_max: None,
        match_secret: None,
        join_secret: None,
        spectate_secret: None,
        instance: false,
    };
    update_presence(&presence)
}

// === FFI helper: marshal a RichPresence into the native struct ==============

fn opt_cstr(s: &Option<String>) -> Option<CString> {
    match s {
        // A NUL byte would truncate the string; drop the field instead.
        Some(v) => CString::new(v.as_str()).ok(),
        None => None,
    }
}

fn ptr_or_null(c: &Option<CString>) -> *const c_char {
    match c {
        Some(cs) => cs.as_ptr(),
        None => std::ptr::null(),
    }
}

fn update_presence_ffi(conn: &DiscordConnection, p: &RichPresence) {
    let state = opt_cstr(&p.state);
    let details = opt_cstr(&p.details);
    let large_image_key = opt_cstr(&p.large_image_key);
    let large_image_text = opt_cstr(&p.large_image_text);
    let small_image_key = opt_cstr(&p.small_image_key);
    let small_image_text = opt_cstr(&p.small_image_text);
    let party_id = opt_cstr(&p.party_id);
    let match_secret = opt_cstr(&p.match_secret);
    let join_secret = opt_cstr(&p.join_secret);
    let spectate_secret = opt_cstr(&p.spectate_secret);

    let ffi = DiscordRichPresenceFFI {
        state: ptr_or_null(&state),
        details: ptr_or_null(&details),
        start_timestamp: p.start_timestamp.unwrap_or(0) as i64,
        end_timestamp: p.end_timestamp.unwrap_or(0) as i64,
        large_image_key: ptr_or_null(&large_image_key),
        large_image_text: ptr_or_null(&large_image_text),
        small_image_key: ptr_or_null(&small_image_key),
        small_image_text: ptr_or_null(&small_image_text),
        party_id: ptr_or_null(&party_id),
        party_size: p.party_size.unwrap_or(0) as i32,
        party_max: p.party_max.unwrap_or(0) as i32,
        match_secret: ptr_or_null(&match_secret),
        join_secret: ptr_or_null(&join_secret),
        spectate_secret: ptr_or_null(&spectate_secret),
        instance: if p.instance { 1 } else { 0 },
    };

    // SAFETY: `ffi` and every `CString` it points at stay alive for the whole
    // synchronous call; the library copies the data internally.
    unsafe {
        (conn.sym_update_presence)(&ffi);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile;

    // All discord tests share the process-wide `STATE` (mirrors the real FFI,
    // which is a single global bridge), so they must run serially and each
    // start from a known baseline via [`reset`].
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Reset the global bridge state to a known baseline.
    fn reset() {
        let mut st = state_lock();
        st.connection = None;
        st.config = DiscordConfig::default();
        st.last_error = None;
    }

    /// Acquire the test serialisation lock.
    fn guard() -> MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn config_defaults_are_sane() {
        let _g = guard();
        reset();
        let c = DiscordConfig::default();
        assert!(!c.enabled);
        assert_eq!(c.application_id, DEFAULT_APPLICATION_ID);
        assert!(c.library_path.is_none());
    }

    #[test]
    fn presence_round_trips_through_json() {
        let _g = guard();
        reset();
        let p = RichPresence {
            state: Some("Playing".into()),
            details: Some("Minecraft 1.20.4".into()),
            start_timestamp: Some(1_700_000_000),
            large_image_key: Some("minecraft".into()),
            instance: true,
            ..Default::default()
        };
        let j = serde_json::to_string(&p).unwrap();
        let back: RichPresence = serde_json::from_str(&j).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn config_round_trips_through_json() {
        let _g = guard();
        reset();
        let j = r#"{"enabled":true,"application_id":"123456789012345678"}"#;
        let c: DiscordConfig = DiscordConfig::from_json(j).unwrap();
        assert!(c.enabled);
        assert_eq!(c.application_id, "123456789012345678");
        assert!(c.library_path.is_none());
    }

    #[test]
    fn status_is_disabled_by_default() {
        let _g = guard();
        reset();
        // A pristine state (no connection attempted) reports disabled.
        let info = status();
        assert!(!info.enabled);
        assert!(!info.connected);
        assert_eq!(info.status, "disabled");
    }

    #[test]
    fn disabled_config_is_a_noop_and_safe() {
        let _g = guard();
        reset();
        let info = configure(&DiscordConfig {
            enabled: false,
            application_id: DEFAULT_APPLICATION_ID.into(),
            library_path: None,
        });
        assert_eq!(info.status, "disabled");
        assert!(!info.connected);

        // Updating / clearing while disabled must not panic and must stay safe.
        let p = RichPresence {
            state: Some("x".into()),
            details: Some("y".into()),
            ..Default::default()
        };
        let u = update_presence(&p);
        assert_eq!(u.status, "disabled");
        let c = clear_presence();
        assert_eq!(c.status, "disabled");
    }

    #[test]
    fn enable_without_native_lib_degrades_gracefully() {
        let _g = guard();
        reset();
        // On the host there is no libdiscord-rpc.so; enabling must report
        // "unavailable" rather than panic or abort.
        let info = configure(&DiscordConfig {
            enabled: true,
            application_id: DEFAULT_APPLICATION_ID.into(),
            library_path: None,
        });
        assert!(info.enabled);
        assert_eq!(info.status, "unavailable");
        assert!(!info.connected);
        assert!(!info.detail.is_empty());

        // And it goes back to disabled cleanly.
        let off = set_enabled(false);
        assert_eq!(off.status, "disabled");
    }

    #[test]
    fn is_available_is_false_without_bundled_lib() {
        let _g = guard();
        reset();
        // The native lib is not present in this build, so availability is false.
        // This documents the graceful-degradation contract.
        assert!(!is_available());
    }

    #[test]
    fn notify_game_launch_is_safe_when_disabled() {
        let _g = guard();
        reset();
        // With the default (disabled) config this is a no-op that must not panic.
        let info = notify_game_launch("1.20.4");
        assert!(!info.connected);
    }

    // === Native-linking integration test ====================================
    //
    // Builds a tiny stand-in `libdiscord-rpc.so` with the exact C symbols the
    // real Discord RPC library exports, then drives the full Rust bridge
    // (configure -> update -> clear -> shutdown) and finally re-opens the *same*
    // library to read back what the Rust side actually called. This proves the
    // FFI struct layout, symbol names and calling convention are correct end to
    // end. Skipped automatically when no C compiler is available.

    #[cfg(unix)]
    const FAKE_DISCORD_C_HEAD: &str = r#"
#include <stdint.h>
#include <string.h>
#include <stdio.h>

typedef struct {
    const char* state;
    const char* details;
    int64_t startTimestamp;
    int64_t endTimestamp;
    const char* largeImageKey;
    const char* largeImageText;
    const char* smallImageKey;
    const char* smallImageText;
    const char* partyId;
    int partySize;
    int partyMax;
    const char* matchSecret;
    const char* joinSecret;
    const char* spectateSecret;
    int8_t instance;
} DiscordRichPresence;

typedef struct {
    void (*ready)(const void*);
    void (*disconnected)(int, const char*);
    void (*errored)(int, const char*);
    void (*joinGame)(const char*);
    void (*spectateGame)(const char*);
    void (*joinRequest)(const void*);
} DiscordEventHandlers;

static FILE* rc_probe(void) {
    return fopen(RC_PROBE_FILE, "a");
}

void Discord_Initialize(const char* appId, DiscordEventHandlers* h, int autoReg, const char* steam) {
    (void)h; (void)autoReg; (void)steam;
    FILE* f = rc_probe();
    if (f) { fprintf(f, "init %s\n", appId ? appId : ""); fclose(f); }
}
void Discord_Shutdown(void) {
    FILE* f = rc_probe();
    if (f) { fprintf(f, "shutdown\n"); fclose(f); }
}
void Discord_RunCallbacks(void) { }
void Discord_UpdatePresence(const DiscordRichPresence* p) {
    FILE* f = rc_probe();
    if (f) {
        fprintf(f, "presence %s|%s\n",
            (p && p->state) ? p->state : "",
            (p && p->details) ? p->details : "");
        fclose(f);
    }
}
void Discord_ClearPresence(void) {
    FILE* f = rc_probe();
    if (f) { fprintf(f, "clear\n"); fclose(f); }
}
"#;

    #[cfg(unix)]
    fn build_fake_lib() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        use std::process::Command;
        let cc = if Command::new("gcc").arg("--version").output().is_ok() {
            "gcc"
        } else if Command::new("cc").arg("--version").output().is_ok() {
            "cc"
        } else {
            return None;
        };
        let dir = tempfile::tempdir().ok()?;
        let lib = dir.path().join("libdiscord-rpc.so");
        let probe = dir.path().join("probe.txt");
        // Embed the probe file path as a C string literal. Linux temp paths use
        // only `/` and alphanumerics, so a direct embedding is safe here.
        let probe_cstr = probe.to_string_lossy().into_owned();
        let mut src = String::new();
        src.push_str("#define RC_PROBE_FILE ");
        src.push('"');
        src.push_str(&probe_cstr);
        src.push('"');
        src.push('\n');
        src.push_str(FAKE_DISCORD_C_HEAD);
        let src_path = dir.path().join("fake_discord.c");
        std::fs::write(&src_path, src).ok()?;
        let status = Command::new(cc)
            .args(["-shared", "-fPIC", "-o"])
            .arg(&lib)
            .arg(&src_path)
            .status()
            .ok()?;
        if !status.success() {
            return None;
        }
        std::mem::forget(dir);
        Some((lib, probe))
    }

    #[cfg(unix)]
    #[test]
    fn native_linking_calls_into_bundled_library() {
        let _g = guard();
        reset();
        let (lib, probe) = match build_fake_lib() {
            Some(x) => x,
            None => {
                eprintln!("gcc/cc unavailable; skipping native linking test");
                return;
            }
        };
        // Start from a clean probe file.
        let _ = std::fs::remove_file(&probe);

        // Configure with the fake library -> should connect.
        let info = configure(&DiscordConfig {
            enabled: true,
            application_id: "987654321098765432".into(),
            library_path: Some(lib.to_string_lossy().into_owned()),
        });
        assert_eq!(info.status, "connected", "expected to connect to fake lib");
        assert!(info.connected);

        // Update the rich presence, clear, then shut down.
        let p = RichPresence {
            state: Some("Playing".into()),
            details: Some("Minecraft 1.20.4".into()),
            start_timestamp: Some(1_700_000_000),
            large_image_key: Some("minecraft".into()),
            instance: true,
            ..Default::default()
        };
        update_presence(&p);
        clear_presence();
        shutdown();

        // The fake lib recorded every call it received into the probe file. Read it
        // back and verify the Rust FFI bindings reached the native symbols with the
        // right arguments.
        let log = std::fs::read_to_string(&probe).unwrap_or_default();
        assert!(
            log.contains("init 987654321098765432"),
            "Discord_Initialize not called with app id; log=\n{log}"
        );
        assert!(
            log.contains("presence Playing|Minecraft 1.20.4"),
            "Discord_UpdatePresence not called with expected fields; log=\n{log}"
        );
        assert!(
            log.contains("clear"),
            "Discord_ClearPresence not called; log=\n{log}"
        );
        assert!(
            log.contains("shutdown"),
            "Discord_Shutdown not called; log=\n{log}"
        );
    }
}
