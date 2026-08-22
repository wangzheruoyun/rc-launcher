//! JNI bridge.
//!
//! Functions here follow the `Java_<package>_<Class>_<method>` naming
//! convention expected by `System.loadLibrary("rc_launcher")` from
//! `com.rc.launcher.core.RustBridge`. Every public Rust entry point is wrapped
//! in `catch_unwind` so a panic becomes a `null` return instead of an aborted
//! VM (defensive boundary — see task 19).

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use jni::JavaVM;

use crate::net::default_mirrors;
use crate::{greet, net, VERSION};

/// `RustBridge.getVersion(): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_getVersion(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| env.new_string(VERSION)));
    match built {
        Ok(Ok(s)) => s.into_raw(),
        _ => std::ptr::null_mut(),
    }
}

/// `RustBridge.greet(name: String): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_greet(
    mut env: JNIEnv,
    _class: JClass,
    name: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // 1) read the input (immutable borrow of env)
        let input = match env.get_string(&name) {
            Ok(s) => s.to_str().unwrap_or("").to_string(),
            Err(_) => return std::ptr::null_mut(),
        };
        // 2) pure Rust work (no env involved)
        let out = greet(&input);
        // 3) build the result (mutable borrow of env)
        match env.new_string(out) {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.getDefaultMirrors(): String` — returns the built-in mirror list
/// (task 3) as a JSON string. Wrapped in `catch_unwind` like the other entry
/// points so a panic never aborts the VM.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_getDefaultMirrors(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mirrors = default_mirrors();
        let json = serde_json::to_string(&mirrors).unwrap_or_else(|_| "[]".to_string());
        match env.new_string(json) {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.getDefaultDohServers(): String` — returns the built-in DoH
/// upstream list (task 3) as a JSON string.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_getDefaultDohServers(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let servers = net::default_doh_servers();
        let json = serde_json::to_string(&servers).unwrap_or_else(|_| "[]".to_string());
        match env.new_string(json) {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

// === Account & authentication FFI (task 5) =================================
//
// JSON-in / JSON-out bridge for the `auth` subsystem. Async Microsoft flows
// are driven by a process-wide tokio runtime through `block_on` and MUST be
// invoked from a background thread on the Kotlin side. Every entry point is
// wrapped in `catch_unwind` so a panic never aborts the VM.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use serde_json::json;

use crate::auth::manager::AccountManager;
use crate::auth::microsoft::{self, DeviceCodeChallenge};
use crate::auth::store::{FileTokenStorage, MemoryTokenStorage, TokenStorage};
use crate::auth::transport::ReqwestTransport;
use crate::auth::vault::{AesGcmVault, InsecureVault, SecretVault};
use crate::error::RcResult;

static MANAGER: OnceLock<Mutex<AccountManager>> = OnceLock::new();
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("init auth runtime")
    })
}

fn manager() -> &'static Mutex<AccountManager> {
    MANAGER.get_or_init(|| {
        let transport = ReqwestTransport::with_defaults().expect("init auth transport");
        let mgr = AccountManager::new(
            Box::new(MemoryTokenStorage::new()),
            Arc::new(transport),
            microsoft::DEFAULT_CLIENT_ID,
        )
        .expect("init auth manager");
        Mutex::new(mgr)
    })
}

fn lock_manager() -> MutexGuard<'static, AccountManager> {
    manager().lock().unwrap_or_else(|e| e.into_inner())
}

fn jstr(env: &mut JNIEnv, s: &str) -> jstring {
    match env.new_string(s) {
        Ok(j) => j.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

fn read_input(env: &mut JNIEnv, input: &JString) -> Option<String> {
    match env.get_string(input) {
        Ok(s) => Some(s.to_str().unwrap_or("").to_string()),
        Err(_) => None,
    }
}

fn err_json(env: &mut JNIEnv, msg: &str) -> jstring {
    jstr(env, &json!({ "error": msg }).to_string())
}

fn rc_to_json(env: &mut JNIEnv, r: RcResult<serde_json::Value>) -> jstring {
    match r {
        Ok(v) => jstr(env, &v.to_string()),
        Err(e) => err_json(env, &e.to_string()),
    }
}

/// Parse a hex string into bytes (even length, all hex digits).
fn parse_hex_key(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok())
        .collect()
}

/// Wrap an FFI body (which uses the already-`mut` `env` param) in
/// `catch_unwind`, returning a `jstring` (null on panic).
macro_rules! auth_ffi {
    ($body:block) => {{
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $body));
        match built {
            Ok(s) => s,
            Err(_) => std::ptr::null_mut(),
        }
    }};
}

/// `RustBridge.authInit(configJson): String` — (re)configure the global account
/// store. `configJson` = `{ "path"?: string, "key_hex"?: string, "client_id"?:
/// string }`. With `path` the store is persisted (encrypted when `key_hex` is
/// present; on Android `key_hex` is the Keystore-backed key). Without `path` an
/// in-memory store is used.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_authInit(
    mut env: JNIEnv,
    _class: JClass,
    config: JString,
) -> jstring {
    auth_ffi!({
        let raw = match read_input(&mut env, &config) {
            Some(s) => s,
            None => return err_json(&mut env, "missing config"),
        };
        let cfg: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad config: {e}")),
        };
        let transport = match ReqwestTransport::with_defaults() {
            Ok(t) => t,
            Err(e) => return err_json(&mut env, &e.to_string()),
        };
        let client_id = cfg
            .get("client_id")
            .and_then(|v| v.as_str())
            .unwrap_or(microsoft::DEFAULT_CLIENT_ID)
            .to_string();
        let storage: Box<dyn TokenStorage> =
            if let Some(path) = cfg.get("path").and_then(|v| v.as_str()) {
                let vault: Box<dyn SecretVault> =
                    if let Some(key_hex) = cfg.get("key_hex").and_then(|v| v.as_str()) {
                        match parse_hex_key(key_hex) {
                            Some(k) => match AesGcmVault::new(k) {
                                Ok(v) => Box::new(v),
                                Err(e) => return err_json(&mut env, &e.to_string()),
                            },
                            None => return err_json(&mut env, "invalid key_hex"),
                        }
                    } else {
                        Box::new(InsecureVault)
                    };
                Box::new(FileTokenStorage::with_vault(path, vault))
            } else {
                Box::new(MemoryTokenStorage::new())
            };
        let mgr = match AccountManager::new(storage, Arc::new(transport), client_id) {
            Ok(m) => m,
            Err(e) => return err_json(&mut env, &e.to_string()),
        };
        let mut g = lock_manager();
        let count = mgr.accounts().len();
        *g = mgr;
        jstr(
            &mut env,
            &json!({ "ok": true, "accounts": count }).to_string(),
        )
    })
}

/// `RustBridge.authListAccounts(): String` — JSON array of redacted accounts.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_authListAccounts(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    auth_ffi!({
        let g = lock_manager();
        let summaries = g.summaries();
        drop(g);
        jstr(
            &mut env,
            &serde_json::to_string(&summaries).unwrap_or_else(|_| "[]".to_string()),
        )
    })
}

/// `RustBridge.authAddOfflineAccount(name): String` — JSON account (or error).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_authAddOfflineAccount(
    mut env: JNIEnv,
    _class: JClass,
    name: JString,
) -> jstring {
    auth_ffi!({
        let username = match read_input(&mut env, &name) {
            Some(s) => s,
            None => return err_json(&mut env, "missing name"),
        };
        let mut g = lock_manager();
        match g.add_offline(&username) {
            Ok(a) => rc_to_json(
                &mut env,
                Ok(serde_json::to_value(&a).unwrap_or(serde_json::Value::Null)),
            ),
            Err(e) => err_json(&mut env, &e.to_string()),
        }
    })
}

/// `RustBridge.authBeginMicrosoft(): String` — JSON device-code challenge.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_authBeginMicrosoft(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    auth_ffi!({
        let g = lock_manager();
        let challenge = runtime().block_on(g.begin_microsoft());
        drop(g);
        rc_to_json(
            &mut env,
            challenge.and_then(|c| {
                serde_json::to_value(&c).map_err(|e| crate::error::RcError::Auth(e.to_string()))
            }),
        )
    })
}

/// `RustBridge.authCompleteMicrosoft(challengeJson): String` — JSON account.
/// Blocks until the user finishes sign-in (call from a background thread).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_authCompleteMicrosoft(
    mut env: JNIEnv,
    _class: JClass,
    challenge_json: JString,
) -> jstring {
    auth_ffi!({
        let raw = match read_input(&mut env, &challenge_json) {
            Some(s) => s,
            None => return err_json(&mut env, "missing challenge"),
        };
        let challenge: DeviceCodeChallenge = match serde_json::from_str(&raw) {
            Ok(c) => c,
            Err(e) => return err_json(&mut env, &format!("invalid challenge: {e}")),
        };
        let mut g = lock_manager();
        let account = runtime().block_on(g.complete_microsoft(&challenge, |_| {}));
        drop(g);
        rc_to_json(
            &mut env,
            account.and_then(|a| {
                serde_json::to_value(&a).map_err(|e| crate::error::RcError::Auth(e.to_string()))
            }),
        )
    })
}

/// `RustBridge.authRemoveAccount(uuid): String` — `{"removed": bool}`.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_authRemoveAccount(
    mut env: JNIEnv,
    _class: JClass,
    uuid: JString,
) -> jstring {
    auth_ffi!({
        let id = match read_input(&mut env, &uuid) {
            Some(s) => s,
            None => return err_json(&mut env, "missing uuid"),
        };
        let mut g = lock_manager();
        match g.remove(&id) {
            Ok(b) => rc_to_json(&mut env, Ok(serde_json::json!({ "removed": b }))),
            Err(e) => err_json(&mut env, &e.to_string()),
        }
    })
}

/// `RustBridge.authRefreshAccount(uuid): String` — JSON account (or error).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_authRefreshAccount(
    mut env: JNIEnv,
    _class: JClass,
    uuid: JString,
) -> jstring {
    auth_ffi!({
        let id = match read_input(&mut env, &uuid) {
            Some(s) => s,
            None => return err_json(&mut env, "missing uuid"),
        };
        let mut g = lock_manager();
        let account = runtime().block_on(g.refresh(&id));
        drop(g);
        rc_to_json(
            &mut env,
            account.and_then(|a| {
                serde_json::to_value(&a).map_err(|e| crate::error::RcError::Auth(e.to_string()))
            }),
        )
    })
}

/// `RustBridge.authEnsureFresh(uuid): String` — JSON account, transparently
/// refreshed if the Microsoft token is near expiry (or error).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_authEnsureFresh(
    mut env: JNIEnv,
    _class: JClass,
    uuid: JString,
) -> jstring {
    auth_ffi!({
        let id = match read_input(&mut env, &uuid) {
            Some(s) => s,
            None => return err_json(&mut env, "missing uuid"),
        };
        let mut g = lock_manager();
        let account = runtime().block_on(g.ensure_fresh(&id));
        drop(g);
        rc_to_json(
            &mut env,
            account.and_then(|a| {
                serde_json::to_value(&a).map_err(|e| crate::error::RcError::Auth(e.to_string()))
            }),
        )
    })
}

// === JRE / JDK supply FFI (task 6) ===========================================
//
// The Rust core provisions FCL's prebuilt Android OpenJDK packages (see the
// `runtime` module). These entry points expose the supported ABIs and Java
// versions to the Compose UI so it can build selector controls without knowing
// the FCL asset layout. Every entry point is wrapped in `catch_unwind`.

/// `RustBridge.getSupportedJreAbis(): String` — JSON array of Android ABIs
/// the runtime layer can install a JRE for (e.g. `["arm64-v8a", …]`).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_getSupportedJreAbis(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let abis: Vec<&'static str> = crate::runtime::Abi::all()
            .iter()
            .map(|a| a.as_android_abi())
            .collect();
        let json = serde_json::to_string(&abis).unwrap_or_else(|_| "[]".to_string());
        match env.new_string(json) {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.getSupportedJavaVersions(): String` — JSON array of Java versions
/// the runtime layer can provision (e.g. `["jre8","jre17","jre21","jre25"]`).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_getSupportedJavaVersions(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let versions: Vec<&'static str> = crate::runtime::JavaVersion::all()
            .iter()
            .map(|v| v.as_jre_dir())
            .collect();
        let json = serde_json::to_string(&versions).unwrap_or_else(|_| "[]".to_string());
        match env.new_string(json) {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

// === Launch engine FFI (task 7) ==============================================
//
// JSON-in / JSON-out bridge for the `launch` subsystem. The *logic* lives in the
// `launch_*_json` helpers below so it stays unit-testable without a JVM; the
// `extern "system"` wrappers only marshal strings and catch panics.
//
// Streaming the game log and the process lifecycle (start/stop events) needs the
// async callback / event-bus machinery of task 10; until then the UI uses
// `launchPreview` (settings preview + preflight) and `launchDiagnose` (crash
// analysis of a finished session, e.g. FCL's JVMCrashActivity).

use crate::game::version::{merge_chain, VersionJson};
use crate::launch::{diagnose, LaunchEngine, LaunchOptions};

/// Pure core of `RustBridge.launchPreview`.
///
/// `request` = `{ "options": <LaunchOptions>, "version": <version.json>,
///                "preflight"?: bool }`.
///
/// With `preflight: true` every disk check runs (this is what the *launch* button
/// should call first); otherwise the command line is assembled without touching
/// the filesystem (settings preview on a device where nothing is installed yet).
pub fn launch_preview_json(request: &serde_json::Value) -> RcResult<serde_json::Value> {
    let options: LaunchOptions = serde_json::from_value(
        request
            .get("options")
            .cloned()
            .ok_or_else(|| crate::RcError::Launch("missing `options`".into()))?,
    )
    .map_err(crate::RcError::Json)?;
    let version_value = request
        .get("version")
        .cloned()
        .ok_or_else(|| crate::RcError::Launch("missing `version`".into()))?;
    let parsed: VersionJson =
        serde_json::from_value(version_value).map_err(crate::RcError::Json)?;
    let resolved = merge_chain(&[parsed]);

    let preflight = request
        .get("preflight")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let engine = if preflight {
        LaunchEngine::new(options)
    } else {
        LaunchEngine::dry_run(options)
    };
    Ok(engine.prepare(&resolved)?.to_json())
}

/// Pure core of `RustBridge.launchDiagnose`.
///
/// `request` = `{ "exit_code"?: int, "signal"?: int, "log": string,
///                "requested_stop"?: bool }`.
pub fn launch_diagnose_json(request: &serde_json::Value) -> serde_json::Value {
    let code = request
        .get("exit_code")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let signal = request
        .get("signal")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let requested_stop = request
        .get("requested_stop")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let log = request.get("log").and_then(|v| v.as_str()).unwrap_or("");
    let report = diagnose(code, signal, log.lines(), requested_stop);
    // Task 20: an optional `language` tag localises the verdict for the UI. It
    // is negotiated (so `zh-Hant-TW` works) and defaults to the *current* UI
    // language, which is itself Chinese-first.
    match request.get("language").and_then(|v| v.as_str()) {
        Some(tag) => {
            let lang = crate::i18n::Language::from_tag(tag)
                .or_else(|| crate::i18n::Language::negotiate(tag))
                .unwrap_or_else(crate::i18n::current_language);
            report.to_json_in(lang)
        }
        None => report.to_json_in(crate::i18n::current_language()),
    }
}

/// Pure core of `RustBridge.launchRenderers` — the renderer catalogue for the
/// settings UI (id, LWJGL library name, environment).
pub fn launch_renderers_json() -> serde_json::Value {
    // Sourced from the pluggable renderer registry (task 9) so the catalogue
    // stays in lock-step with the core's `RendererPlugin` descriptors instead
    // of being hard-coded here.
    use std::collections::BTreeMap;
    let renderers: Vec<serde_json::Value> = crate::plugins::RendererRegistry::builtin()
        .all()
        .iter()
        .map(|p| {
            let env: BTreeMap<&str, String> =
                p.env.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            json!({
                "id": p.id,
                "gl_libname": p.gl_libname,
                "display_name": p.display_name,
                "backend": format!("{:?}", p.backend),
                "env": env,
            })
        })
        .collect();
    serde_json::Value::Array(renderers)
}

/// `RustBridge.launchPreview(requestJson): String` — assemble (and optionally
/// preflight) the JVM command line. Returns the prepared-launch JSON, which
/// never contains the access token, or `{"error": ...}`.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_launchPreview(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    auth_ffi!({
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request"),
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad request: {e}")),
        };
        rc_to_json(&mut env, launch_preview_json(&value))
    })
}

/// `RustBridge.launchDiagnose(requestJson): String` — classify a finished game
/// session (exit code / signal / log) into an actionable crash report.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_launchDiagnose(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    auth_ffi!({
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request"),
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad request: {e}")),
        };
        jstr(&mut env, &launch_diagnose_json(&value).to_string())
    })
}

/// `RustBridge.launchRenderers(): String` — JSON array of the renderers the
/// launch engine can configure.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_launchRenderers(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    auth_ffi!({ jstr(&mut env, &launch_renderers_json().to_string()) })
}

// === FFI / JNI bridge: event bus + async callbacks (task 10) =================
//
// Mirrors MCTier's two-layer Rust↔Kotlin bridge:
//   * The C-ABI in `capi` (consumed by `cbindgen`) is the portable contract.
//   * These JNI functions are the *thin* wrapper that registers a Kotlin
//     `RcEventSink` as the bus sink and forwards async jobs to `jobs`.
//
// Thread model: the Kotlin callback object is kept as a JNI `GlobalRef` and the
// `JavaVM` is retained, so events emitted from *any* Rust worker thread attach
// to the JVM and invoke the callback (attach-per-thread, the EasyTier pattern).
// Every entry point is wrapped in `catch_unwind` so a panic never aborts the VM.

use crate::event::{self, Event};
use crate::jobs;

/// Bridges [`event::EventSink`] to a Kotlin object implementing
/// `com.rc.launcher.core.RcEventSink { fun onEvent(json: String) }`.
///
/// The Kotlin object is held as a JNI [`GlobalRef`] so it survives past the
/// subscribing JNI call; the [`JavaVM`] is retained so events emitted from any
/// Rust worker thread can attach and invoke the callback.
struct JniEventSink {
    vm: JavaVM,
    callback: GlobalRef,
}

impl event::EventSink for JniEventSink {
    fn emit(&self, event: &Event) {
        let json = event.to_json();
        // Attach the *current* OS thread to the JVM (tokio worker threads have
        // no attached JNIEnv). The guard detaches on drop, so we never leak a
        // thread attachment.
        let mut guard = match self.vm.attach_current_thread() {
            Ok(g) => g,
            Err(_) => return,
        };
        let env = &mut *guard; // &mut JNIEnv
        let jstr = match env.new_string(json) {
            Ok(s) => s,
            Err(_) => return,
        };
        let _ = env.call_method(
            self.callback.as_obj(),
            "onEvent",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&jstr)],
        );
    }
}

/// Active Kotlin sink (for introspection / idempotent replace).
static JNI_BUS_CTX: OnceLock<Mutex<Option<std::sync::Arc<JniEventSink>>>> = OnceLock::new();

fn jni_bus_ctx() -> &'static Mutex<Option<std::sync::Arc<JniEventSink>>> {
    JNI_BUS_CTX.get_or_init(|| Mutex::new(None))
}

/// `RustBridge.eventBusSubscribe(sink): Boolean` — register a Kotlin
/// `RcEventSink` as the bus sink. Returns `true` if a previous sink was
/// replaced. The callback is invoked on a background thread for every event.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_eventBusSubscribe(
    env: JNIEnv,
    _class: JClass,
    sink: JObject,
) -> jboolean {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if sink.is_null() {
            return JNI_FALSE;
        }
        let vm = match env.get_java_vm() {
            Ok(v) => v,
            Err(_) => return JNI_FALSE,
        };
        let callback = match env.new_global_ref(sink) {
            Ok(g) => g,
            Err(_) => return JNI_FALSE,
        };
        let sink_arc = std::sync::Arc::new(JniEventSink { vm, callback });
        // Wire the core bus to the JNI sink, replacing any previous one.
        event::subscribe(sink_arc.clone());
        *jni_bus_ctx().lock().unwrap_or_else(|e| e.into_inner()) = Some(sink_arc);
        JNI_TRUE
    }));
    match built {
        Ok(v) => v,
        Err(_) => JNI_FALSE,
    }
}

/// `RustBridge.eventBusUnsubscribe(): Unit` — detach the current Kotlin sink.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_eventBusUnsubscribe(
    _env: JNIEnv,
    _class: JClass,
) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        event::unsubscribe();
        *jni_bus_ctx().lock().unwrap_or_else(|e| e.into_inner()) = None;
    }));
}

/// `RustBridge.eventBusHasSink(): Boolean` — whether a Kotlin sink is attached.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_eventBusHasSink(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    if event::has_sink() {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// `RustBridge.eventBusPublish(json): Boolean` — inject a pre-serialised JSON
/// event into the bus (also used to test the round-trip and to replay logs from
/// the Kotlin side). Returns `false` if `json` is not valid event JSON.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_eventBusPublish(
    mut env: JNIEnv,
    _class: JClass,
    json: JString,
) -> jboolean {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &json) {
            Some(s) => s,
            None => return JNI_FALSE,
        };
        if event::publish_json(&raw) {
            JNI_TRUE
        } else {
            JNI_FALSE
        }
    }));
    match built {
        Ok(v) => v,
        Err(_) => JNI_FALSE,
    }
}

/// `RustBridge.runAsync(specJson): String` — fire-and-forget async job that
/// streams progress / lifecycle / error events to the bus and returns
/// immediately with `{ "ok": Boolean, "scope": String }`. This is the "async
/// callback" half of task 10: the UI never blocks; it learns the outcome
/// exclusively through the event bus.
///
/// `specJson` = `{ "scope"?: string, "label"?: string, "steps"?: u32,
/// "fail_at"?: u32, "delay_ms"?: u64 }`.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_runAsync(
    mut env: JNIEnv,
    _class: JClass,
    spec: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &spec) {
            Some(s) => s,
            None => return err_json(&mut env, "missing spec"),
        };
        let spec_val: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad spec: {e}")),
        };
        match jobs::spawn_job(&spec_val) {
            Ok(v) => jstr(&mut env, &v.to_string()),
            Err(e) => err_json(&mut env, &e.to_string()),
        }
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.cancelAsync(scope): Boolean` — cancel a running async job by
/// scope. Returns `true` if a matching job was found.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_cancelAsync(
    mut env: JNIEnv,
    _class: JClass,
    scope: JString,
) -> jboolean {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &scope) {
            Some(s) => s,
            None => return JNI_FALSE,
        };
        if jobs::cancel_job(&raw) {
            JNI_TRUE
        } else {
            JNI_FALSE
        }
    }));
    match built {
        Ok(v) => v,
        Err(_) => JNI_FALSE,
    }
}

// === AWT / Swing bridge FFI (task 18) ========================================
//
// The Compose canvas that shows Minecraft's embedded AWT/Swing UI is driven
// entirely through these entry points. One process hosts at most **one** live
// session (one game), kept in a global behind a `Mutex` exactly like the account
// manager above, so the lock discipline stays in this layer.
//
// Split by cost, mirroring the two things the UI does per frame:
//   * **control plane** (JSON): open / close / configure / input batches. Cheap,
//     readable, and unit-testable on the host (`awt_*_json` helpers).
//   * **pixel plane** (binary): `awtPollFrame` copies only the damaged rows
//     straight into the *direct* `ByteBuffer` that backs the Compose `Bitmap`
//     (zero copy — no Java array, no JSON, no intermediate allocation), and
//     `awtDrainEvents` hands the queued 32-byte AWT records to a Kotlin-side
//     transport when the launcher does not own the named pipes itself.
//
// Every entry point is wrapped in `catch_unwind`; a missing session is a normal
// `{"error": ...}` result, never a crash.

use crate::launch::awt::{MouseButton, PointerPhase, ScaleMode};
use crate::launch::awt_host::AwtHost;
use crate::launch::fakefx::{AwtSession, AwtSessionConfig};
use crate::launch::AwtTransport;
use crate::runtime::JavaVersion;
use jni::objects::{JByteArray, JByteBuffer};
use jni::sys::jbyteArray;

/// The single live AWT session (`None` = no game is showing an AWT canvas).
type AwtSlot = Option<AwtHost>;

static AWT_HOST: OnceLock<Mutex<AwtSlot>> = OnceLock::new();

fn awt_slot() -> &'static Mutex<AwtSlot> {
    AWT_HOST.get_or_init(|| Mutex::new(None))
}

fn lock_awt() -> MutexGuard<'static, AwtSlot> {
    awt_slot().lock().unwrap_or_else(|e| e.into_inner())
}

fn awt_host(slot: &AwtSlot) -> RcResult<&AwtHost> {
    slot.as_ref().ok_or_else(|| {
        crate::error::RcError::Launch(
            "no AWT session is open (call awtOpen before touching the canvas)".to_string(),
        )
    })
}

fn rect_json(rect: &crate::launch::awt::Rect, bytes: usize) -> serde_json::Value {
    json!({
        "changed": true,
        "x": rect.x,
        "y": rect.y,
        "width": rect.width,
        "height": rect.height,
        "bytes": bytes,
    })
}

/// `{"width":w,"height":h}` or `[w,h]`.
fn parse_size(value: &serde_json::Value) -> RcResult<(u32, u32)> {
    let pair = match value {
        serde_json::Value::Array(a) if a.len() == 2 => (a[0].as_u64(), a[1].as_u64()),
        serde_json::Value::Object(_) => (
            value.get("width").and_then(|v| v.as_u64()),
            value.get("height").and_then(|v| v.as_u64()),
        ),
        _ => (None, None),
    };
    match pair {
        (Some(w), Some(h)) => Ok((w.min(u32::MAX as u64) as u32, h.min(u32::MAX as u64) as u32)),
        _ => Err(crate::error::RcError::Launch(format!(
            "expected a size as {{\"width\":…,\"height\":…}}, got {value}"
        ))),
    }
}

/// `{"dir":…}` (the conventional channel pair) or `{"frames":…,"events":…}`.
fn parse_transport(value: &serde_json::Value) -> RcResult<AwtTransport> {
    if let Some(dir) = value.as_str() {
        return Ok(AwtTransport::in_dir(dir));
    }
    if let Some(dir) = value.get("dir").and_then(|v| v.as_str()) {
        return Ok(AwtTransport::in_dir(dir));
    }
    match (
        value.get("frames").and_then(|v| v.as_str()),
        value.get("events").and_then(|v| v.as_str()),
    ) {
        (Some(frames), Some(events)) => Ok(AwtTransport::new(frames, events)),
        _ => Err(crate::error::RcError::Launch(
            "transport needs a \"dir\", or both \"frames\" and \"events\"".to_string(),
        )),
    }
}

/// Open (or replace) the live session. See `RustBridge.awtOpen`.
fn awt_open_json(slot: &mut AwtSlot, req: &serde_json::Value) -> RcResult<serde_json::Value> {
    let mut config: AwtSessionConfig = if req.is_null() {
        AwtSessionConfig::default()
    } else {
        serde_json::from_value(req.clone())
            .map_err(|e| crate::error::RcError::Launch(format!("bad AWT session config: {e}")))?
    };
    if let Some(value) = req.get("java_version") {
        let java: JavaVersion = serde_json::from_value(value.clone())
            .map_err(|e| crate::error::RcError::Launch(format!("bad java_version: {e}")))?;
        config = config.for_java(java);
    }
    // A new game session gets a clean canvas: tear the previous one down first
    // (and *join* its pumps, so no thread writes into the session we drop).
    if let Some(previous) = slot.as_mut() {
        previous.stop_and_join();
    }
    *slot = None;

    let mut host = AwtHost::open(config)?;
    match req.get("transport") {
        Some(value) if !value.is_null() => {
            let transport = parse_transport(value)?;
            host.attach_transport(transport)?;
        }
        _ => {}
    }
    let snapshot = host.to_json();
    *slot = Some(host);
    Ok(snapshot)
}

/// Close the live session, stopping its pump threads.
fn awt_close_json(slot: &mut AwtSlot) -> serde_json::Value {
    match slot.take() {
        Some(mut host) => {
            host.stop_and_join();
            json!({ "closed": true, "link": host.link_stats().to_json() })
        }
        None => json!({ "closed": false }),
    }
}

/// Snapshot of the session + transport (diagnostics screen, HUD).
fn awt_info_json(slot: &AwtSlot) -> serde_json::Value {
    match slot {
        Some(host) => {
            let mut value = host.to_json();
            if let Some(obj) = value.as_object_mut() {
                obj.insert("open".to_string(), json!(true));
            }
            value
        }
        None => json!({ "open": false }),
    }
}

/// Geometry / focus / repaint changes coming from the Compose layer.
fn awt_configure_json(slot: &AwtSlot, req: &serde_json::Value) -> RcResult<serde_json::Value> {
    let host = awt_host(slot)?;
    if let Some(value) = req.get("surface") {
        let (w, h) = parse_size(value)?;
        host.set_surface_size(w, h)?;
    }
    if let Some(value) = req.get("screen") {
        let (w, h) = parse_size(value)?;
        host.resize_screen(w, h)?;
    }
    if let Some(value) = req.get("scale_mode") {
        let mode: ScaleMode = serde_json::from_value(value.clone())
            .map_err(|e| crate::error::RcError::Launch(format!("bad scale_mode: {e}")))?;
        host.set_scale_mode(mode);
    }
    {
        let mut session = host.session();
        if let Some(gained) = req.get("focus").and_then(|v| v.as_bool()) {
            session.set_focus(gained);
        }
        if req.get("release_all").and_then(|v| v.as_bool()) == Some(true) {
            session.release_all();
        }
        if req.get("reset_input").and_then(|v| v.as_bool()) == Some(true) {
            session.reset_input();
        }
        if let Some(argb) = req.get("fill").and_then(|v| v.as_u64()) {
            session.fill(argb as u32);
        }
        if req.get("clear").and_then(|v| v.as_bool()) == Some(true) {
            session.clear();
        }
    }
    Ok(host.to_json())
}

/// Attach the named-pipe transport to an already open session.
fn awt_attach_transport_json(
    slot: &mut AwtSlot,
    req: &serde_json::Value,
) -> RcResult<serde_json::Value> {
    let transport = parse_transport(req.get("transport").unwrap_or(req))?;
    let host = slot.as_mut().ok_or_else(|| {
        crate::error::RcError::Launch("no AWT session is open (call awtOpen first)".to_string())
    })?;
    host.attach_transport(transport)?;
    Ok(host.to_json())
}

/// One input event from Compose. Returns how many AWT records it queued.
///
/// An unknown / malformed event is an error for *that* event only: the rest of
/// the batch still reaches the JVM (a dropped touch beats a dropped gesture).
fn apply_awt_event(session: &mut AwtSession, event: &serde_json::Value) -> RcResult<usize> {
    let bad = |msg: String| crate::error::RcError::Launch(msg);
    let kind = event
        .get("type")
        .or_else(|| event.get("kind"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| bad(format!("input event without a \"type\": {event}")))?;
    let f32_at = |key: &str| -> f32 {
        event
            .get(key)
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
            .unwrap_or(0.0)
    };
    match kind {
        "pointer" | "touch" | "mouse" => {
            let phase = match event
                .get("phase")
                .and_then(|v| v.as_str())
                .unwrap_or("move")
            {
                "down" | "press" => PointerPhase::Down,
                "move" | "drag" => PointerPhase::Move,
                "up" | "release" => PointerPhase::Up,
                other => return Err(bad(format!("unknown pointer phase {other:?}"))),
            };
            let button = match event.get("button") {
                None | Some(serde_json::Value::Null) => MouseButton::Left,
                Some(serde_json::Value::Number(n)) => {
                    MouseButton::from_number(n.as_i64().unwrap_or(1) as i32)
                        .ok_or_else(|| bad(format!("unknown mouse button {n}")))?
                }
                Some(value) => match value.as_str().unwrap_or("") {
                    "left" | "primary" => MouseButton::Left,
                    "middle" => MouseButton::Middle,
                    "right" | "secondary" => MouseButton::Right,
                    other => return Err(bad(format!("unknown mouse button {other:?}"))),
                },
            };
            Ok(session.pointer(phase, f32_at("x"), f32_at("y"), button))
        }
        "scroll" | "wheel" => {
            let ticks = event
                .get("ticks")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .clamp(i32::MIN as i64, i32::MAX as i64) as i32;
            Ok(session.scroll(f32_at("x"), f32_at("y"), ticks))
        }
        "key_down" | "key_up" => {
            let down = kind == "key_down";
            if let Some(code) = event.get("code").and_then(|v| v.as_i64()) {
                let code = code.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
                return Ok(if down {
                    session.key_down(code)
                } else {
                    session.key_up(code)
                });
            }
            let name = event
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| bad(format!("{kind} needs a \"code\" or a \"name\"")))?;
            Ok(if down {
                session.key_down_named(name)
            } else {
                session.key_up_named(name)
            })
        }
        "text" | "type" => {
            let text = event
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| bad("text event without \"text\"".to_string()))?;
            Ok(session.type_text(text))
        }
        "focus" => {
            let gained = event
                .get("gained")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            Ok(session.set_focus(gained))
        }
        "release_all" => Ok(session.release_all()),
        "reset_input" => {
            session.reset_input();
            Ok(0)
        }
        other => Err(bad(format!("unknown AWT input event type {other:?}"))),
    }
}

/// A batch of input events (one JNI call per UI frame, not per touch).
fn awt_input_json(slot: &AwtSlot, req: &serde_json::Value) -> RcResult<serde_json::Value> {
    let host = awt_host(slot)?;
    let events: Vec<serde_json::Value> = match req.get("events") {
        Some(serde_json::Value::Array(a)) => a.clone(),
        Some(other) => vec![other.clone()],
        None => match req {
            serde_json::Value::Array(a) => a.clone(),
            other => vec![other.clone()],
        },
    };
    let mut session = host.session();
    let mut queued = 0usize;
    let mut rejected: Vec<String> = Vec::new();
    for event in &events {
        match apply_awt_event(&mut session, event) {
            Ok(n) => queued += n,
            Err(e) => rejected.push(e.to_string()),
        }
    }
    let (px, py) = session.pointer_position();
    Ok(json!({
        "queued": queued,
        "pending": session.pending_events(),
        "modifiers": session.modifiers(),
        "focused": session.is_focused(),
        "pointer": { "x": px, "y": py },
        "rejected": rejected,
    }))
}

/// Feed one encoded frame in (a Kotlin-side transport, or a test).
fn awt_submit_frame_json(slot: &AwtSlot, bytes: &[u8]) -> RcResult<serde_json::Value> {
    let host = awt_host(slot)?;
    Ok(match host.submit_frame_bytes(bytes)? {
        Some(rect) => rect_json(&rect, rect.area() as usize * 4),
        None => json!({ "changed": false }),
    })
}

/// Refresh the caller's RGBA framebuffer with whatever changed.
fn awt_poll_frame_json(slot: &AwtSlot, dst: &mut [u8]) -> RcResult<serde_json::Value> {
    let host = awt_host(slot)?;
    Ok(match host.poll_frame_into(dst)? {
        Some((rect, bytes)) => rect_json(&rect, bytes),
        None => json!({ "changed": false }),
    })
}

/// The queued AWT records, encoded for the JVM-side bridge.
fn awt_drain_events_bytes(slot: &AwtSlot) -> Vec<u8> {
    match slot {
        Some(host) => host.session().drain_encoded(),
        None => Vec::new(),
    }
}

/// `RustBridge.awtOpen(configJson): String` — open (or replace) the live AWT
/// session. `configJson` =
/// `{"screen":{"width":1280,"height":720},"surface":{…},"scale_mode":"fit",
///   "click_slop":8,"max_pending_events":4096,"java_version":"jre17",
///   "transport":{"dir":"…"}?}`. Returns the session snapshot.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_awtOpen(
    mut env: JNIEnv,
    _class: JClass,
    config_json: JString,
) -> jstring {
    auth_ffi!({
        let raw = match read_input(&mut env, &config_json) {
            Some(s) => s,
            None => return err_json(&mut env, "missing config json"),
        };
        let value: serde_json::Value = if raw.trim().is_empty() {
            serde_json::Value::Null
        } else {
            match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => return err_json(&mut env, &format!("bad config json: {e}")),
            }
        };
        let mut slot = lock_awt();
        let out = awt_open_json(&mut slot, &value);
        drop(slot);
        rc_to_json(&mut env, out)
    })
}

/// `RustBridge.awtClose(): String` — close the session and stop its pumps.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_awtClose(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    auth_ffi!({
        let mut slot = lock_awt();
        let out = awt_close_json(&mut slot);
        drop(slot);
        jstr(&mut env, &out.to_string())
    })
}

/// `RustBridge.awtInfo(): String` — session + transport snapshot
/// (`{"open":false}` when no session is running).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_awtInfo(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    auth_ffi!({
        let slot = lock_awt();
        let out = awt_info_json(&slot);
        drop(slot);
        jstr(&mut env, &out.to_string())
    })
}

/// `RustBridge.awtConfigure(json): String` — surface / desktop size, scale mode,
/// focus and repaint requests: `{"surface":{"width":…,"height":…},"screen":{…},
/// "scale_mode":"fit","focus":true,"release_all":true,"clear":true,"fill":…}`.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_awtConfigure(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    auth_ffi!({
        let raw = match read_input(&mut env, &request_json) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request json"),
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad request json: {e}")),
        };
        let slot = lock_awt();
        let out = awt_configure_json(&slot, &value);
        drop(slot);
        rc_to_json(&mut env, out)
    })
}

/// `RustBridge.awtAttachTransport(json): String` — create + pump the named-pipe
/// channels of an already open session (`{"dir":…}` or `{"frames":…,"events":…}`).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_awtAttachTransport(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    auth_ffi!({
        let raw = match read_input(&mut env, &request_json) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request json"),
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad request json: {e}")),
        };
        let mut slot = lock_awt();
        let out = awt_attach_transport_json(&mut slot, &value);
        drop(slot);
        rc_to_json(&mut env, out)
    })
}

/// `RustBridge.awtInput(json): String` — a batch of Compose input events
/// (`{"events":[{"type":"pointer","phase":"down","x":…,"y":…,"button":"left"},
///   {"type":"key_down","name":"escape"},{"type":"text","text":"hi"}]}`).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_awtInput(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    auth_ffi!({
        let raw = match read_input(&mut env, &request_json) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request json"),
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad request json: {e}")),
        };
        let slot = lock_awt();
        let out = awt_input_json(&slot, &value);
        drop(slot);
        rc_to_json(&mut env, out)
    })
}

/// `RustBridge.awtSubmitFrame(frame): String` — hand one encoded `RCAF` frame to
/// the session (used when Kotlin owns the transport instead of the Rust pump).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_awtSubmitFrame(
    mut env: JNIEnv,
    _class: JClass,
    frame: JByteArray,
) -> jstring {
    auth_ffi!({
        let bytes = match env.convert_byte_array(&frame) {
            Ok(b) => b,
            Err(e) => return err_json(&mut env, &format!("cannot read the frame: {e}")),
        };
        let slot = lock_awt();
        let out = awt_submit_frame_json(&slot, &bytes);
        drop(slot);
        rc_to_json(&mut env, out)
    })
}

/// `RustBridge.awtPollFrame(buffer): String` — the *hot path*: convert the
/// damaged region straight into `buffer`, which must be the **direct**
/// `ByteBuffer` backing the Compose `Bitmap` (`Bitmap.copyPixelsFromBuffer`).
///
/// Zero copy: the pixels never pass through a Java array. Returns
/// `{"changed":false}` when nothing changed, so the UI can skip both the upload
/// and the recomposition.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_awtPollFrame(
    mut env: JNIEnv,
    _class: JClass,
    buffer: JByteBuffer,
) -> jstring {
    auth_ffi!({
        let address = match env.get_direct_buffer_address(&buffer) {
            Ok(p) if !p.is_null() => p,
            _ => {
                return err_json(
                    &mut env,
                    "awtPollFrame needs a direct ByteBuffer (ByteBuffer.allocateDirect)",
                )
            }
        };
        let capacity = match env.get_direct_buffer_capacity(&buffer) {
            Ok(c) => c,
            Err(e) => return err_json(&mut env, &format!("cannot size the ByteBuffer: {e}")),
        };
        // SAFETY: `address` / `capacity` describe a direct ByteBuffer owned by
        // the caller, which stays alive for the duration of this call. The
        // session validates every write against `capacity` (a short buffer is an
        // error, never an overflow).
        let dst = unsafe { std::slice::from_raw_parts_mut(address, capacity) };
        let slot = lock_awt();
        let out = awt_poll_frame_json(&slot, dst);
        drop(slot);
        rc_to_json(&mut env, out)
    })
}

/// `RustBridge.awtPollFrameArray(buffer): String` — same as `awtPollFrame` for
/// callers that cannot allocate a direct buffer (one extra copy each way).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_awtPollFrameArray(
    mut env: JNIEnv,
    _class: JClass,
    buffer: JByteArray,
) -> jstring {
    auth_ffi!({
        let len = match env.get_array_length(&buffer) {
            Ok(n) if n >= 0 => n as usize,
            _ => return err_json(&mut env, "cannot size the frame buffer"),
        };
        // The framebuffer is *persistent* (only damaged rows are rewritten), so
        // the current contents have to be read back before polling.
        let mut bytes = vec![0u8; len];
        {
            let view =
                unsafe { std::slice::from_raw_parts_mut(bytes.as_mut_ptr() as *mut i8, len) };
            if let Err(e) = env.get_byte_array_region(&buffer, 0, view) {
                return err_json(&mut env, &format!("cannot read the frame buffer: {e}"));
            }
        }
        let slot = lock_awt();
        let out = awt_poll_frame_json(&slot, &mut bytes);
        drop(slot);
        let changed = matches!(&out, Ok(v) if v["changed"] == json!(true));
        if changed {
            let view = unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const i8, len) };
            if let Err(e) = env.set_byte_array_region(&buffer, 0, view) {
                return err_json(&mut env, &format!("cannot write the frame buffer: {e}"));
            }
        }
        rc_to_json(&mut env, out)
    })
}

/// `RustBridge.awtDrainEvents(): ByteArray` — the queued AWT records as 32-byte
/// little-endian rows, for a Kotlin-side transport. Empty when no session is
/// open, so the caller never has to null-check.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_awtDrainEvents(
    env: JNIEnv,
    _class: JClass,
) -> jbyteArray {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let slot = lock_awt();
        let bytes = awt_drain_events_bytes(&slot);
        drop(slot);
        match env.byte_array_from_slice(&bytes) {
            Ok(array) => array.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }));
    match built {
        Ok(a) => a,
        Err(_) => std::ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options_json(root: &std::path::Path) -> serde_json::Value {
        json!({
            "game_dir": root.join(".minecraft"),
            "data_root": root,
            "java_home": root.join("jre17"),
            "java_version": "jre17",
            "account": {
                "username": "Steve", "uuid": "0-0-0-0",
                "access_token": "0", "user_type": "offline"
            },
            "use_cacio": false
        })
    }

    fn version_json() -> serde_json::Value {
        json!({
            "id": "1.20.4",
            "type": "release",
            "mainClass": "net.minecraft.client.main.Main",
            "libraries": [{ "name": "com.mojang:patchy:1.3.9" }],
            "arguments": {
                "game": ["--username", "${auth_player_name}", "--version", "${version_name}"],
                "jvm": ["-cp", "${classpath}"]
            }
        })
    }

    #[test]
    fn preview_builds_a_command_without_touching_the_disk() {
        let req = json!({
            "options": options_json(std::path::Path::new("/data/mc")),
            "version": version_json()
        });
        let out = launch_preview_json(&req).unwrap();
        assert_eq!(out["version_id"], "1.20.4");
        assert_eq!(out["main_class"], "net.minecraft.client.main.Main");
        assert_eq!(out["program"], "/data/mc/jre17/bin/java");
        let jvm: Vec<String> = serde_json::from_value(out["jvm_args"].clone()).unwrap();
        assert!(jvm.iter().any(|a| a == "-cp"));
        assert!(jvm.iter().any(|a| a.starts_with("-Djava.library.path=")));
        let game: Vec<String> = serde_json::from_value(out["game_args"].clone()).unwrap();
        assert_eq!(game[0], "--username");
        assert_eq!(game[1], "Steve");
        assert!(out["command_line"].as_str().unwrap().contains("java"));
    }

    #[test]
    fn preview_preflight_reports_missing_files() {
        let req = json!({
            "options": options_json(std::path::Path::new("/data/definitely-missing")),
            "version": version_json(),
            "preflight": true
        });
        let err = launch_preview_json(&req).unwrap_err();
        assert!(
            err.to_string().contains("java executable not found"),
            "{err}"
        );
    }

    #[test]
    fn preview_rejects_malformed_requests() {
        assert!(launch_preview_json(&json!({}))
            .unwrap_err()
            .to_string()
            .contains("options"));
        assert!(
            launch_preview_json(&json!({ "options": { "game_dir": "/a" } }))
                .unwrap_err()
                .to_string()
                .contains("json error")
        );
        let req = json!({ "options": options_json(std::path::Path::new("/data/mc")) });
        assert!(launch_preview_json(&req)
            .unwrap_err()
            .to_string()
            .contains("version"));
    }

    #[test]
    fn diagnose_classifies_a_log() {
        let out = launch_diagnose_json(&json!({
            "exit_code": 1,
            "log": "[main/INFO]: hi\njava.lang.OutOfMemoryError: Java heap space"
        }));
        assert_eq!(out["category"], "out_of_memory");
        assert_eq!(out["crashed"], true);
        assert!(out["advice_zh"].as_str().unwrap().contains("内存"));

        // an empty request is a clean exit, not a panic
        let out = launch_diagnose_json(&json!({}));
        assert_eq!(out["category"], "unknown");
        let out = launch_diagnose_json(&json!({ "exit_code": 0 }));
        assert_eq!(out["category"], "clean_exit");
        // launcher-initiated stop
        let out = launch_diagnose_json(&json!({ "exit_code": 143, "requested_stop": true }));
        assert_eq!(out["category"], "user_terminated");
    }

    #[test]
    fn renderer_catalogue_is_complete() {
        let out = launch_renderers_json();
        let arr = out.as_array().unwrap();
        assert_eq!(arr.len(), 5);
        assert_eq!(arr[0]["id"], "opengles2");
        assert_eq!(arr[0]["gl_libname"], "libgl4es_114.so");
        assert_eq!(arr[0]["env"]["LIBGL_ES"], "2");
        assert!(arr
            .iter()
            .all(|r| r["id"].is_string() && r["gl_libname"].is_string()));
    }
}

#[cfg(test)]
mod awt_tests {
    use super::*;
    use crate::launch::awt::AwtFrame;

    /// A tiny 4×2 desktop on an 8×4 surface (exact 2× scale, no letterbox bars).
    fn open_tiny(slot: &mut AwtSlot) -> serde_json::Value {
        awt_open_json(
            slot,
            &json!({
                "screen": { "width": 4, "height": 2 },
                "surface": { "width": 8, "height": 4 },
                "java_version": "jre17"
            }),
        )
        .expect("open")
    }

    fn frame_bytes(argb: u32) -> Vec<u8> {
        AwtFrame::full(1, 4, 2, vec![argb; 8]).unwrap().encode()
    }

    #[test]
    fn open_reports_the_session_and_picks_the_cacio_backend() {
        let mut slot: AwtSlot = None;
        let out = open_tiny(&mut slot);
        assert_eq!(out["screen"]["width"], 4);
        assert_eq!(out["surface"]["height"], 4);
        assert_eq!(out["backend"], "cacio17", "java 17 -> caciocavallo17");
        assert_eq!(out["scale_mode"], "fit");
        assert_eq!(out["link"]["state"], "detached");
        assert_eq!(out["rgba_len"], 4 * 2 * 4);

        // `awtInfo` mirrors it and adds the `open` flag.
        let info = awt_info_json(&slot);
        assert_eq!(info["open"], true);
        assert_eq!(info["screen"]["width"], 4);
    }

    #[test]
    fn open_defaults_to_a_720p_desktop_and_sanitizes_absurd_values() {
        let mut slot: AwtSlot = None;
        let out = awt_open_json(&mut slot, &serde_json::Value::Null).unwrap();
        assert_eq!(out["screen"]["width"], 1280);
        assert_eq!(out["screen"]["height"], 720);

        let out = awt_open_json(
            &mut slot,
            &json!({ "screen": { "width": 0, "height": 999999 } }),
        )
        .unwrap();
        assert_eq!(
            out["screen"]["width"], 1,
            "a zero-wide desktop is impossible"
        );
        assert_eq!(out["screen"]["height"], 8192, "clamped to MAX_CANVAS_DIM");
    }

    #[test]
    fn open_rejects_a_malformed_config_without_touching_the_old_session() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        let err = awt_open_json(&mut slot, &json!({ "scale_mode": "diagonally" })).unwrap_err();
        assert!(err.to_string().contains("bad AWT session config"), "{err}");
        let err = awt_open_json(&mut slot, &json!({ "java_version": "jre99" })).unwrap_err();
        assert!(err.to_string().contains("bad java_version"), "{err}");
    }

    #[test]
    fn open_replaces_the_previous_session_with_a_clean_canvas() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        awt_submit_frame_json(&slot, &frame_bytes(0xFF00_FF00)).unwrap();
        let out = open_tiny(&mut slot);
        assert_eq!(out["session"]["frames_accepted"], 0, "counters start fresh");
        assert_eq!(
            awt_info_json(&slot)["canvas"]["frames_presented"],
            0,
            "so does the canvas"
        );
    }

    #[test]
    fn every_call_without_a_session_is_an_error_not_a_panic() {
        let mut slot: AwtSlot = None;
        assert_eq!(awt_info_json(&slot), json!({ "open": false }));
        assert_eq!(awt_close_json(&mut slot), json!({ "closed": false }));
        for err in [
            awt_configure_json(&slot, &json!({ "focus": true })).unwrap_err(),
            awt_input_json(&slot, &json!({ "events": [] })).unwrap_err(),
            awt_submit_frame_json(&slot, &frame_bytes(0)).unwrap_err(),
            awt_poll_frame_json(&slot, &mut [0u8; 32]).unwrap_err(),
            awt_attach_transport_json(&mut slot, &json!({ "dir": "/tmp/nope" })).unwrap_err(),
        ] {
            assert!(err.to_string().contains("no AWT session is open"), "{err}");
        }
        assert!(awt_drain_events_bytes(&slot).is_empty());
    }

    #[test]
    fn close_stops_the_session_and_is_idempotent() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        let out = awt_close_json(&mut slot);
        assert_eq!(out["closed"], true);
        // No transport was ever attached, so the link never left "detached" —
        // but the stop reason is recorded either way.
        assert_eq!(out["link"]["state"], "detached");
        assert_eq!(out["link"]["reason"], "stopped by the launcher");
        assert_eq!(awt_close_json(&mut slot), json!({ "closed": false }));
    }

    #[test]
    fn configure_applies_geometry_scale_focus_and_repaints() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        // Consume the initial full-screen damage first.
        let mut fb = vec![0u8; 32];
        awt_poll_frame_json(&slot, &mut fb).unwrap();

        let out = awt_configure_json(
            &slot,
            &json!({
                "surface": { "width": 16, "height": 8 },
                "scale_mode": "stretch",
                "focus": false
            }),
        )
        .unwrap();
        assert_eq!(out["surface"]["width"], 16);
        assert_eq!(out["scale_mode"], "stretch");
        assert_eq!(out["focused"], false);
        assert_eq!(out["placement"], json!({"x":0,"y":0,"width":16,"height":8}));

        // A desktop resize reallocates the canvas and tells the JVM.
        let out = awt_configure_json(&slot, &json!({ "screen": [8, 4] })).unwrap();
        assert_eq!(out["screen"]["width"], 8);
        assert_eq!(out["rgba_len"], 8 * 4 * 4);
        assert!(
            out["pending_events"].as_u64().unwrap() >= 1,
            "COMPONENT_RESIZED"
        );

        // `fill` repaints everything, so the next poll uploads the whole desktop.
        awt_configure_json(&slot, &json!({ "fill": 0xFFFF_0000u32 })).unwrap();
        let mut fb = vec![0u8; 8 * 4 * 4];
        let out = awt_poll_frame_json(&slot, &mut fb).unwrap();
        assert_eq!(out["changed"], true);
        assert_eq!(&fb[0..4], &[0xFF, 0x00, 0x00, 0xFF]);

        // A bogus size / mode is a clean error.
        assert!(awt_configure_json(&slot, &json!({ "surface": "big" }))
            .unwrap_err()
            .to_string()
            .contains("expected a size"));
        assert!(awt_configure_json(&slot, &json!({ "scale_mode": 7 }))
            .unwrap_err()
            .to_string()
            .contains("bad scale_mode"));
    }

    #[test]
    fn an_input_batch_queues_awt_records_and_reports_the_state() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        let out = awt_input_json(
            &slot,
            &json!({ "events": [
                { "type": "pointer", "phase": "down", "x": 4.0, "y": 2.0, "button": "left" },
                { "type": "pointer", "phase": "up", "x": 4.0, "y": 2.0, "button": 1 },
                { "type": "key_down", "name": "left.shift" },
                { "type": "key_down", "code": 65 },
                { "type": "text", "text": "hi" },
                { "type": "scroll", "x": 4.0, "y": 2.0, "ticks": -2 }
            ]}),
        )
        .unwrap();
        // press + release + synthetic click + 2 keys + 2 typed chars + wheel
        assert_eq!(out["queued"], 8);
        assert_eq!(out["pending"], 8);
        assert_eq!(out["pointer"], json!({ "x": 2, "y": 1 }));
        assert_eq!(out["focused"], true);
        assert_eq!(out["rejected"], json!([]));
        // SHIFT is held, so the modifier mask is non-zero.
        assert_ne!(out["modifiers"], 0);

        // Draining hands them to the JVM as 32-byte records.
        let bytes = awt_drain_events_bytes(&slot);
        assert_eq!(bytes.len(), 8 * 32);
        assert_eq!(
            awt_input_json(&slot, &json!({ "events": [] })).unwrap()["pending"],
            0
        );
    }

    #[test]
    fn a_single_bad_event_does_not_drop_the_rest_of_the_batch() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        let out = awt_input_json(
            &slot,
            &json!({ "events": [
                { "type": "wiggle" },
                { "phase": "down" },
                { "type": "pointer", "phase": "sideways", "x": 1.0, "y": 1.0 },
                { "type": "key_up" },
                { "type": "key_down", "name": "escape" }
            ]}),
        )
        .unwrap();
        assert_eq!(out["queued"], 1, "the good event still reached the JVM");
        assert_eq!(out["rejected"].as_array().unwrap().len(), 4);
        assert!(out["rejected"][0].as_str().unwrap().contains("wiggle"));

        // A bare event object (no "events" wrapper) is accepted too.
        let out = awt_input_json(&slot, &json!({ "type": "release_all" })).unwrap();
        assert!(out["queued"].as_u64().unwrap() <= 1);
        // A tap on the letterbox bar queues nothing but is not an error.
        awt_configure_json(&slot, &json!({ "surface": { "width": 8, "height": 40 } })).unwrap();
        let out = awt_input_json(
            &slot,
            &json!([{ "type": "pointer", "phase": "down", "x": 4.0, "y": 39.0 }]),
        )
        .unwrap();
        assert_eq!(out["queued"], 0);
        assert_eq!(out["rejected"], json!([]));
    }

    #[test]
    fn submit_and_poll_round_trip_pixels_as_rgba() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        let mut fb = vec![0u8; 4 * 2 * 4];
        // The freshly opened canvas is fully damaged (opaque black).
        let out = awt_poll_frame_json(&slot, &mut fb).unwrap();
        assert_eq!(out["changed"], true);
        assert_eq!(out["width"], 4);
        assert_eq!(&fb[0..4], &[0x00, 0x00, 0x00, 0xFF]);
        // Nothing changed since => no upload, no recomposition.
        assert_eq!(
            awt_poll_frame_json(&slot, &mut fb).unwrap(),
            json!({ "changed": false })
        );

        let out = awt_submit_frame_json(&slot, &frame_bytes(0xFF10_2030)).unwrap();
        assert_eq!(out["changed"], true);
        assert_eq!(out["bytes"], 4 * 2 * 4);
        let out = awt_poll_frame_json(&slot, &mut fb).unwrap();
        assert_eq!(out["changed"], true);
        assert_eq!(&fb[0..4], &[0x10, 0x20, 0x30, 0xFF]);

        // A corrupt frame is an error, never a panic — and it is accounted for.
        assert!(awt_submit_frame_json(&slot, b"not a frame").is_err());
        assert_eq!(
            awt_info_json(&slot)["session"]["frames_rejected"],
            1,
            "the rejected frame is accounted for"
        );
        // A framebuffer too small for the damage is rejected instead of
        // overflowing (the previous frame is still pending an upload).
        awt_submit_frame_json(&slot, &frame_bytes(0xFF00_00FF)).unwrap();
        let err = awt_poll_frame_json(&slot, &mut [0u8; 4]).unwrap_err();
        assert!(err.to_string().contains("too small"), "{err}");
        // The damage survives a failed upload, so the next (correct) poll still
        // delivers the frame instead of losing it.
        let out = awt_poll_frame_json(&slot, &mut fb).unwrap();
        assert_eq!(out["changed"], true);
        assert_eq!(&fb[0..4], &[0x00, 0x00, 0xFF, 0xFF]);
    }

    /// Contract test for the Compose layer: the snapshot must carry every key
    /// `app/src/main/java/com/rc/launcher/ui/awt/AwtSessionInfo.kt` reads. CI
    /// cannot run the Kotlin unit tests, so this is what keeps the two halves of
    /// the bridge from drifting apart.
    #[test]
    fn the_snapshot_carries_every_field_the_compose_layer_parses() {
        let dir = tempfile::tempdir().unwrap();
        let mut slot: AwtSlot = None;
        let out = awt_open_json(
            &mut slot,
            &json!({
                "screen": { "width": 4, "height": 2 },
                "surface": { "width": 8, "height": 4 },
                "transport": { "dir": dir.path().to_string_lossy() }
            }),
        )
        .unwrap();

        for key in [
            "backend",
            "screen",
            "surface",
            "scale_mode",
            "placement",
            "focused",
            "modifiers",
            "pending_events",
            "rgba_len",
            "uptime_ms",
            "canvas",
            "session",
            "link",
            "transport",
        ] {
            assert!(out.get(key).is_some(), "snapshot is missing {key}");
        }
        for key in ["x", "y", "width", "height"] {
            assert!(out["placement"].get(key).is_some(), "placement.{key}");
        }
        for key in ["fps", "frames_presented", "frames_dropped"] {
            assert!(out["canvas"].get(key).is_some(), "canvas.{key}");
        }
        for key in ["frames_accepted", "frames_rejected", "events_dropped"] {
            assert!(out["session"].get(key).is_some(), "session.{key}");
        }
        for key in [
            "state",
            "frames_accepted",
            "frames_rejected",
            "events_written",
            "events_lost",
            "reason",
        ] {
            assert!(out["link"].get(key).is_some(), "link.{key}");
        }
        for key in ["protocol", "frames", "events"] {
            assert!(out["transport"].get(key).is_some(), "transport.{key}");
        }

        // The `awtInfo` flag, the poll result and the input result too.
        assert_eq!(awt_info_json(&slot)["open"], true);
        let mut fb = vec![0u8; 4 * 2 * 4];
        let poll = awt_poll_frame_json(&slot, &mut fb).unwrap();
        for key in ["changed", "x", "y", "width", "height", "bytes"] {
            assert!(poll.get(key).is_some(), "poll.{key}");
        }
        let input =
            awt_input_json(&slot, &json!({ "events": [{ "type": "release_all" }] })).unwrap();
        for key in [
            "queued",
            "pending",
            "modifiers",
            "focused",
            "pointer",
            "rejected",
        ] {
            assert!(input.get(key).is_some(), "input.{key}");
        }
        assert!(input["pointer"].get("x").is_some());
        assert_eq!(awt_close_json(&mut slot)["closed"], true);
    }

    #[test]
    fn transport_paths_are_parsed_from_a_dir_or_an_explicit_pair() {
        let t = parse_transport(&json!({ "dir": "/data/awt" })).unwrap();
        assert_eq!(t.frames, std::path::Path::new("/data/awt/awt-frames.rcaf"));
        assert_eq!(t.events, std::path::Path::new("/data/awt/awt-events.rcae"));
        assert_eq!(parse_transport(&json!("/data/awt")).unwrap(), t);
        let t = parse_transport(&json!({ "frames": "/a/f", "events": "/a/e" })).unwrap();
        assert_eq!(t.frames, std::path::Path::new("/a/f"));
        assert!(parse_transport(&json!({ "frames": "/a/f" }))
            .unwrap_err()
            .to_string()
            .contains("both \"frames\" and \"events\""));
    }

    #[test]
    fn opening_with_a_transport_creates_and_pumps_the_channels() {
        use std::os::unix::fs::FileTypeExt;
        let dir = tempfile::tempdir().unwrap();
        let mut slot: AwtSlot = None;
        let out = awt_open_json(
            &mut slot,
            &json!({
                "screen": { "width": 4, "height": 2 },
                "surface": { "width": 8, "height": 4 },
                "transport": { "dir": dir.path().to_string_lossy() }
            }),
        )
        .unwrap();
        assert_eq!(out["transport"]["protocol"], "rcaf1");
        assert!(out["transport"]["frames"]
            .as_str()
            .unwrap()
            .ends_with("awt-frames.rcaf"));
        for name in ["awt-frames.rcaf", "awt-events.rcae"] {
            let meta = std::fs::metadata(dir.path().join(name)).unwrap();
            assert!(meta.file_type().is_fifo(), "{name} must be a named pipe");
        }
        // Closing joins the pump threads, so the channels are nobody's any more.
        assert_eq!(awt_close_json(&mut slot)["closed"], true);
    }
}

// === Internationalisation FFI (task 20) ====================================
//
// JSON-in / JSON-out, mirroring the rest of the bridge. The *pure* helpers below
// carry all the logic so they can be unit-tested on the host without a JVM; the
// JNI entry points are thin `catch_unwind` wrappers.
//
// The Compose layer uses these to (a) populate the language picker, (b) apply a
// language instantly, and (c) hydrate its string table from the core so the UI
// and the core never disagree about an error or crash message.

/// Pure core of `RustBridge.i18nLanguages` — the language catalogue for the
/// settings picker (tag, endonym, completeness, Android qualifier).
pub fn i18n_languages_json() -> serde_json::Value {
    serde_json::json!({
        "base": crate::i18n::Language::BASE.tag(),
        "current": crate::i18n::current_language().tag(),
        "languages": crate::i18n::available_languages(),
    })
}

/// Pure core of `RustBridge.i18nSetLanguage`.
///
/// `request` = `{ "tag": "zh-Hant" }` or `{ "preferred": ["zh-Hant-TW", "en"] }`
/// (an Android `LocaleList`). An unknown/absent value resolves to the
/// Chinese-first base locale rather than failing.
pub fn i18n_set_language_json(request: &serde_json::Value) -> serde_json::Value {
    let chosen = if let Some(list) = request.get("preferred").and_then(|v| v.as_array()) {
        let tags: Vec<&str> = list.iter().filter_map(|v| v.as_str()).collect();
        crate::i18n::set_language_from_preferences(tags)
    } else {
        let tag = request.get("tag").and_then(|v| v.as_str()).unwrap_or("");
        crate::i18n::set_language_tag(tag)
    };
    serde_json::json!({
        "tag": chosen.tag(),
        "native_name": chosen.native_name(),
        "english_name": chosen.english_name(),
        "android_qualifier": chosen.android_qualifier(),
        "rtl": chosen.is_rtl(),
        "base": chosen == crate::i18n::Language::BASE,
    })
}

/// Resolve the language a request refers to: an explicit `language` tag,
/// otherwise the current UI language.
fn requested_language(request: &serde_json::Value) -> crate::i18n::Language {
    match request.get("language").and_then(|v| v.as_str()) {
        Some(tag) => crate::i18n::Language::from_tag(tag)
            .or_else(|| crate::i18n::Language::negotiate(tag))
            .unwrap_or_else(crate::i18n::current_language),
        None => crate::i18n::current_language(),
    }
}

/// Pure core of `RustBridge.i18nTranslate`.
///
/// `request` = `{ "key": "error.checksum", "language"?: "en",
///                "args"?: { "path": "/sdcard/x.jar" }, "count"?: 3 }`
///
/// With `count` the key is treated as a plural *base* key (`<key>.one` /
/// `<key>.other` per the language's CLDR rules) and `{count}` is provided
/// automatically. A missing key echoes back as `value == key` with
/// `"missing": true`, so the UI can still render *something*.
pub fn i18n_translate_json(request: &serde_json::Value) -> serde_json::Value {
    let language = requested_language(request);
    let key = request.get("key").and_then(|v| v.as_str()).unwrap_or("");
    if key.is_empty() {
        return serde_json::json!({
            "key": "",
            "value": "",
            "language": language.tag(),
            "missing": true,
        });
    }

    // Collect `args` (numbers/bools are stringified so the UI can pass raw JSON).
    let mut owned: Vec<(String, String)> = Vec::new();
    if let Some(map) = request.get("args").and_then(|v| v.as_object()) {
        for (k, v) in map {
            let s = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            };
            owned.push((k.clone(), s));
        }
    }
    let count = request.get("count").and_then(|v| v.as_i64());
    if let Some(n) = count {
        // An explicit `args.count` wins over the derived one.
        if !owned.iter().any(|(k, _)| k == "count") {
            owned.push(("count".to_string(), n.to_string()));
        }
    }
    let args: Vec<(&str, &str)> = owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let effective_key = match count {
        Some(n) => crate::i18n::format::plural_key(language, key, n),
        None => key.to_string(),
    };
    let missing = !crate::i18n::has_key(language, &effective_key);
    serde_json::json!({
        "key": effective_key,
        "value": crate::i18n::t_args_in(language, &effective_key, &args),
        "language": language.tag(),
        "missing": missing,
    })
}

/// Pure core of `RustBridge.i18nBundle` — the *whole* resolved catalogue.
///
/// `request` = `{ "language"?: "en" }`. Handing Kotlin the full map once (rather
/// than crossing the JNI boundary per string) keeps the UI allocation-free while
/// scrolling and guarantees it renders the same copy as the core.
pub fn i18n_bundle_json(request: &serde_json::Value) -> serde_json::Value {
    let language = requested_language(request);
    serde_json::json!({
        "language": language.tag(),
        "messages": crate::i18n::bundle_json(language),
    })
}

/// Pure core of `RustBridge.i18nDiagnostics` — catalogue health (missing keys,
/// placeholder drift, parse problems, runtime misses).
pub fn i18n_diagnostics_json() -> serde_json::Value {
    crate::i18n::diagnostics()
}

/// Pure core of `RustBridge.i18nOverlay` — install / clear a runtime overlay.
///
/// `request` = `{ "action": "install"|"dir"|"clear",
///                "language"?: "en", "text"?: "...", "path"?: "/data/.../i18n" }`
///
/// This is how a community translation or a wording hot-fix ships without a new
/// APK. Never fails: an unreadable path or a malformed document installs
/// nothing (task-19 robustness contract).
pub fn i18n_overlay_json(request: &serde_json::Value) -> serde_json::Value {
    let action = request
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("install");
    let installed = match action {
        "clear" => {
            crate::i18n::clear_overlay();
            0
        }
        "dir" => match request.get("path").and_then(|v| v.as_str()) {
            Some(p) => crate::i18n::load_overlay_dir(p),
            None => 0,
        },
        _ => {
            let language = requested_language(request);
            match request.get("text").and_then(|v| v.as_str()) {
                Some(text) => crate::i18n::install_overlay_text(language, text),
                None => 0,
            }
        }
    };
    serde_json::json!({
        "action": action,
        "installed": installed,
        "overlay_active": crate::i18n::catalog::has_overlay(),
    })
}

/// `RustBridge.i18nLanguages(): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_i18nLanguages(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let json = i18n_languages_json().to_string();
        match env.new_string(json) {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }));
    built.unwrap_or(std::ptr::null_mut())
}

/// `RustBridge.i18nCurrentLanguage(): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_i18nCurrentLanguage(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match env.new_string(crate::i18n::current_language().tag()) {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }));
    built.unwrap_or(std::ptr::null_mut())
}

/// Read a `JString` argument, parse it as JSON, run `f`, return the result as a
/// `jstring`. Absent / malformed input degrades to `null` JSON so every helper
/// still applies its own defaults instead of failing.
fn i18n_json_call(
    env: &mut JNIEnv,
    arg: &JString,
    f: impl FnOnce(&serde_json::Value) -> serde_json::Value,
) -> jstring {
    let raw = read_input(env, arg).unwrap_or_default();
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
    let out = f(&value).to_string();
    match env.new_string(out) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.i18nSetLanguage(requestJson): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_i18nSetLanguage(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    auth_ffi!({ i18n_json_call(&mut env, &request, i18n_set_language_json) })
}

/// `RustBridge.i18nTranslate(requestJson): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_i18nTranslate(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    auth_ffi!({ i18n_json_call(&mut env, &request, i18n_translate_json) })
}

/// `RustBridge.i18nBundle(requestJson): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_i18nBundle(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    auth_ffi!({ i18n_json_call(&mut env, &request, i18n_bundle_json) })
}

/// `RustBridge.i18nDiagnostics(): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_i18nDiagnostics(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let json = i18n_diagnostics_json().to_string();
        match env.new_string(json) {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }));
    built.unwrap_or(std::ptr::null_mut())
}

/// `RustBridge.i18nOverlay(requestJson): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_i18nOverlay(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    auth_ffi!({ i18n_json_call(&mut env, &request, i18n_overlay_json) })
}

#[cfg(test)]
mod i18n_tests {
    use super::*;
    use crate::i18n::Language;

    /// Serialises against every other test that touches the global i18n state.
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        crate::i18n::GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn languages_json_lists_every_catalogue_base_first() {
        let _g = lock();
        let out = i18n_languages_json();
        assert_eq!(out["base"], "zh-CN", "中文优先");
        let langs = out["languages"].as_array().unwrap();
        assert_eq!(langs.len(), 3);
        assert_eq!(langs[0]["tag"], "zh-CN");
        assert_eq!(langs[0]["native_name"], "简体中文");
        assert_eq!(langs[0]["base"], true);
        assert_eq!(langs[0]["android_qualifier"], serde_json::Value::Null);
        assert_eq!(langs[1]["android_qualifier"], "zh-rTW");
        assert_eq!(langs[2]["tag"], "en");
        for l in langs {
            assert_eq!(l["completeness"], 1.0);
            assert!(l["messages"].as_u64().unwrap() >= 90);
        }
    }

    #[test]
    fn set_language_accepts_a_tag_or_a_locale_list() {
        let _g = lock();
        let restore = crate::i18n::current_language();

        let out = i18n_set_language_json(&json!({ "tag": "zh_TW" }));
        assert_eq!(out["tag"], "zh-Hant");
        assert_eq!(out["native_name"], "繁體中文");
        assert_eq!(out["android_qualifier"], "zh-rTW");
        assert_eq!(out["base"], false);
        assert_eq!(crate::i18n::current_language(), Language::ZhHant);

        // An Android LocaleList: first supported preference wins.
        let out = i18n_set_language_json(&json!({ "preferred": ["ja-JP", "en-GB", "zh-CN"] }));
        assert_eq!(out["tag"], "en");
        assert_eq!(crate::i18n::current_language(), Language::En);

        // Nothing supported / nothing supplied -> Chinese-first base locale.
        assert_eq!(
            i18n_set_language_json(&json!({ "preferred": ["ko"] }))["tag"],
            "zh-CN"
        );
        assert_eq!(i18n_set_language_json(&json!({}))["tag"], "zh-CN");
        assert_eq!(
            i18n_set_language_json(&serde_json::Value::Null)["tag"],
            "zh-CN"
        );

        crate::i18n::set_language(restore);
    }

    #[test]
    fn translate_resolves_keys_args_and_plurals() {
        let _g = lock();
        let restore = crate::i18n::current_language();
        crate::i18n::set_language(Language::ZhCn);

        // Explicit language overrides the current one.
        let out = i18n_translate_json(&json!({ "key": "nav.home", "language": "en" }));
        assert_eq!(out["value"], "Home");
        assert_eq!(out["language"], "en");
        assert_eq!(out["missing"], false);

        // Falls back to the current UI language.
        assert_eq!(
            i18n_translate_json(&json!({ "key": "nav.home" }))["value"],
            "主页"
        );

        // Named args, including non-string JSON values.
        let out = i18n_translate_json(&json!({
            "key": "error.retry_scheduled",
            "language": "en",
            "args": { "attempt": 2, "delay_secs": 5 }
        }));
        assert_eq!(out["value"], "Retry 2 starts in 5 seconds");

        // Plurals: `count` selects the sub-key and feeds `{count}`.
        let out =
            i18n_translate_json(&json!({ "key": "download.files", "language": "en", "count": 1 }));
        assert_eq!(out["key"], "download.files.one");
        assert_eq!(out["value"], "1 file");
        let out =
            i18n_translate_json(&json!({ "key": "download.files", "language": "en", "count": 4 }));
        assert_eq!(out["value"], "4 files");
        // Chinese has one form for any count.
        let out = i18n_translate_json(
            &json!({ "key": "download.files", "language": "zh-CN", "count": 4 }),
        );
        assert_eq!(out["key"], "download.files.other");
        assert_eq!(out["value"], "共 4 个文件");

        crate::i18n::set_language(restore);
    }

    #[test]
    fn translate_degrades_gracefully_on_bad_input() {
        let _g = lock();
        // Missing key -> empty, flagged.
        let out = i18n_translate_json(&json!({}));
        assert_eq!(out["value"], "");
        assert_eq!(out["missing"], true);
        // Unknown key -> echoes the key so the UI shows *something* greppable.
        let out = i18n_translate_json(&json!({ "key": "no.such.key", "language": "en" }));
        assert_eq!(out["value"], "no.such.key");
        assert_eq!(out["missing"], true);
        // An unknown language tag falls back instead of failing.
        let out = i18n_translate_json(&json!({ "key": "nav.home", "language": "xx-YY" }));
        assert!(out["value"].as_str().is_some_and(|s| !s.is_empty()));
        // Null request.
        assert_eq!(
            i18n_translate_json(&serde_json::Value::Null)["missing"],
            true
        );
    }

    #[test]
    fn bundle_hands_over_the_whole_catalogue() {
        let _g = lock();
        let out = i18n_bundle_json(&json!({ "language": "zh-Hant" }));
        assert_eq!(out["language"], "zh-Hant");
        let msgs = out["messages"].as_object().unwrap();
        assert!(msgs.len() >= 90);
        assert_eq!(msgs["nav.home"], "主頁");
        assert_eq!(msgs["crash.out_of_memory.summary"], "遊戲記憶體耗盡");
        assert!(msgs.values().all(|v| v.is_string()));
        // Every language yields the same key set (no holes for the UI).
        let en = i18n_bundle_json(&json!({ "language": "en" }));
        assert_eq!(en["messages"].as_object().unwrap().len(), msgs.len());
    }

    #[test]
    fn diagnostics_are_clean_and_serialisable() {
        let _g = lock();
        let out = i18n_diagnostics_json();
        assert_eq!(out["base"], "zh-CN");
        assert!(out["languages"].as_array().unwrap().len() == 3);
        assert!(serde_json::to_string(&out).is_ok());
    }

    #[test]
    fn overlay_round_trips_through_the_ffi() {
        let _g = lock();
        crate::i18n::clear_overlay();

        let out = i18n_overlay_json(&json!({
            "action": "install",
            "language": "en",
            "text": "nav.home = Base\nnav.settings = Prefs\n"
        }));
        assert_eq!(out["installed"], 2);
        assert_eq!(out["overlay_active"], true);
        assert_eq!(
            i18n_translate_json(&json!({ "key": "nav.home", "language": "en" }))["value"],
            "Base"
        );
        // The bundle reflects the hot-fix, so the UI picks it up on next read.
        let msgs = i18n_bundle_json(&json!({ "language": "en" }));
        assert_eq!(msgs["messages"]["nav.home"], "Base");

        // From a directory ...
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("en.properties"), "nav.home = FromDir\n").unwrap();
        let out =
            i18n_overlay_json(&json!({ "action": "dir", "path": dir.path().to_string_lossy() }));
        assert_eq!(out["installed"], 1);
        assert_eq!(
            i18n_translate_json(&json!({ "key": "nav.home", "language": "en" }))["value"],
            "FromDir"
        );

        // ... and cleared.
        let out = i18n_overlay_json(&json!({ "action": "clear" }));
        assert_eq!(out["overlay_active"], false);
        assert_eq!(
            i18n_translate_json(&json!({ "key": "nav.home", "language": "en" }))["value"],
            "Home"
        );

        // Bad input never panics and installs nothing.
        assert_eq!(
            i18n_overlay_json(&json!({ "action": "dir" }))["installed"],
            0
        );
        assert_eq!(
            i18n_overlay_json(&json!({ "action": "install" }))["installed"],
            0
        );
        assert_eq!(
            i18n_overlay_json(&json!({ "action": "dir", "path": "/no/such/dir" }))["installed"],
            0
        );
        crate::i18n::clear_overlay();
    }

    #[test]
    fn launch_diagnose_is_localised() {
        let _g = lock();
        let restore = crate::i18n::current_language();
        let req = json!({
            "exit_code": 1,
            "log": "java.lang.OutOfMemoryError: Java heap space",
            "language": "zh-Hant"
        });
        let out = super::launch_diagnose_json(&req);
        assert_eq!(out["category"], "out_of_memory");
        assert_eq!(out["language"], "zh-Hant");
        assert_eq!(out["summary_localized"], "遊戲記憶體耗盡");
        assert!(out["advice_localized"].as_str().unwrap().contains("記憶體"));
        // The English fields stay put for the log / bug report.
        assert_eq!(out["summary"], "the game ran out of memory");

        // Without a tag it follows the current UI language ...
        crate::i18n::set_language(Language::En);
        let out = super::launch_diagnose_json(
            &json!({ "exit_code": 1, "log": "java.lang.OutOfMemoryError: Java heap space" }),
        );
        assert_eq!(out["language"], "en");
        assert_eq!(out["summary_localized"], "the game ran out of memory");
        // ... and an unparseable tag degrades to it too.
        let out =
            super::launch_diagnose_json(&json!({ "exit_code": 1, "log": "x", "language": "??" }));
        assert_eq!(out["language"], "en");
        crate::i18n::set_language(restore);
    }
}
