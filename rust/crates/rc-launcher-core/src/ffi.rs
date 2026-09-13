//! JNI bridge.
//!
//! Functions here follow the `Java_<package>_<Class>_<method>` naming
//! convention expected by `System.loadLibrary("rc_launcher")` from
//! `com.rc.launcher.core.RustBridge`. Every public Rust entry point is wrapped
//! in `catch_unwind` so a panic becomes a `null` return instead of an aborted
//! VM (defensive boundary — see task 19).

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jfloat, jint, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use jni::JavaVM;

use crate::net::default_mirrors;
use crate::{greet, net, VERSION};
use std::path::PathBuf;

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
use crate::auth::transport::{AuthTransport, ReqwestTransport};
use crate::auth::vault::{AesGcmVault, InsecureVault, SecretVault};
use crate::error::{RcError, RcResult};

static MANAGER: OnceLock<Mutex<AccountManager>> = OnceLock::new();

/// Fallback tokio runtime used by [`block_on_async`] when no current
/// runtime is in scope — i.e. the FFI is being called from a plain
/// (non-tokio) thread, the way Java calls it in production. Lazily
/// initialised on first use so unit tests that already run inside a
/// tokio test runtime never spin it up.
static FALLBACK_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

#[inline]
pub(crate) fn fallback_runtime() -> &'static tokio::runtime::Runtime {
    FALLBACK_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("init FFI fallback runtime")
    })
}

/// Run a future to completion on the current tokio runtime if one
/// is already active (e.g. inside a `#[tokio::test]`), otherwise
/// fall back to the FFI's process-wide multi-thread runtime.
///
/// `block_on` panics if the calling thread is already inside a runtime,
/// which would break every unit test that runs the FFI under a tokio
/// test runtime. This helper exists so the FFI can stay
/// synchronous-from-Java's-perspective while remaining testable.
pub(crate) fn block_on_async<F: std::future::Future>(fut: F) -> F::Output {
    // We're already inside a runtime — `block_in_place` requires a
    // multi-thread runtime, so probe the runtime flavour and only
    // call it when safe. Otherwise a direct `block_on` is fine: there
    // is no worker thread to block in a current-thread runtime.
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            // `block_in_place` panics on a current-thread runtime;
            // detect the runtime flavour by introspecting metrics
            // (multi-thread runtimes expose worker counts).
            let is_multi_thread = handle.metrics().num_workers() > 0;
            if is_multi_thread {
                tokio::task::block_in_place(move || handle.block_on(fut))
            } else {
                handle.block_on(fut)
            }
        }
        Err(_) => fallback_runtime().block_on(fut),
    }
}

pub(crate) fn manager() -> &'static Mutex<AccountManager> {
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

pub(crate) fn lock_manager() -> MutexGuard<'static, AccountManager> {
    manager().lock().unwrap_or_else(|e| e.into_inner())
}

/// Build an [`AccountManager`] from a config JSON value. Shared by the JNI
/// `authInit` entry point and the C-ABI `rc_auth_init` so both surfaces expose
/// exactly the same store-configuration contract. `cfg` accepts
/// `{ "path"?: string, "key_hex"?: string, "client_id"?: string,
///   "redirect_uri"?: string }`:
/// * with `path` the store is persisted on disk, and when `key_hex` is present
///   the on-disk JSON is sealed with AES-256-GCM (on Android `key_hex` is the
///   Keystore-held key surfaced by the FFI bridge);
/// * without `path` an in-memory store is used.
/// * `redirect_uri` (task 28) optionally sets a custom callback address for the
///   Microsoft browser redirect; the embedded `microsoft_auth.html` page can
///   serve as this address.
/// * `proxy` (task 28) optionally configures an HTTP/HTTPS/SOCKS5 proxy for
///   the auth transport so Microsoft/Xbox/Mojang calls can reach the network
///   through the Great Firewall.
pub(crate) fn auth_manager_from_config(cfg: &serde_json::Value) -> RcResult<AccountManager> {
    // Task 28: build the auth transport with proxy support so that the
    // Microsoft/Xbox/Mojang token-exchange calls (xbox_chain / minecraft_chain)
    // can punch through the Great Firewall when the user has configured a proxy.
    let transport: Arc<dyn AuthTransport> = if let Some(url) = cfg
        .get("proxy")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        let proxy = parse_proxy_url(url);
        Arc::new(ReqwestTransport::with_proxy(&proxy)?)
    } else {
        Arc::new(ReqwestTransport::with_defaults()?)
    };
    let client_id = cfg
        .get("client_id")
        .and_then(|v| v.as_str())
        .unwrap_or(microsoft::DEFAULT_CLIENT_ID)
        .to_string();
    let redirect_uri = cfg
        .get("redirect_uri")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let storage: Box<dyn TokenStorage> =
        if let Some(path) = cfg.get("path").and_then(|v| v.as_str()) {
            let vault: Box<dyn SecretVault> =
                if let Some(key_hex) = cfg.get("key_hex").and_then(|v| v.as_str()) {
                    match parse_hex_key(key_hex) {
                        Some(k) => match AesGcmVault::new(k) {
                            Ok(v) => Box::new(v),
                            Err(e) => return Err(crate::error::RcError::Auth(e.to_string())),
                        },
                        None => return Err(crate::error::RcError::Auth("invalid key_hex".into())),
                    }
                } else {
                    Box::new(InsecureVault)
                };
            Box::new(FileTokenStorage::with_vault(path, vault))
        } else {
            Box::new(MemoryTokenStorage::new())
        };
    let mut mgr = AccountManager::new(storage, transport, client_id)?;
    if let Some(ru) = redirect_uri {
        mgr.set_redirect_uri(ru);
    }
    Ok(mgr)
}

/// Parse a proxy URL string into a [`crate::net::ProxyConfig`], inferring the
/// variant from the URL scheme (task 28).
fn parse_proxy_url(url: &str) -> crate::net::ProxyConfig {
    if url.starts_with("socks5://") {
        crate::net::ProxyConfig::socks5(url)
    } else if url.starts_with("https://") {
        crate::net::ProxyConfig::https(url)
    } else {
        crate::net::ProxyConfig::http(url)
    }
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
        Err(e) => auth_err_json(env, &e),
    }
}

/// Like [`err_json`] but also attaches the mainland-China network fallback hint
/// (`cn_fallback_hint`) to the error payload when `e` is a network-level failure
/// (task 10). The Kotlin UI surfaces the hint when a login fails because
/// Microsoft / Xbox / Mojang (or a third-party auth server) is unreachable.
fn auth_err_json(env: &mut JNIEnv, e: &crate::error::RcError) -> jstring {
    let mut obj = json!({ "error": e.to_string() });
    if let Some(hint) = e.cn_login_fallback_hint() {
        obj["cn_fallback_hint"] = json!(hint);
    }
    jstr(env, &obj.to_string())
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
        let mgr = match auth_manager_from_config(&cfg) {
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
/// Includes `redirect_uri` in the response when a custom callback address was
/// configured via `authInit` (task 28), so the UI can pass it back to
/// `authCompleteMicrosoft` if needed.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_authBeginMicrosoft(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    auth_ffi!({
        let g = lock_manager();
        let challenge = block_on_async(g.begin_microsoft());
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
        let account = block_on_async(g.complete_microsoft(&challenge, |_| {}));
        drop(g);
        rc_to_json(
            &mut env,
            account.and_then(|a| {
                serde_json::to_value(&a).map_err(|e| crate::error::RcError::Auth(e.to_string()))
            }),
        )
    })
}

/// `RustBridge.authGetCallbackHtml(): String` — returns the embedded
/// `microsoft_auth.html` callback page (task 28). The caller can write this
/// to `assets/microsoft_auth.html` at first launch to guarantee the callback
/// asset exists even when the APK build omits it.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_authGetCallbackHtml(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let lang = crate::i18n::current_language();
        let html = crate::auth::callback_html(lang, true);
        env.new_string(html)
    }));
    match built {
        Ok(Ok(s)) => s.into_raw(),
        _ => std::ptr::null_mut(),
    }
}

/// `RustBridge.authDefaultRedirectUri(): String` — returns the default
/// `redirect_uri` for the embedded callback page (task 28).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_authDefaultRedirectUri(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        env.new_string(crate::auth::DEFAULT_REDIRECT_URI)
    }));
    match built {
        Ok(Ok(s)) => s.into_raw(),
        _ => std::ptr::null_mut(),
    }
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
        let account = block_on_async(g.refresh(&id));
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
        let account = block_on_async(g.ensure_fresh(&id));
        drop(g);
        rc_to_json(
            &mut env,
            account.and_then(|a| {
                serde_json::to_value(&a).map_err(|e| crate::error::RcError::Auth(e.to_string()))
            }),
        )
    })
}

/// `RustBridge.authBeginThirdParty(serverUrl): String` — JSON server metadata.
///
/// Discovers an external auth server's name + register links so the UI can show
/// them before the user types credentials (task 10).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_authBeginThirdParty(
    mut env: JNIEnv,
    _class: JClass,
    server_url: JString,
) -> jstring {
    auth_ffi!({
        let url = match read_input(&mut env, &server_url) {
            Some(s) => s,
            None => {
                return auth_err_json(
                    &mut env,
                    &crate::error::RcError::Auth("missing server url".into()),
                )
            }
        };
        let g = lock_manager();
        let info = block_on_async(g.begin_third_party(&url));
        drop(g);
        rc_to_json(
            &mut env,
            info.and_then(|i| {
                serde_json::to_value(&i).map_err(|e| crate::error::RcError::Auth(e.to_string()))
            }),
        )
    })
}

/// `RustBridge.authCompleteThirdParty(loginJson): String` — JSON account (or error).
///
/// Completes a third-party login (Authlib-Injector / token relay). Blocks while
/// authenticating against the external auth server; call from a background
/// thread. On a network-level failure the returned JSON carries `cn_fallback_hint`
/// (task 10).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_authCompleteThirdParty(
    mut env: JNIEnv,
    _class: JClass,
    login_json: JString,
) -> jstring {
    auth_ffi!({
        let raw = match read_input(&mut env, &login_json) {
            Some(s) => s,
            None => {
                return auth_err_json(
                    &mut env,
                    &crate::error::RcError::Auth("missing login config".into()),
                )
            }
        };
        let login: crate::auth::third_party::ThirdPartyLogin = match serde_json::from_str(&raw) {
            Ok(l) => l,
            Err(e) => {
                return auth_err_json(
                    &mut env,
                    &crate::error::RcError::Auth(format!("invalid third-party login: {e}")),
                )
            }
        };
        let mut g = lock_manager();
        let account = block_on_async(g.complete_third_party(&login));
        drop(g);
        rc_to_json(
            &mut env,
            account.and_then(|a| {
                serde_json::to_value(&a).map_err(|e| crate::error::RcError::Auth(e.to_string()))
            }),
        )
    })
}
/// `RustBridge.authFetchSkin(uuid): String` — JSON `SkinModel` (or `{"error":...}`).
///
/// Fetches skin + cape metadata (download URLs, model type, cache state) for a
/// Microsoft account from Mojang's session profile API. The PNG bytes are *not*
/// downloaded here — the UI fetches them from `skin_url`/`cape_url` and caches
/// them locally for offline display (task 22).
///
/// The account must have a valid Minecraft access token; call `authEnsureFresh`
/// first so the token is not expired by the time the session-profile request is
/// made. Blocks on network I/O — call from a background thread.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_authFetchSkin(
    mut env: JNIEnv,
    _class: JClass,
    uuid: JString,
) -> jstring {
    auth_ffi!({
        let id = match read_input(&mut env, &uuid) {
            Some(s) => s,
            None => return err_json(&mut env, "missing uuid"),
        };
        let g = lock_manager();
        let model = block_on_async(g.fetch_skin(&id));
        drop(g);
        rc_to_json(
            &mut env,
            model.and_then(|m| {
                serde_json::to_value(&m).map_err(|e| crate::error::RcError::Auth(e.to_string()))
            }),
        )
    })
}

/// `RustBridge.authUploadSkin(uuid, model, skinBase64): String` —
/// `{"ok":true}` or `{"error":...}`.
///
/// Uploads a custom skin for a Microsoft account to Mojang. `model` is
/// "slim" or "classic" (the format Mojang's PUT endpoint expects); `skin_base64`
/// is the raw PNG bytes base64-encoded. The account must have a valid Minecraft
/// access token (`authEnsureFresh` first). Blocks on network I/O — call from a
/// background thread. On a network-level failure the error JSON carries
/// `cn_fallback_hint` for mainland-China players (task 22 / task 10).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_authUploadSkin(
    mut env: JNIEnv,
    _class: JClass,
    uuid: JString,
    model: JString,
    skin_base64: JString,
) -> jstring {
    auth_ffi!({
        let id = match read_input(&mut env, &uuid) {
            Some(s) => s,
            None => return err_json(&mut env, "missing uuid"),
        };
        let model_str = match read_input(&mut env, &model) {
            Some(s) => s,
            None => return err_json(&mut env, "missing model"),
        };
        let b64 = match read_input(&mut env, &skin_base64) {
            Some(s) => s,
            None => return err_json(&mut env, "missing skin data"),
        };
        use base64::Engine;
        let skin_data = match base64::engine::general_purpose::STANDARD.decode(&b64) {
            Ok(b) => b,
            Err(e) => return err_json(&mut env, &format!("base64: {e}")),
        };
        let mut g = lock_manager();
        let result = block_on_async(g.upload_skin(&id, &model_str, &skin_data));
        drop(g);
        match result {
            Ok(()) => jstr(&mut env, &serde_json::json!({ "ok": true }).to_string()),
            Err(e) => auth_err_json(&mut env, &e),
        }
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
    let device_info: crate::launch::crash::DeviceInfo = request
        .get("device_info")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let report = diagnose(code, signal, log.lines(), requested_stop, device_info);
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

// === Screen orientation / adaptive layout FFI (task 9) =======================
//
// The Compose layer mirrors `display::WindowMetrics` locally (a rotation must
// not cost a JNI round trip per recomposition), so these two entry points exist
// to keep the mirror honest and to feed the settings picker / diagnostics from
// the core: `displayOrientations()` is the catalogue behind
// 「跟随系统 / 强制横屏 / 强制竖屏」, and `displayLayout()` resolves one concrete
// window size the same way the Kotlin `RcWindowInfo` does. The Kotlin parity
// test asserts the two answers are identical.

/// `RustBridge.displayOrientations(): String` — JSON array of the orientation
/// policies the launcher supports (`id` + `android_screen_orientation`).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_displayOrientations(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    auth_ffi!({
        jstr(
            &mut env,
            &crate::display::orientation_catalog_json().to_string(),
        )
    })
}

/// `RustBridge.displayLayout(requestJson): String` — resolve the adaptive-layout
/// decisions for one window.
///
/// `requestJson` = `{ "width_dp": u32, "height_dp": u32, "orientation"?: string }`.
/// The optional `orientation` is an [`crate::display::OrientationPolicy`] id; when
/// present, the reply also carries the `window` geometry the policy would force
/// (`orient`), which is what the launch options do with the game window.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_displayLayout(
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
        jstr(&mut env, &display_layout_json(&value).to_string())
    })
}

/// Pure core of [`Java_com_rc_launcher_core_RustBridge_displayLayout`] (unit-tested
/// without a JVM).
pub fn display_layout_json(request: &serde_json::Value) -> serde_json::Value {
    use crate::display::{OrientationPolicy, WindowMetrics};

    let dp = |key: &str, fallback: u32| -> u32 {
        request
            .get(key)
            .and_then(|v| v.as_u64())
            .map(|v| v.min(u64::from(u16::MAX)) as u32)
            .unwrap_or(fallback)
    };
    let default = WindowMetrics::default();
    let metrics = WindowMetrics::new(
        dp("width_dp", default.width_dp),
        dp("height_dp", default.height_dp),
    );
    let mut out = metrics.to_json();
    if let Some(id) = request.get("orientation").and_then(|v| v.as_str()) {
        let policy = OrientationPolicy::from_id_or_default(id);
        let window = policy.orient(crate::launch::WindowSize {
            width: metrics.width_dp,
            height: metrics.height_dp,
        });
        out["policy"] = policy.to_json();
        out["window"] = serde_json::json!({
            "width": window.width,
            "height": window.height,
        });
    }
    out
}

// === Discord Rich Presence FFI (task 5) ======================================
//
// JSON-in / JSON-out bridge for the `discord` subsystem. The Compose settings
// screen drives the bridge through these four entry points (configure / update /
// clear / status) plus `discordShutdown` for a full disconnect. Every entry
// point is wrapped in `catch_unwind` so a panic never aborts the VM, and each
// returns a JSON `DiscordStateInfo` snapshot the UI can render directly.

async fn build_version_list_client(
    request: &serde_json::Value,
) -> RcResult<crate::net::NetworkClient> {
    use crate::net::MirrorMode;
    let mut builder =
        crate::net::NetworkClientBuilder::default().mirrors(crate::net::default_mirrors());
    if let Some(mode) = request.get("mirror_mode").and_then(|v| v.as_str()) {
        let parsed = match mode {
            "all" => MirrorMode::All,
            "mirrors_only" => MirrorMode::MirrorsOnly,
            "auto" => MirrorMode::Auto,
            "off" => MirrorMode::Off,
            _ => MirrorMode::All,
        };
        builder = builder.mirror_mode(parsed);
    }
    // DNS: a quick DoH/system choice that the caller may opt into. We never
    // block on the DNS leg here - the network client already races
    // IPv4/IPv6 and applies the static overrides.
    let mut cfg = crate::net::NetworkConfig::default();
    if let Some(dns) = request.get("dns_mode") {
        if let Some(mode) = dns.get("mode").and_then(|v| v.as_str()) {
            match mode {
                "doh" => {
                    let urls: Vec<String> = dns
                        .get("servers")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_else(crate::net::default_doh_servers);
                    cfg.dns = crate::net::DnsConfig::doh(urls);
                }
                "system" => cfg.dns = crate::net::DnsConfig::system(),
                _ => {}
            }
        }
    }
    builder = builder.config(cfg);
    builder.build().await
}
// === Task 16: complete and auto-updating game-version list ===========
//
// Three FFI entry points cover the UI side of task 16:
//   * `gameFetchVersionList(request)`  - return the cached or freshly-fetched
//     manifest, always augmented with the built-in unlisted DB.
//   * `gameRefreshVersionList()`       - force a network refresh, ignoring TTL.
//   * `gameVersionListCacheInfo()`     - return the [VersionListInfo] struct so
//     the UI can render a "stale" / "offline-only" badge.
//
// The process-wide cache + mirror fallback lives in
// [`crate::game::version_list::VersionListCache`]; the three FFI helpers are
// thin wrappers that exchange JSON so the Compose layer never has to hold
// native state beyond a single request/response.

use crate::game::version_list::{VersionGroups, VersionListCache, VersionListSearch};

/// Process-wide version-list cache. Cheap to clone (Arc-internal); the JNI
/// helpers all share it via this single static.
fn version_list_cache() -> &'static VersionListCache {
    static CACHE: OnceLock<VersionListCache> = OnceLock::new();
    CACHE.get_or_init(VersionListCache::new)
}

/// Build a one-shot network client honouring the caller's `mirror_mode` and
/// `dns_mode` JSON. Falls back to the default (origin-first mirror list, no
/// overrides) when `request` is empty or malformed.

/// Helper: serialise a `VersionManifest` into the JSON envelope the Kotlin UI
/// consumes. Includes the manifest itself, the [VersionListInfo] struct, and
/// the pre-grouped buckets ([VersionGroups]) so the UI does not have to re-run
/// grouping in Kotlin.
fn version_list_envelope(
    manifest: &crate::game::VersionManifest,
    info: crate::game::version_list::VersionListInfo,
    groups: &VersionGroups,
) -> serde_json::Value {
    serde_json::json!({
        "manifest": manifest,
        "info": info,
        "groups": serde_json::json!({
            "release": groups.release,
            "snapshot": groups.snapshot,
            "pre_release": groups.pre_release,
            "old_alpha": groups.old_alpha,
            "old_beta": groups.old_beta,
            "special": groups.special,
        }),
    })
}

/// `RustBridge.gameFetchVersionList(request): String` -- return the cached or
/// freshly-fetched version list.
///
/// `requestJson` = `{ "ttl_secs"?: uint, "force_refresh"?: bool,
///   "query"?: string, "group"?: string, "mirror_mode"?: string, "dns_mode"?: { mode, servers? } }`.
///
/// * `force_refresh` - bypass TTL even when the cache is fresh.
/// * `ttl_secs`     - override the cache TTL for this call only.
/// * `query`        - case-insensitive substring filter applied to the result.
/// * `group`        - one of `release / snapshot / pre_release / old_alpha /
///   old_beta / special`; when present only that bucket is returned (the
///   envelope still carries the others for completeness).
///
/// The reply envelope is
/// `{ manifest, info, groups, filtered }` where `filtered` is the (optionally
/// grouped, optionally searched) subset the UI is meant to render.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_gameFetchVersionList(
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
        let ttl = value
            .get("ttl_secs")
            .and_then(|v| v.as_u64())
            .map(std::time::Duration::from_secs);
        let force_refresh = value
            .get("force_refresh")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let query = value.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let group_filter = value
            .get("group")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        // Build a one-shot network client. DNS / mirror overrides are applied
        // before the very first request, so they affect *both* the initial
        // manifest fetch and the auto-select mirror speed-test (task 3).
        let build_result = block_on_async(async {
            let client = match build_version_list_client(&value).await {
                Ok(c) => c,
                Err(e) => return Err(e),
            };
            let manifest = version_list_cache()
                .fetch_or_load(&client, ttl, force_refresh)
                .await;
            Ok::<_, crate::error::RcError>(manifest)
        });
        let manifest = match build_result {
            Ok(m) => m,
            Err(e) => return auth_err_json(&mut env, &e),
        };
        let cache = version_list_cache();
        let info = cache.info(ttl);
        let groups = VersionGroups::from(&manifest);

        // Apply the optional group + query filter to the *bucket* the caller
        // asked for. When `group` is absent we return the entire manifest
        // (groups still expose all six buckets for UI tabs).
        let filtered: Vec<crate::game::VersionEntry> = if group_filter.is_some() {
            VersionListSearch::new(&manifest)
                .search(query)
                .into_iter()
                .filter(|e| match group_filter.as_deref() {
                    Some("release") => groups.release.iter().any(|g| g.id == e.id),
                    Some("snapshot") => groups.snapshot.iter().any(|g| g.id == e.id),
                    Some("pre_release") => groups.pre_release.iter().any(|g| g.id == e.id),
                    Some("old_alpha") => groups.old_alpha.iter().any(|g| g.id == e.id),
                    Some("old_beta") => groups.old_beta.iter().any(|g| g.id == e.id),
                    Some("special") => groups.special.iter().any(|g| g.id == e.id),
                    _ => true,
                })
                .collect()
        } else if !query.is_empty() {
            VersionListSearch::new(&manifest).search(query)
        } else {
            manifest.versions.clone()
        };
        let _ = &group_filter; // keep the import warm when only `query` is used

        let envelope = version_list_envelope(&manifest, info, &groups);
        let out = serde_json::json!({
            "manifest": envelope["manifest"],
            "info": envelope["info"],
            "groups": envelope["groups"],
            "filtered": filtered,
            "query": query,
            "group": group_filter,
        });
        jstr(&mut env, &out.to_string())
    })
}

/// `RustBridge.gameRefreshVersionList(request): String` -- force a network
/// refresh of the version list. Same reply envelope as
/// [Java_com_rc_launcher_core_RustBridge_gameFetchVersionList] but
/// `force_refresh` is always true and the resulting manifest is freshly
/// written through to the fresh + last-good slots.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_gameRefreshVersionList(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    auth_ffi!({
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => "{}".to_string(),
        };
        let mut value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => serde_json::json!({}),
        };
        // Force a network refresh regardless of what the caller asked.
        if let Some(obj) = value.as_object_mut() {
            obj.insert("force_refresh".to_string(), serde_json::json!(true));
        }
        let build_result = block_on_async(async {
            let client = match build_version_list_client(&value).await {
                Ok(c) => c,
                Err(e) => return Err(e),
            };
            let manifest = version_list_cache().refresh(&client).await;
            Ok::<_, crate::error::RcError>(manifest)
        });
        let manifest = match build_result {
            Ok(m) => m,
            Err(e) => return auth_err_json(&mut env, &e),
        };
        let cache = version_list_cache();
        let info = cache.info(None);
        let groups = VersionGroups::from(&manifest);
        let envelope = version_list_envelope(&manifest, info, &groups);
        jstr(&mut env, &envelope.to_string())
    })
}

/// `RustBridge.gameVersionListCacheInfo(request): String` -- inspect the
/// cache without performing any IO. Returns the [VersionListInfo] JSON; the
/// manifest itself is omitted (call [gameFetchVersionList] to retrieve it).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_gameVersionListCacheInfo(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    auth_ffi!({
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => "{}".to_string(),
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => serde_json::json!({}),
        };
        let ttl = value
            .get("ttl_secs")
            .and_then(|v| v.as_u64())
            .map(std::time::Duration::from_secs);
        let info = version_list_cache().info(ttl);
        jstr(
            &mut env,
            &serde_json::to_string(&info).unwrap_or_else(|_| "{}".to_string()),
        )
    })
}

/// `RustBridge.gameVersionListClearCache(): String` -- drop both the fresh and
/// last-good cache slots. Returns `{ "cleared": true }`. The next
/// `gameFetchVersionList` call will re-fetch from the network (or fall back
/// to the offline built-in DB).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_gameVersionListClearCache(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    auth_ffi!({
        version_list_cache().clear();
        jstr(&mut env, &serde_json::json!({"cleared": true}).to_string())
    })
}

use crate::discord::{self, DiscordConfig, RichPresence};

/// `RustBridge.discordConfigure(requestJson): String` — (re)configure the bridge.
///
/// `requestJson` = `{ "enabled": bool, "application_id"?: string,
/// "library_path"?: string }`. Returns the [`DiscordStateInfo`] JSON snapshot.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_discordConfigure(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    auth_ffi!({
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request"),
        };
        let cfg: DiscordConfig = match DiscordConfig::from_json(&raw) {
            Ok(c) => c,
            Err(e) => return err_json(&mut env, &format!("bad request: {e}")),
        };
        jstr(&mut env, &discord::configure(&cfg).to_json())
    })
}

/// `RustBridge.discordUpdate(presenceJson): String` — update the rich presence.
///
/// `presenceJson` is a [`RichPresence`] JSON object; any omitted field is left
/// unset. Returns the [`DiscordStateInfo`] JSON snapshot.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_discordUpdate(
    mut env: JNIEnv,
    _class: JClass,
    presence: JString,
) -> jstring {
    auth_ffi!({
        let raw = match read_input(&mut env, &presence) {
            Some(s) => s,
            None => return err_json(&mut env, "missing presence"),
        };
        let p: RichPresence = match RichPresence::from_json(&raw) {
            Ok(p) => p,
            Err(e) => return err_json(&mut env, &format!("bad presence: {e}")),
        };
        jstr(&mut env, &discord::update_presence(&p).to_json())
    })
}

/// `RustBridge.discordClear(): String` — clear the "now playing" card without
/// disconnecting. Returns the [`DiscordStateInfo`] JSON snapshot.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_discordClear(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // `discord::clear_presence` is panic-free; still guard the boundary.
        let info = discord::clear_presence();
        info.to_json()
    }));
    match built {
        Ok(s) => match env.new_string(s) {
            Ok(j) => j.into_raw(),
            Err(_) => std::ptr::null_mut(),
        },
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.discordStatus(): String` — return the current
/// [`DiscordStateInfo`] JSON snapshot (no side effects).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_discordStatus(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let built =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| discord::status().to_json()));
    match built {
        Ok(s) => match env.new_string(s) {
            Ok(j) => j.into_raw(),
            Err(_) => std::ptr::null_mut(),
        },
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.discordShutdown(): String` — fully disconnect from Discord and
/// release the native library. Returns the [`DiscordStateInfo`] JSON snapshot.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_discordShutdown(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        discord::shutdown().to_json()
    }));
    match built {
        Ok(s) => match env.new_string(s) {
            Ok(j) => j.into_raw(),
            Err(_) => std::ptr::null_mut(),
        },
        Err(_) => std::ptr::null_mut(),
    }
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

/// `RustBridge.downloadAsync(specJson): String` — fire-and-forget async
/// *download* job (task 2 ⇄ task 10 integration). The same event bus carries
/// progress / lifecycle / error events; the returned JSON is
/// `{ "ok": bool, "scope": string }`.
///
/// `specJson` = `{ "scope"?, "label"?, "concurrency"?, "tasks": [ { "url",
/// "dest", "size"?, "sha1"?, "md5"?, "mirrors"? } ] }`.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_downloadAsync(
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
        match jobs::spawn_download_job(&spec_val, None) {
            Ok(v) => jstr(&mut env, &v.to_string()),
            Err(e) => err_json(&mut env, &e.to_string()),
        }
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

// === Modpack import FFI (task 17) ============================================
//
// The Compose UI drives the modpack import pipeline through these entry
// points. Each takes a JSON spec (so we stay forward-compatible) and returns
// a JSON reply the Kotlin side parses with `org.json.JSONObject`.
//
// `modpackInspect`: parse a manifest (or archive) into the normalised
//                  `ModpackSpec` JSON, without performing any I/O.
// `modpackImport`:  run the full pipeline (parse → download → verify →
//                  extract overrides) and return the per-file report.
// `modpackExtractOverrides`: extract the `overrides/` directory out of an
//                  archive already saved on disk.

/// JSON wrapper around `ModpackImporter::new`. Keeps the JNI layer in charge
/// of building the mirror-aware network client (DoH resolution is async).
#[derive(Debug, Default)]
#[allow(dead_code)]
struct ModpackImporterState {
    importer: std::sync::Mutex<Option<crate::mods::modpack::ModpackImporter>>,
}

static MODPACK_STATE: OnceLock<ModpackImporterState> = OnceLock::new();

#[allow(dead_code)]
fn modpack_state() -> &'static ModpackImporterState {
    MODPACK_STATE.get_or_init(ModpackImporterState::default)
}

/// Internal helper: build a fresh importer from a JSON `config_json`. The
/// caller hands us the `instances_root` (absolute path on the device) plus
/// optional concurrency / chunk size / retry knobs.
fn build_importer_from_config(
    config_json: &str,
) -> RcResult<crate::mods::modpack::ModpackImporter> {
    #[derive(serde::Deserialize)]
    struct Config {
        #[serde(default)]
        instances_root: Option<String>,
        #[serde(default)]
        concurrency: Option<usize>,
        #[serde(default)]
        chunk_size: Option<u64>,
        #[serde(default)]
        max_retries: Option<u32>,
    }
    let cfg: Config = serde_json::from_str(config_json)
        .map_err(|e| RcError::Other(format!("bad modpack config: {e}")))?;
    let instances_root = cfg
        .instances_root
        .ok_or_else(|| RcError::Other("config missing `instances_root`".into()))?;
    let options = crate::mods::modpack::ImportOptions {
        concurrency: cfg.concurrency.unwrap_or(4),
        chunk_size: cfg.chunk_size.unwrap_or(4 * 1024 * 1024),
        max_retries: cfg.max_retries.unwrap_or(3),
        ..Default::default()
    };
    // We don't take a `MirrorProvider` from the Kotlin side today: the importer
    // already wires up its own mirror-aware `NetworkClient` via the built-in
    // mirror list. Callers wanting full China-network tuning can call
    // `modpackSetMirror(json)` first.
    let mirror = match get_active_mirror() {
        Some(m) => Some(std::sync::Arc::new(m)),
        None => None,
    };
    let instances_path = std::path::PathBuf::from(instances_root);
    // Async construction: spin up a one-shot runtime so we can `await` it.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| RcError::Other(format!("tokio runtime: {e}")))?;
    rt.block_on(crate::mods::modpack::ModpackImporter::new(
        instances_path,
        mirror,
        options,
    ))
}

fn get_active_mirror() -> Option<crate::net::MirrorProvider> {
    Some(crate::net::MirrorProvider::new(default_mirrors()))
}

/// `RustBridge.modpackInspect(textJson): String` — parse a modpack manifest
/// (Modrinth / CurseForge / MMC) and return the normalised
/// `ModpackSpec` JSON. Pure parse, no I/O. Input is the manifest *text*
/// (`{"text": "...", "origin": "..."}`); origin is optional metadata the
/// UI uses to display "loaded from ..."
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_modpackInspect(
    mut env: JNIEnv,
    _class: JClass,
    spec: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &spec) {
            Some(s) => s,
            None => return err_json(&mut env, "missing spec"),
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad spec: {e}")),
        };
        let text = match parsed.get("text").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return err_json(&mut env, "spec missing `text`"),
        };
        let origin = parsed.get("origin").and_then(|v| v.as_str());
        match crate::mods::modpack::detect_and_parse(text, origin) {
            Ok(manifest) => match serde_json::to_string(&manifest) {
                Ok(s) => jstr(&mut env, &s),
                Err(e) => err_json(&mut env, &format!("serialise: {e}")),
            },
            Err(e) => err_json(&mut env, &e.to_string()),
        }
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.modpackInspectArchive(bytesJson): String` — same as
/// [`modpackInspect`] but takes a base64-encoded archive body. Useful for
/// Compose picking a `.zip` / `.mrpack` through the system file picker.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_modpackInspectArchive(
    mut env: JNIEnv,
    _class: JClass,
    spec: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &spec) {
            Some(s) => s,
            None => return err_json(&mut env, "missing spec"),
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad spec: {e}")),
        };
        let bytes_b64 = match parsed.get("bytes_b64").and_then(|v| v.as_str()) {
            Some(b) => b,
            None => return err_json(&mut env, "spec missing `bytes_b64`"),
        };
        use base64::Engine;
        let bytes = match base64::engine::general_purpose::STANDARD.decode(bytes_b64) {
            Ok(b) => b,
            Err(e) => return err_json(&mut env, &format!("base64: {e}")),
        };
        let origin = parsed.get("origin").and_then(|v| v.as_str());
        match crate::mods::modpack::parse_archive_bytes(&bytes, origin) {
            Ok(manifest) => match serde_json::to_string(&manifest) {
                Ok(s) => jstr(&mut env, &s),
                Err(e) => err_json(&mut env, &format!("serialise: {e}")),
            },
            Err(e) => err_json(&mut env, &e.to_string()),
        }
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.modpackImport(specJson): String` — fire-and-forget async
/// modpack import. The actual work is run on the same event bus as
/// `downloadAsync`; the reply is `{ "ok": bool, "scope": string, "report"?: ...}`.
///
/// `specJson` = `{"config": { ... }, "manifest": { ... } }`. `config` carries
/// the importer settings (instances_root etc.); `manifest` is the output of
/// `modpackInspect` (re-parsed here so the Kotlin side never holds raw JSON).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_modpackImport(
    mut env: JNIEnv,
    _class: JClass,
    spec: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &spec) {
            Some(s) => s,
            None => return err_json(&mut env, "missing spec"),
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad spec: {e}")),
        };
        // Build the importer once (sync helper), then hand the rest to the
        // async job manager so the UI stays responsive.
        let config_json = match parsed.get("config") {
            Some(v) => v.to_string(),
            None => return err_json(&mut env, "spec missing `config`"),
        };
        let manifest_json = match parsed.get("manifest") {
            Some(v) => v.to_string(),
            None => return err_json(&mut env, "spec missing `manifest`"),
        };
        let instance_id = match parsed.get("instance_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return err_json(&mut env, "spec missing `instance_id`"),
        };
        let allow_overwrite = parsed
            .get("allow_overwrite")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let importer = match build_importer_from_config(&config_json) {
            Ok(i) => i,
            Err(e) => return err_json(&mut env, &e.to_string()),
        };
        let manifest: crate::mods::modpack::Manifest = match serde_json::from_str(&manifest_json) {
            Ok(m) => m,
            Err(e) => return err_json(&mut env, &format!("bad manifest: {e}")),
        };
        let reply =
            jobs::spawn_modpack_import_job(importer, manifest, instance_id, allow_overwrite);
        jstr(&mut env, &reply.to_string())
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.modpackExtractOverrides(reqJson): String` — synchronously
/// extract the `overrides/` directory out of an already-imported archive.
/// `reqJson` = `{"bytes_b64": "...", "instance_root": "/data/..."}`.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_modpackExtractOverrides(
    mut env: JNIEnv,
    _class: JClass,
    spec: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &spec) {
            Some(s) => s,
            None => return err_json(&mut env, "missing spec"),
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad spec: {e}")),
        };
        let bytes_b64 = match parsed.get("bytes_b64").and_then(|v| v.as_str()) {
            Some(b) => b,
            None => return err_json(&mut env, "spec missing `bytes_b64`"),
        };
        let root = match parsed.get("instance_root").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return err_json(&mut env, "spec missing `instance_root`"),
        };
        use base64::Engine;
        let bytes = match base64::engine::general_purpose::STANDARD.decode(bytes_b64) {
            Ok(b) => b,
            Err(e) => return err_json(&mut env, &format!("base64: {e}")),
        };
        match crate::mods::modpack::extract_overrides(&bytes, std::path::Path::new(root)) {
            Ok(written) => {
                let paths: Vec<String> = written
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                let reply = serde_json::json!({ "ok": true, "written": paths });
                jstr(&mut env, &reply.to_string())
            }
            Err(e) => err_json(&mut env, &e.to_string()),
        }
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

// === AWT / Swing bridge FFI (task 18) ========================================

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
use crate::launch::input::PointerSource;
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
    let mut input_notes: Vec<String> = Vec::new();
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
        // Task 12: sensitivity / pointer mode / hybrid touch / key remapping.
        // A stale binding is reported, never fatal: the rest of the update lands.
        if let Some(value) = req.get("input") {
            let (_, notes) = session.apply_input_json(value);
            input_notes = notes;
        }
        if let Some(argb) = req.get("fill").and_then(|v| v.as_u64()) {
            session.fill(argb as u32);
        }
        if req.get("clear").and_then(|v| v.as_bool()) == Some(true) {
            session.clear();
        }
    }
    let mut out = host.to_json();
    if !input_notes.is_empty() {
        if let Some(map) = out.as_object_mut() {
            map.insert("input_notes".to_string(), json!(input_notes));
        }
    }
    Ok(out)
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
            let source = event
                .get("source")
                .and_then(|v| v.as_str())
                .map(PointerSource::from_id)
                .unwrap_or(if kind == "mouse" {
                    PointerSource::Mouse
                } else {
                    PointerSource::Touch
                });
            Ok(session.pointer_from(phase, f32_at("x"), f32_at("y"), button, source))
        }
        // Task 12: relative motion from a captured physical mouse. Android
        // reports how far the mouse moved, not where it is, so this is a
        // *different* event and not a pointer with a computed position.
        "pointer_relative" | "mouse_relative" | "relative" => {
            let source = event
                .get("source")
                .and_then(|v| v.as_str())
                .map(PointerSource::from_id)
                .unwrap_or(PointerSource::Mouse);
            Ok(session.pointer_relative(f32_at("dx"), f32_at("dy"), source))
        }
        // Task 12: the pointer was captured / released by the UI.
        "capture" | "pointer_capture" | "grab" => {
            let captured = event
                .get("captured")
                .or_else(|| event.get("grabbed"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            Ok(session.set_capture(captured))
        }
        // Task 12: a captured mouse has no surface position, so its clicks
        // happen wherever the virtual pointer currently is.
        "button" => {
            let down = event
                .get("down")
                .or_else(|| event.get("pressed"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let button = match event
                .get("button")
                .and_then(|v| v.as_str())
                .unwrap_or("left")
            {
                "left" | "primary" => MouseButton::Left,
                "middle" => MouseButton::Middle,
                "right" | "secondary" => MouseButton::Right,
                other => return Err(bad(format!("unknown mouse button {other:?}"))),
            };
            Ok(session.button(button, down))
        }
        "scroll" | "wheel" => {
            let ticks = event
                .get("ticks")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .clamp(i32::MIN as i64, i32::MAX as i64) as i32;
            // No position at all means "at the pointer" (a captured mouse).
            if event.get("x").is_none() && event.get("y").is_none() {
                return Ok(session.scroll_at_pointer(ticks));
            }
            Ok(session.scroll(f32_at("x"), f32_at("y"), ticks))
        }
        "key_down" | "key_up" => {
            let down = kind == "key_down";
            // Task 12: `KeyEvent.getScanCode()` is the Linux evdev code, which is
            // exactly what `GLFWKeyCallback` wants — forward it verbatim when the
            // UI has one, and let the core fill it in when it does not.
            let scancode = event
                .get("scancode")
                .and_then(|v| v.as_i64())
                .map(|v| v.clamp(0, i32::MAX as i64) as i32)
                .filter(|v| *v > 0);
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
            Ok(session.key_named(name, scancode, down))
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
    let (gx, gy) = session.game_cursor();
    Ok(json!({
        "queued": queued,
        "pending": session.pending_events(),
        "modifiers": session.modifiers(),
        "focused": session.is_focused(),
        "pointer": { "x": px, "y": py },
        "rejected": rejected,
        // Task 12: the UI mirrors the capture state (it has to hide its own
        // pointer overlay and keep the Android pointer captured) and shows the
        // game cursor in the diagnostics panel.
        "captured": session.is_captured(),
        "pointer_mode": session.input_settings().pointer_mode.id(),
        "game_cursor": { "x": gx, "y": gy },
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

/// Take the control messages the JVM sent (cursor / title / clipboard / IME).
///
/// Returns both the *messages* (the UI must act on each exactly once: push this
/// text to the Android clipboard, buzz, pop the keyboard) and the *projection*
/// (what the UI renders every frame), so one call per frame is enough.
fn awt_drain_control_json(slot: &AwtSlot) -> RcResult<serde_json::Value> {
    let host = awt_host(slot)?;
    let (messages, control, pending) = {
        let mut session = host.session();
        let messages: Vec<serde_json::Value> = session
            .drain_control()
            .iter()
            .map(|c| c.to_json())
            .collect();
        let control = session.control().to_json();
        let pending = session.pending_clipboard_requests();
        (messages, control, pending)
    };
    Ok(json!({
        "control": messages,
        "count": messages.len(),
        "state": control,
        "clipboard_requests": pending,
    }))
}

/// Answers / commands for the control plane, coming from the Compose layer.
///
/// * `{"clipboard": "text"}` - answer every pending `Clipboard.getContents()`.
/// * `{"clipboard": null}` / `{"clipboard_empty": true}` - answer "no text"
///   (still an *answer*: a Swing thread may be blocked on it).
/// * `{"clipboard_seq": n, "clipboard": ...}` - answer one specific request.
/// * `{"pong": n}` - liveness answer.
/// * `{"reset": true}` - forget the projection (arrow cursor, no keyboard).
fn awt_control_json(slot: &AwtSlot, req: &serde_json::Value) -> RcResult<serde_json::Value> {
    let host = awt_host(slot)?;
    let mut queued = 0usize;
    {
        let mut session = host.session();
        let has_clipboard_key = req.get("clipboard").is_some();
        let empty = req.get("clipboard_empty").and_then(|v| v.as_bool()) == Some(true);
        if has_clipboard_key || empty {
            let text = req.get("clipboard").and_then(|v| v.as_str());
            let text = if empty { None } else { text };
            queued += match req.get("clipboard_seq").and_then(|v| v.as_u64()) {
                Some(seq) => session.answer_clipboard_seq(seq as u32, text),
                None => session.answer_clipboard(text),
            };
        }
        if let Some(seq) = req.get("pong").and_then(|v| v.as_u64()) {
            queued += session.answer_pong(seq as u32);
        }
        if req.get("reset").and_then(|v| v.as_bool()) == Some(true) {
            session.reset_control();
        }
    }
    Ok(json!({
        "queued": queued,
        "clipboard_requests": host.session().pending_clipboard_requests(),
        "state": host.session().control().to_json(),
    }))
}

/// Feed one encoded (`RCAC`) control message in - the mirror of
/// `awtSubmitFrame`, for a Kotlin-owned transport or a self-test.
fn awt_submit_control_json(slot: &AwtSlot, bytes: &[u8]) -> RcResult<serde_json::Value> {
    let host = awt_host(slot)?;
    host.submit_control_bytes(bytes)?;
    Ok(json!({
        "accepted": true,
        "state": host.session().control().to_json(),
    }))
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

/// `RustBridge.awtDrainControl(): String` - the control messages the JVM sent
/// (cursor shape, window title, clipboard hand-off / request, IME, beep) plus the
/// current projection. One call per UI frame; `{"count":0}` when nothing changed.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_awtDrainControl(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    auth_ffi!({
        let slot = lock_awt();
        let out = awt_drain_control_json(&slot);
        drop(slot);
        rc_to_json(&mut env, out)
    })
}

/// `RustBridge.awtControl(json): String` - the launcher's answers to the control
/// plane (clipboard contents, liveness, reset). See `awt_control_json`.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_awtControl(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    auth_ffi!({
        let raw = match read_input(&mut env, &request_json) {
            Some(s) => s,
            None => return err_json(&mut env, "awtControl needs a JSON request"),
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad AWT control request: {e}")),
        };
        let slot = lock_awt();
        let out = awt_control_json(&slot, &value);
        drop(slot);
        rc_to_json(&mut env, out)
    })
}

/// `RustBridge.awtSubmitControl(bytes): String` - hand one encoded `RCAC` control
/// message to the session (Kotlin-owned transport / self-test).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_awtSubmitControl(
    mut env: JNIEnv,
    _class: JClass,
    message: JByteArray,
) -> jstring {
    auth_ffi!({
        let bytes = match env.convert_byte_array(&message) {
            Ok(b) => b,
            Err(e) => return err_json(&mut env, &format!("cannot read the control message: {e}")),
        };
        let slot = lock_awt();
        let out = awt_submit_control_json(&slot, &bytes);
        drop(slot);
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

// === Gamepad mapping database + input calibration FFI (task 4) ============
//
// JSON-in / JSON-out bridge for the `gamepad` subsystem. Profile metadata and
// plug-and-play identification are returned as JSON (consumed by the UI picker
// and the Kotlin `GamepadDatabase` mirror); axis/stick calibration is plain
// float math so it can run per-frame on the input thread. Every entry point is
// wrapped in `catch_unwind` so a panic never aborts the VM.

/// `RustBridge.getControllerProfiles(): String` — built-in controller profile
/// metadata as a JSON array.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_getControllerProfiles(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let metas: Vec<crate::gamepad::GamepadProfileMeta> = crate::gamepad::list_profiles();
        let json = serde_json::to_string(&metas).unwrap_or_else(|_| "[]".to_string());
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

/// `RustBridge.identifyController(vendorId: Int, productId: Int): String` —
/// returns the matched profile metadata as JSON (or the generic fallback).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_identifyController(
    env: JNIEnv,
    _class: JClass,
    vendor_id: jint,
    product_id: jint,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let vid = (vendor_id as u32 & 0xFFFF) as u16;
        let pid = (product_id as u32 & 0xFFFF) as u16;
        let meta = crate::gamepad::identify(vid, pid).meta();
        let json = serde_json::to_string(&meta).unwrap_or_else(|_| "{}".to_string());
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

/// `RustBridge.calibrateAxis(value, deadzone, sensitivity, invert): Float` —
/// applies a single-axis dead-zone + sensitivity + invert (task 4 input layer).
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_calibrateAxis(
    _env: JNIEnv,
    _class: JClass,
    value: jfloat,
    deadzone: jfloat,
    sensitivity: jfloat,
    invert: jboolean,
) -> jfloat {
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let cal = crate::gamepad::AxisCalibration {
            deadzone,
            sensitivity,
            invert: invert == JNI_TRUE,
        };
        cal.calibrate(value)
    }));
    match out {
        Ok(v) => v,
        Err(_) => 0.0,
    }
}

/// `RustBridge.calibrateStick(x, y, deadzone, sensitivity, invertX, invertY): String` —
/// applies a radial stick dead-zone + sensitivity + per-axis invert, returning
/// the calibrated `(x, y)` as a JSON pair.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_calibrateStick(
    env: JNIEnv,
    _class: JClass,
    x: jfloat,
    y: jfloat,
    deadzone: jfloat,
    sensitivity: jfloat,
    invert_x: jboolean,
    invert_y: jboolean,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let cal = crate::gamepad::StickCalibration {
            deadzone,
            sensitivity,
            invert_x: invert_x == JNI_TRUE,
            invert_y: invert_y == JNI_TRUE,
        };
        let (nx, ny) = cal.calibrate(x, y);
        let json = format!("[{},{}]", nx, ny);
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

// === Translation FFI (task 13) ===========================================
//
// Inline auto-translation for the mod browser. The Kotlin UI hands us
// JSON requests through `RustBridge.translate` / `translateBatch`, and
// gets JSON results back. The actual translation work happens in
// `crate::translate::TranslationService`; this module is the thin
// glue layer that:
//
//  * owns the **process-wide** `TranslationService` (lazy-init, so the
//    JNI `RustBridge` class can be loaded even before the network
//    subsystem has been built);
//  * decodes the JSON request, calls the service, and re-encodes the
//    JSON response;
//  * never panics — every entry point is wrapped in `auth_ffi!`, and
//    every failure becomes `{"error": ...}` JSON so the Compose layer
//    can keep rendering the browser.
//
// The translation cache lives on disk under the launcher's cache
// directory; the path is configurable through `translateInit` so the
// UI can pass the canonical Android cache dir at startup.

use crate::translate::{
    TranslationGateway, TranslationMode, TranslationRequest, TranslationResult, TranslationService,
    TranslationServiceBuilder,
};

/// The process-wide translation service. `OnceLock` because the network
/// subsystem is built lazily (the same pattern as
/// `crate::event::EventBus`).
static TRANSLATION: OnceLock<std::sync::Mutex<Option<TranslationService>>> = OnceLock::new();

fn translation_slot() -> &'static std::sync::Mutex<Option<TranslationService>> {
    TRANSLATION.get_or_init(|| std::sync::Mutex::new(None))
}

fn ensure_translation_service(network: crate::net::NetworkClient) -> TranslationService {
    let mut guard = translation_slot()
        .lock()
        .expect("translation slot poisoned");
    if let Some(svc) = guard.as_ref() {
        return svc.clone();
    }
    let svc = TranslationServiceBuilder::new()
        .network(network)
        .build()
        .expect("TranslationServiceBuilder::build only fails on missing network");
    *guard = Some(svc.clone());
    svc
}

fn shared_network() -> crate::net::NetworkClient {
    static NET: OnceLock<crate::net::NetworkClient> = OnceLock::new();
    if let Some(c) = NET.get() {
        return c.clone();
    }
    // Best-effort: if the network client cannot build (no mirrors, no
    // DNS, etc.), fall back to a plain offline-friendly client.
    let client =
        block_on_async(crate::net::NetworkClient::builder().build()).unwrap_or_else(|_| {
            block_on_async(
                crate::net::NetworkClient::builder()
                    .config(crate::net::NetworkConfig::default())
                    .build(),
            )
            .expect("default NetworkClient::build() must succeed")
        });
    NET.get_or_init(|| client.clone()).clone()
}

/// Pure core of `RustBridge.translateInit` — (re)configure the process-wide
/// translation service. `requestJson` =
/// `{ "cache_root"?: string, "gateway"?: TranslationGateway,
///   "force_offline"?: bool, "default_mode"?: "online"|"offline"|"hybrid" }`.
///
/// Returns `{"ok": true}` on success.
pub fn translate_init_json(request: &serde_json::Value) -> serde_json::Value {
    let cache_root = request
        .get("cache_root")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(crate::translate::service::default_cache_root);
    let force_offline = request
        .get("force_offline")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let default_mode = match request.get("default_mode").and_then(|v| v.as_str()) {
        Some("online") => TranslationMode::Online,
        Some("offline") => TranslationMode::Offline,
        _ => TranslationMode::Hybrid,
    };
    let gateway: TranslationGateway = request
        .get("gateway")
        .cloned()
        .and_then(|v| serde_json::from_value::<TranslationGateway>(v).ok())
        .unwrap_or_default();

    let network = shared_network();
    let cache = crate::translate::TranslationCache::open(cache_root)
        .unwrap_or_else(|_| crate::translate::TranslationCache::open(crate::translate::service::default_cache_root()).unwrap());
    let dictionary = crate::translate::BuiltInDictionary::builtin();
    let svc = TranslationService::with_parts(network, cache, dictionary, gateway)
        .with_default_mode(default_mode)
        .with_force_offline(force_offline);
    let mut guard = translation_slot()
        .lock()
        .expect("translation slot poisoned");
    *guard = Some(svc);
    serde_json::json!({ "ok": true })
}

/// Pure core of `RustBridge.translate` — translate one request.
pub fn translate_json(request: &serde_json::Value) -> RcResult<serde_json::Value> {
    let req: TranslationRequest = serde_json::from_value(request.clone())
        .map_err(|e| RcError::Other(format!("bad translate request: {e}")))?;
    let svc = ensure_translation_service(shared_network());
    // Synchronous wait — the service is async because of the gateway, but
    // the JNI contract wants a synchronous return. `futures_util::executor::block_on`
    // is fine here: the calling thread is already an off-main background
    // thread (Kotlin dispatchers.IO), and a single translation should
    // take <100 ms (dictionary) or a few seconds (gateway, but cached).
    let result: TranslationResult = block_on_async(svc.translate(&req))?;
    Ok(serde_json::to_value(&result).map_err(RcError::Json)?)
}

/// Pure core of `RustBridge.translateBatch` — translate N requests in
/// order; the output array preserves input order.
pub fn translate_batch_json(request: &serde_json::Value) -> RcResult<serde_json::Value> {
    let reqs: Vec<TranslationRequest> = serde_json::from_value(request.clone())
        .map_err(|e| RcError::Other(format!("bad translateBatch request: {e}")))?;
    let svc = ensure_translation_service(shared_network());
    let results: Vec<TranslationResult> = block_on_async(svc.translate_batch(reqs))?;
    Ok(serde_json::to_value(&results).map_err(RcError::Json)?)
}

/// Pure core of `RustBridge.translateLanguages` — JSON array of the
/// translation languages the UI exposes in its picker.
pub fn translate_languages_json() -> serde_json::Value {
    use crate::translate::model::TranslationLanguage;
    let langs: Vec<serde_json::Value> = TranslationLanguage::ALL
        .iter()
        .map(|l| {
            serde_json::json!({
                "tag": l.tag(),
                "label": l.label(),
                "english_label": l.english_label(),
            })
        })
        .collect();
    serde_json::json!({
        "languages": langs,
        "modes": ["online", "offline", "hybrid"],
        "sources": ["passthrough", "dictionary", "cache", "gateway", "unavailable"],
    })
}

/// Pure core of `RustBridge.translateCacheStats` — report cache size and
/// entry count, so the settings UI can show "已缓存 N 条翻译 (X MB)".
pub fn translate_cache_stats_json() -> serde_json::Value {
    let svc = ensure_translation_service(shared_network());
    let cache = svc.cache();
    serde_json::json!({
        "root": cache.root().to_string_lossy(),
        "entry_count": cache.entry_count(),
        "total_bytes": cache.total_bytes(),
        "max_entries": cache.config().max_entries,
        "max_bytes": cache.config().max_bytes,
    })
}

/// Pure core of `RustBridge.translateClearCache` — drop everything from
/// the cache (the next translation will re-hit the gateway or
/// dictionary). Returns the number of removed files.
pub fn translate_clear_cache_json() -> serde_json::Value {
    let svc = ensure_translation_service(shared_network());
    let removed = svc.cache().clear().unwrap_or(0);
    serde_json::json!({ "removed": removed })
}

/// Pure core of `RustBridge.translateGatewayJson` — return the
/// currently-configured gateway (the UI uses it to populate the
/// "Settings -> Translation -> Gateway" form).
pub fn translate_gateway_json() -> serde_json::Value {
    let svc = ensure_translation_service(shared_network());
    serde_json::to_value(svc.gateway()).unwrap_or_else(|_| serde_json::json!({}))
}

/// `RustBridge.translateInit(requestJson): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_translateInit(
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
        jstr(&mut env, &translate_init_json(&value).to_string())
    })
}

/// `RustBridge.translate(requestJson): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_translate(
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
        rc_to_json(&mut env, translate_json(&value))
    })
}

/// `RustBridge.translateBatch(requestJson): String` — accepts `{ "requests": [...] }`.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_translateBatch(
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
        // Accept either a bare array or `{ "requests": [...] }` for
        // convenience.
        let normalised = if value.is_array() {
            serde_json::json!({ "requests": value })
        } else {
            value
        };
        let reqs = match normalised.get("requests").cloned() {
            Some(v) => v,
            None => return err_json(&mut env, "missing 'requests' field"),
        };
        rc_to_json(&mut env, translate_batch_json(&reqs))
    })
}

/// `RustBridge.translateLanguages(): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_translateLanguages(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    auth_ffi!({ jstr(&mut env, &translate_languages_json().to_string()) })
}

/// `RustBridge.translateCacheStats(): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_translateCacheStats(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    auth_ffi!({ jstr(&mut env, &translate_cache_stats_json().to_string()) })
}

/// `RustBridge.translateClearCache(): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_translateClearCache(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    auth_ffi!({ jstr(&mut env, &translate_clear_cache_json().to_string()) })
}

/// `RustBridge.translateGateway(): String` — current gateway config.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_translateGateway(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    auth_ffi!({ jstr(&mut env, &translate_gateway_json().to_string()) })
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
    fn orientation_catalogue_is_ui_ready() {
        let arr = crate::display::orientation_catalog_json();
        let items = arr.as_array().unwrap();
        assert_eq!(items.len(), 3);
        let ids: Vec<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
        // Exactly the three ids the Kotlin `OrientationMode` persists.
        assert_eq!(ids, vec!["system", "landscape", "portrait"]);
    }

    #[test]
    fn display_layout_resolves_a_window() {
        // Phone portrait: bottom bar, one column.
        let out = display_layout_json(&json!({ "width_dp": 392, "height_dp": 872 }));
        assert_eq!(out["orientation"], "portrait");
        assert_eq!(out["width_class"], "compact");
        assert_eq!(out["navigation_rail"], false);
        assert_eq!(out["instance_columns"], 1);

        // The same phone rotated: rail, three columns, no bottom bar.
        let out = display_layout_json(&json!({ "width_dp": 872, "height_dp": 392 }));
        assert_eq!(out["orientation"], "landscape");
        assert_eq!(out["navigation_rail"], true);
        assert_eq!(out["instance_columns"], 3);
        assert_eq!(out["short"], true);
    }

    #[test]
    fn display_layout_is_fail_soft_and_can_orient_a_window() {
        // A missing / bogus request degrades to the reference phone instead of
        // erroring out (the UI must always get a usable layout).
        let out = display_layout_json(&json!({}));
        assert_eq!(out["width_dp"], 392);
        assert_eq!(out["height_dp"], 872);
        let out = display_layout_json(&json!({ "width_dp": 0, "height_dp": 0 }));
        // Degenerate geometry is clamped, never rejected, and still yields a
        // renderable layout (a 1x1 window counts as square ⇒ rail side).
        assert_eq!(out["width_dp"], 1);
        assert_eq!(out["height_dp"], 1);
        assert_eq!(out["orientation"], "square");
        assert!(out["instance_columns"].as_u64().unwrap() >= 1);
        // A huge width_dp cannot overflow the u32 cast either.
        let out = display_layout_json(&json!({ "width_dp": 9_999_999_999u64, "height_dp": 10 }));
        assert_eq!(out["width_class"], "expanded");

        // With a policy the reply also carries the forced window geometry.
        let out = display_layout_json(
            &json!({ "width_dp": 1280, "height_dp": 720, "orientation": "portrait" }),
        );
        assert_eq!(out["policy"]["id"], "portrait");
        assert_eq!(
            out["policy"]["android_screen_orientation"],
            "sensorPortrait"
        );
        assert_eq!(out["window"]["width"], 720);
        assert_eq!(out["window"]["height"], 1280);
        // An unknown policy id degrades to "follow system" (no swap).
        let out = display_layout_json(
            &json!({ "width_dp": 1280, "height_dp": 720, "orientation": "sideways" }),
        );
        assert_eq!(out["policy"]["id"], "system");
        assert_eq!(out["window"]["width"], 1280);
    }

    #[test]
    fn renderer_catalogue_is_complete() {
        let out = launch_renderers_json();
        let arr = out.as_array().unwrap();
        // 5 FCL stacks + the LWJGL SDL backend (task 9) + Mobile Glues (task 8).
        assert_eq!(arr.len(), 7);
        assert_eq!(arr[0]["id"], "opengles2");
        assert_eq!(arr[0]["gl_libname"], "libgl4es_114.so");
        assert_eq!(arr[0]["env"]["LIBGL_ES"], "2");
        // The catalogue must surface the SDL renderer added for task 9.
        let sdl = arr
            .iter()
            .find(|r| r["id"] == "sdl2")
            .expect("sdl2 renderer must be in the FFI catalogue");
        assert_eq!(sdl["gl_libname"], "liblwjgl_sdl.so");
        assert_eq!(sdl["backend"], "Sdl");
        // The catalogue must surface the Mobile Glues renderer added for task 8.
        let mg = arr
            .iter()
            .find(|r| r["id"] == "mobile_glues")
            .expect("mobile_glues renderer must be in the FFI catalogue");
        assert_eq!(mg["gl_libname"], "libmobileglues.so");
        assert_eq!(mg["backend"], "GlSurface");
        assert!(arr.iter().all(|r| {
            r["id"].is_string() && r["gl_libname"].is_string() && r["backend"].is_string()
        }));
    }
}

#[cfg(test)]
mod awt_tests {
    use super::*;
    use crate::launch::awt::{AwtControl, AwtEventRecord, AwtFrame, CursorKind};
    use crate::launch::input::{game_event, GameInputEvent};

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
        // AWT (8): press + release + synthetic click + 2 keys + 2 typed chars +
        // wheel. Native (7, task 12): button down, button up, 2 keys, 2 chars,
        // wheel — the game gets its own copy of everything it can use. There is
        // no cursor event because (4,2) on the surface *is* the centre of this
        // 4×2 desktop, which is where the game cursor already sits: a move that
        // does not change a pixel is not an event.
        assert_eq!(out["queued"], 15);
        assert_eq!(out["pending"], 15);
        assert_eq!(out["pointer"], json!({ "x": 2, "y": 1 }));
        assert_eq!(out["focused"], true);
        assert_eq!(out["rejected"], json!([]));
        assert_eq!(out["captured"], false);
        assert_eq!(out["pointer_mode"], "absolute");
        // SHIFT is held, so the modifier mask is non-zero.
        assert_ne!(out["modifiers"], 0);

        // Draining hands them to the JVM as 32-byte records.
        let bytes = awt_drain_events_bytes(&slot);
        assert_eq!(bytes.len(), 15 * 32);
        let records = AwtEventRecord::decode_batch(&bytes).unwrap();
        let (native, awt): (Vec<_>, Vec<_>) = records
            .iter()
            .partition(|r| GameInputEvent::from_record(r).is_some());
        assert_eq!(awt.len(), 8, "the AWT stream is unchanged");
        assert!(
            native
                .iter()
                .filter_map(GameInputEvent::from_record)
                .any(|e| e.kind == game_event::KEY),
            "a physical key must reach the game, not only Swing"
        );
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
        assert_eq!(
            out["queued"], 2,
            "the good event still reached the JVM (AWT + native)"
        );
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
            // Task 12: the Compose layer renders the pointer mode / sensitivity
            // and needs the game cursor for the diagnostics panel.
            "input",
            "game_input",
        ] {
            assert!(out.get(key).is_some(), "snapshot is missing {key}");
        }
        for key in [
            "pointer_mode",
            "captured",
            "hybrid_touch",
            "native_input",
            "sensitivity",
            "scroll_permille",
            "bindings",
        ] {
            assert!(out["input"].get(key).is_some(), "input.{key}");
        }
        for key in ["x", "y", "invert_y"] {
            assert!(
                out["input"]["sensitivity"].get(key).is_some(),
                "input.sensitivity.{key}"
            );
        }
        for key in ["cursor", "grabbed", "framebuffer", "stats"] {
            assert!(out["game_input"].get(key).is_some(), "game_input.{key}");
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

    // ---- Control plane ----------------------------------------------------

    #[test]
    fn control_messages_reach_the_ui_through_the_json_plane() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        // Nothing yet.
        let out = awt_drain_control_json(&slot).unwrap();
        assert_eq!(out["count"], 0);
        assert_eq!(out["state"]["cursor"], "default");
        assert_eq!(out["state"]["wants_keyboard"], false);

        for message in [
            AwtControl::cursor(CursorKind::Text),
            AwtControl::title("Forge 安装程序"),
            AwtControl::clipboard_set("copied"),
            AwtControl::ime_show(2, 1, 8),
            AwtControl::beep(),
        ] {
            assert_eq!(
                awt_submit_control_json(&slot, &message.encode()).unwrap()["accepted"],
                true
            );
        }

        let out = awt_drain_control_json(&slot).unwrap();
        assert_eq!(out["count"], 5);
        let kinds: Vec<&str> = out["control"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["kind"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            vec!["cursor", "title", "clipboard_set", "ime_show", "beep"]
        );
        assert_eq!(out["control"][0]["cursor"], "text");
        assert_eq!(out["control"][2]["text"], "copied");
        assert_eq!(out["control"][3]["x"], 2);
        assert_eq!(out["state"]["cursor"], "text");
        assert_eq!(out["state"]["title"], "Forge 安装程序");
        assert_eq!(out["state"]["wants_keyboard"], true);

        // Draining is destructive: the side effects must not fire twice.
        assert_eq!(awt_drain_control_json(&slot).unwrap()["count"], 0);
        assert_eq!(awt_close_json(&mut slot)["closed"], true);
    }

    #[test]
    fn a_clipboard_request_round_trips_through_the_ffi() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        awt_submit_control_json(&slot, &AwtControl::clipboard_request(5).encode()).unwrap();

        let out = awt_drain_control_json(&slot).unwrap();
        assert_eq!(out["control"][0]["kind"], "clipboard_request");
        assert_eq!(out["control"][0]["seq"], 5);
        assert_eq!(out["clipboard_requests"], 1);

        // The UI read the Android clipboard and answers.
        let out = awt_control_json(&slot, &json!({ "clipboard": "seed 42" })).unwrap();
        assert!(out["queued"].as_u64().unwrap() >= 1);
        assert_eq!(out["clipboard_requests"], 0);

        // The answer is on the outbound record stream, ready for the JVM.
        let bytes = awt_drain_events_bytes(&slot);
        let records = crate::launch::awt::AwtEventRecord::decode_batch(&bytes).unwrap();
        let (kind, seq, text) = crate::launch::awt::decode_control_reply(&records).unwrap();
        assert_eq!(kind, crate::launch::awt::AwtReplyKind::Clipboard);
        assert_eq!(seq, 5);
        assert_eq!(text, "seed 42");
        assert_eq!(awt_close_json(&mut slot)["closed"], true);
    }

    #[test]
    fn an_empty_clipboard_is_still_an_answer_and_seq_targeting_works() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        awt_submit_control_json(&slot, &AwtControl::clipboard_request(1).encode()).unwrap();
        awt_submit_control_json(&slot, &AwtControl::clipboard_request(2).encode()).unwrap();

        // Answer only #2, explicitly empty.
        let out = awt_control_json(
            &slot,
            &json!({ "clipboard_seq": 2, "clipboard_empty": true }),
        )
        .unwrap();
        assert_eq!(out["clipboard_requests"], 1, "#1 is still waiting");
        let records =
            crate::launch::awt::AwtEventRecord::decode_batch(&awt_drain_events_bytes(&slot))
                .unwrap();
        let (kind, seq, _) = crate::launch::awt::decode_control_reply(&records).unwrap();
        assert_eq!(kind, crate::launch::awt::AwtReplyKind::ClipboardEmpty);
        assert_eq!(seq, 2);

        // `{"clipboard": null}` answers the rest the same way.
        let out = awt_control_json(&slot, &json!({ "clipboard": null })).unwrap();
        assert!(out["queued"].as_u64().unwrap() >= 1);
        assert_eq!(out["clipboard_requests"], 0);
        assert_eq!(awt_close_json(&mut slot)["closed"], true);
    }

    #[test]
    fn control_reset_and_pong_are_accepted() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        awt_submit_control_json(&slot, &AwtControl::cursor(CursorKind::Hand).encode()).unwrap();
        let out = awt_control_json(&slot, &json!({ "pong": 9 })).unwrap();
        assert!(out["queued"].as_u64().unwrap() >= 1);
        let out = awt_control_json(&slot, &json!({ "reset": true })).unwrap();
        assert_eq!(out["state"]["cursor"], "default");
        assert_eq!(awt_close_json(&mut slot)["closed"], true);
    }

    #[test]
    fn a_garbage_control_message_is_an_error_not_a_crash() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        assert!(awt_submit_control_json(&slot, &[0xAB; 40]).is_err());
        assert!(awt_submit_control_json(&slot, &[]).is_err());
        // The session is still perfectly usable.
        assert_eq!(awt_info_json(&slot)["open"], true);
        assert_eq!(
            awt_info_json(&slot)["session"]["controls_rejected"],
            2,
            "both were counted"
        );
        assert_eq!(awt_close_json(&mut slot)["closed"], true);
    }

    #[test]
    fn the_control_plane_needs_an_open_session() {
        let slot: AwtSlot = None;
        for err in [
            awt_drain_control_json(&slot).err(),
            awt_control_json(&slot, &json!({ "clipboard": "x" })).err(),
            awt_submit_control_json(&slot, &AwtControl::beep().encode()).err(),
        ] {
            assert!(err.unwrap().to_string().contains("no AWT session is open"));
        }
    }

    #[test]
    fn the_control_snapshot_carries_every_field_the_compose_layer_parses() {
        // Guard rail for the Kotlin `AwtControlBatch` / `AwtControlState` parsers:
        // CI cannot run the Kotlin unit tests on every platform, so the contract
        // is pinned here.
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        awt_submit_control_json(&slot, &AwtControl::window_opened(1, "标题").encode()).unwrap();
        awt_submit_control_json(&slot, &AwtControl::ime_show(1, 1, 9).encode()).unwrap();
        awt_submit_control_json(&slot, &AwtControl::clipboard_set("x").encode()).unwrap();
        awt_submit_control_json(&slot, &AwtControl::clipboard_request(4).encode()).unwrap();

        let out = awt_drain_control_json(&slot).unwrap();
        for key in ["control", "count", "state", "clipboard_requests"] {
            assert!(out.get(key).is_some(), "batch is missing {key}");
        }
        for key in [
            "cursor",
            "cursor_awt_type",
            "title",
            "ime",
            "wants_keyboard",
            "clipboard_out",
            "clipboard_requests",
            "windows",
            "window_count",
            "beeps",
            "bye",
        ] {
            assert!(out["state"].get(key).is_some(), "state.{key}");
        }
        for key in ["x", "y", "line_height"] {
            assert!(out["state"]["ime"].get(key).is_some(), "state.ime.{key}");
        }
        for key in ["id", "title"] {
            assert!(
                out["state"]["windows"][0].get(key).is_some(),
                "state.windows[0].{key}"
            );
        }
        for message in out["control"].as_array().unwrap() {
            for key in ["kind", "seq"] {
                assert!(message.get(key).is_some(), "control[].{key}");
            }
        }
        // …and the session snapshot's control section + counters.
        let info = awt_info_json(&slot);
        assert!(info.get("control").is_some());
        assert!(info.get("pending_controls").is_some());
        for key in [
            "controls_accepted",
            "controls_rejected",
            "controls_dropped",
            "screens_adopted",
            "clipboard_answers",
        ] {
            assert!(info["session"].get(key).is_some(), "session.{key}");
        }
        for key in ["controls_accepted", "controls_rejected"] {
            assert!(info["link"].get(key).is_some(), "link.{key}");
        }
        assert_eq!(awt_close_json(&mut slot)["closed"], true);
    }

    #[test]
    fn a_managed_screen_announcement_resizes_the_canvas_through_the_ffi() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        assert_eq!(awt_info_json(&slot)["screen"]["width"], 4);
        awt_submit_control_json(&slot, &AwtControl::screen_size(16, 8).encode()).unwrap();
        let info = awt_info_json(&slot);
        assert_eq!(info["screen"]["width"], 16);
        assert_eq!(info["screen"]["height"], 8);
        assert_eq!(info["rgba_len"], 16 * 8 * 4);
        assert_eq!(info["session"]["screens_adopted"], 1);
        // A frame at the new size is accepted straight away.
        let frame = AwtFrame::full(1, 16, 8, vec![0xFF12_3456; 16 * 8])
            .unwrap()
            .encode();
        assert_eq!(
            awt_submit_frame_json(&slot, &frame).unwrap()["changed"],
            true
        );
        assert_eq!(awt_close_json(&mut slot)["closed"], true);
    }

    // ---- Physical keyboard & mouse over the FFI (task 12) ------------------

    #[test]
    fn a_relative_mouse_batch_moves_the_pointer_without_a_position() {
        let mut slot: AwtSlot = None;
        awt_open_json(
            &mut slot,
            &json!({
                "screen": { "width": 64, "height": 32 },
                "surface": { "width": 64, "height": 32 }
            }),
        )
        .unwrap();
        let out = awt_input_json(
            &slot,
            &json!({ "events": [
                { "type": "capture", "captured": true },
                { "type": "pointer_relative", "dx": 6.0, "dy": 3.0, "source": "mouse" }
            ]}),
        )
        .unwrap();
        assert_eq!(out["rejected"], json!([]));
        assert_eq!(out["captured"], true);
        assert_eq!(out["pointer_mode"], "captured");
        assert_eq!(out["pointer"], json!({ "x": 6, "y": 3 }));
        assert_eq!(out["game_cursor"], json!({ "x": 38, "y": 19 }));
        // The grab state and the move both reached the game.
        let records = AwtEventRecord::decode_batch(&awt_drain_events_bytes(&slot)).unwrap();
        let native: Vec<GameInputEvent> = records
            .iter()
            .filter_map(GameInputEvent::from_record)
            .collect();
        assert!(native
            .iter()
            .any(|e| e.kind == game_event::GRAB_STATE && e.p0 == 1));
        assert!(native.iter().any(|e| e.kind == game_event::CURSOR_POS));
    }

    #[test]
    fn a_touch_is_filtered_while_the_pointer_is_captured() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        awt_configure_json(
            &slot,
            &json!({ "input": { "pointer_mode": "captured", "hybrid_touch": false } }),
        )
        .unwrap();
        let out = awt_input_json(
            &slot,
            &json!({ "events": [
                { "type": "pointer", "phase": "down", "x": 4.0, "y": 2.0, "source": "touch" }
            ]}),
        )
        .unwrap();
        assert_eq!(out["queued"], 0, "a palm must not click for you");
        assert_eq!(out["rejected"], json!([]), "filtered is not an error");
        // A mouse sample still gets through.
        let out = awt_input_json(
            &slot,
            &json!([{ "type": "pointer", "phase": "down", "x": 4.0, "y": 2.0, "source": "mouse" }]),
        )
        .unwrap();
        assert!(out["queued"].as_u64().unwrap() > 0);
    }

    #[test]
    fn a_hardware_key_forwards_its_scancode_to_the_game() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        let out = awt_input_json(
            &slot,
            &json!({ "events": [
                { "type": "key_down", "name": "key.keyboard.w", "scancode": 17 }
            ]}),
        )
        .unwrap();
        assert_eq!(out["rejected"], json!([]));
        let key = AwtEventRecord::decode_batch(&awt_drain_events_bytes(&slot))
            .unwrap()
            .iter()
            .filter_map(GameInputEvent::from_record)
            .find(|e| e.kind == game_event::KEY)
            .expect("the game must see the key");
        assert_eq!(key.p0, 'W' as i32, "GLFW_KEY_W");
        assert_eq!(
            key.p1, 17,
            "evdev KEY_W, straight from KeyEvent.getScanCode()"
        );
        assert_eq!(key.p2, 1, "GLFW_PRESS");
    }

    #[test]
    fn configure_applies_the_input_settings_and_reports_a_stale_binding() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        let out = awt_configure_json(
            &slot,
            &json!({ "input": {
                "sensitivity": { "x": 2.0, "y": 2.0, "invert_y": true },
                "scroll_permille": 2000,
                "bindings": { "keys": { "e": "f", "banana": "q" }, "buttons": { "1": 3 } },
            }}),
        )
        .unwrap();
        assert_eq!(out["input"]["sensitivity"]["x"], 2.0);
        assert_eq!(out["input"]["sensitivity"]["invert_y"], true);
        assert_eq!(out["input"]["scroll_permille"], 2000);
        assert_eq!(out["input"]["bindings"]["keys"]["e"], "f");
        assert_eq!(
            out["input_notes"].as_array().map(|a| a.len()),
            Some(1),
            "a stale binding must be reported, not swallowed: {out}"
        );
        // The remap is live: pressing "e" presses "f".
        awt_input_json(&slot, &json!([{ "type": "key_down", "name": "e" }])).unwrap();
        let records = AwtEventRecord::decode_batch(&awt_drain_events_bytes(&slot)).unwrap();
        assert!(records
            .iter()
            .filter_map(GameInputEvent::from_record)
            .any(|e| e.kind == game_event::KEY && e.p0 == 'F' as i32));
    }

    #[test]
    fn a_bad_capture_or_relative_event_never_kills_the_batch() {
        let mut slot: AwtSlot = None;
        open_tiny(&mut slot);
        let out = awt_input_json(
            &slot,
            &json!({ "events": [
                { "type": "pointer_relative" },
                { "type": "pointer_relative", "dx": "sideways" },
                { "type": "capture" },
                { "type": "capture", "captured": false },
                { "type": "key_down", "name": "escape", "scancode": -5 }
            ]}),
        )
        .unwrap();
        // A relative move with no numbers is a zero move: harmless, not an error.
        assert_eq!(out["rejected"], json!([]));
        assert_eq!(out["captured"], false, "the last capture wins");
        assert!(out["queued"].as_u64().unwrap() > 0, "escape still arrived");
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
        // `current` stays the built-in tag for backwards compatibility;
        // `current_tag` is the truth when a dynamic pack is selected.
        "current": crate::i18n::current_language().tag(),
        "current_tag": crate::i18n::current_language_tag(),
        "languages": crate::i18n::available_languages(),
        "pack_count": crate::i18n::pack::count(),
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
        // The pack tag when a dynamic language was selected.
        "tag_effective": crate::i18n::current_language_tag(),
        "dynamic": crate::i18n::current_scope().is_dynamic(),
    })
}

/// Resolve the language a request refers to: an explicit `language` tag,
/// otherwise the current UI language.
/// The [`crate::i18n::Scope`] a request addresses: a dynamically loaded language
/// pack when `language` names one, else the closest built-in.
///
/// Every JSON-in/JSON-out i18n helper resolves through this, so `{"language":"ja"}`
/// works for a runtime pack exactly as `{"language":"en"}` does for a built-in.
fn requested_scope(request: &serde_json::Value) -> crate::i18n::Scope {
    match request.get("language").and_then(|v| v.as_str()) {
        Some(tag) => crate::i18n::Scope::for_tag(tag),
        None => crate::i18n::current_scope(),
    }
}

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
    let scope = requested_scope(request);
    let key = request.get("key").and_then(|v| v.as_str()).unwrap_or("");
    if key.is_empty() {
        return serde_json::json!({
            "key": "",
            "value": "",
            "language": scope.tag(),
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
        // The *scope's* plural rule, so a pack declaring `_meta.plural` works.
        Some(n) => format!("{}.{}", key, scope.plural_rule().category(n).suffix()),
        None => key.to_string(),
    };
    let missing = crate::i18n::lookup_scoped(&scope, &effective_key).is_none();
    serde_json::json!({
        "key": effective_key,
        "value": crate::i18n::t_args_scoped(&scope, &effective_key, &args),
        "language": scope.tag(),
        "missing": missing,
        "dynamic": scope.is_dynamic(),
    })
}

/// Pure core of `RustBridge.i18nFormat` — locale-aware value formatting.
///
/// `request` = `{ "kind": "bytes"|"rate"|"int"|"decimal"|"percent"|"ratio"
///                       |"duration"|"eta"|"relative"|"fps"|"byte_progress",
///                "value"?: 1536, "total"?: 4096, "digits"?: 1,
///                "language"?: "en" }`
///
/// One crossing per label, and — crucially — the *same* implementation the core
/// itself uses (`crate::i18n::number`). Before this existed the Compose layer
/// carried a private English `B/KB/MB` ladder, so a Chinese user saw Chinese
/// prose wrapped around English units.
///
/// Never fails: an unknown `kind` reports `"supported": false` and echoes the
/// value through the plain integer formatter, so a version-skewed UI still
/// renders a number instead of an error (task-19 degradation contract).
pub fn i18n_format_json(request: &serde_json::Value) -> serde_json::Value {
    use crate::i18n::number;

    let scope = requested_scope(request);
    let kind = request
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("int")
        .trim()
        .to_ascii_lowercase();

    // Accept a JSON number *or* a numeric string: Kotlin `Long`s beyond 2^53
    // are safer to send as strings, and a UI that stringifies everything must
    // not silently format zero.
    let num = |field: &str| -> f64 {
        match request.get(field) {
            Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0),
            Some(serde_json::Value::String(s)) => s.trim().parse::<f64>().unwrap_or(0.0),
            _ => 0.0,
        }
    };
    let int = |field: &str| -> i64 {
        match request.get(field) {
            Some(serde_json::Value::Number(n)) => n
                .as_i64()
                .or_else(|| {
                    n.as_f64()
                        .map(|f| f.clamp(i64::MIN as f64, i64::MAX as f64) as i64)
                })
                .unwrap_or(0),
            Some(serde_json::Value::String(s)) => s.trim().parse::<i64>().unwrap_or(0),
            _ => 0,
        }
    };
    // Byte counts are unsigned; a negative request clamps to 0 rather than
    // wrapping around to 18 EB.
    let uint = |field: &str| -> u64 { int(field).max(0) as u64 };
    let digits = request
        .get("digits")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .min(number::MAX_FRACTION_DIGITS as u64) as usize;

    let mut supported = true;
    let text = match kind.as_str() {
        "bytes" | "size" => number::format_bytes(&scope, uint("value")),
        "rate" | "speed" => number::format_rate(&scope, uint("value")),
        "byte_progress" | "progress" => {
            number::format_byte_progress(&scope, uint("value"), uint("total"))
        }
        "int" | "integer" | "count" => number::format_int(&scope, int("value")),
        "decimal" | "number" => number::format_decimal(&scope, num("value"), digits),
        "percent" => number::format_percent(&scope, num("value"), digits),
        "ratio" => number::format_ratio_percent(&scope, uint("value"), uint("total"), digits),
        "duration" => match request.get("parts").and_then(|v| v.as_u64()) {
            Some(parts) => number::format_duration_parts(&scope, int("value"), parts as usize),
            None => number::format_duration(&scope, int("value")),
        },
        "eta" => number::format_eta(&scope, int("value")),
        "relative" | "relative_time" => number::format_relative_time(&scope, int("value")),
        "fps" => number::format_fps(&scope, num("value")),
        _ => {
            supported = false;
            number::format_int(&scope, int("value"))
        }
    };

    serde_json::json!({
        "kind": kind,
        "text": text,
        "language": scope.tag(),
        "supported": supported,
        "dynamic": scope.is_dynamic(),
    })
}

/// Every `kind` [`i18n_format_json`] understands — the Kotlin mirror and
/// `check_i18n.py` assert against this list so the two sides cannot drift.
pub fn i18n_format_kinds() -> &'static [&'static str] {
    &[
        "bytes",
        "rate",
        "byte_progress",
        "int",
        "decimal",
        "percent",
        "ratio",
        "duration",
        "eta",
        "relative",
        "fps",
    ]
}

/// Pure core of `RustBridge.i18nBundle` — the *whole* resolved catalogue.
///
/// `request` = `{ "language"?: "en" }`. Handing Kotlin the full map once (rather
/// than crossing the JNI boundary per string) keeps the UI allocation-free while
/// scrolling and guarantees it renders the same copy as the core.
pub fn i18n_bundle_json(request: &serde_json::Value) -> serde_json::Value {
    let scope = requested_scope(request);
    let messages: serde_json::Map<String, serde_json::Value> = crate::i18n::bundle_scoped(&scope)
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();
    serde_json::json!({
        "language": scope.tag(),
        "dynamic": scope.is_dynamic(),
        "plural": scope.plural_rule().id(),
        "rtl": scope.is_rtl(),
        "messages": serde_json::Value::Object(messages),
    })
}

/// Pure core of `RustBridge.i18nLanguagePacks` — **dynamic language loading**.
///
/// `request` = `{ "action": "list"|"load"|"install"|"remove"|"clear",
///                "path"?: "/data/.../files/i18n",   // load
///                "text"?: "_meta.tag = ja\n...",    // install
///                "tag"?:  "ja" }`                   // install fallback / remove
///
/// | action | effect |
/// |---|---|
/// | `list` (default) | every registered pack, plus the active one |
/// | `load` | scan a directory for `*.properties` packs |
/// | `install` | register one pack from an in-memory document |
/// | `remove` | unregister one pack (reverting the UI if it was active) |
/// | `clear` | unregister every pack |
///
/// This is what makes a community translation a *first-class language* rather
/// than a wording overlay: after `load`, the new tag appears in
/// `i18nLanguages()`, `i18nSetLanguage {"tag":"ja"}` selects it and
/// `i18nBundle {"language":"ja"}` hydrates the UI from it.
///
/// Never fails. Every rejected file is reported in `skipped` with a human reason
/// (too big, built-in tag, no messages, bad encoding, …) so the settings screen
/// can *show* the user why their pack did not appear — the single most common
/// support question for user-supplied translations.
pub fn i18n_language_packs_json(request: &serde_json::Value) -> serde_json::Value {
    use crate::i18n::pack;

    let action = request
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("list")
        .trim()
        .to_ascii_lowercase();
    let tag_arg = request.get("tag").and_then(|v| v.as_str());

    let mut loaded: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut ok = true;

    match action.as_str() {
        "load" | "dir" => match request.get("path").and_then(|v| v.as_str()) {
            Some(path) => {
                let report = pack::load_dir(std::path::Path::new(path));
                loaded = report.loaded;
                skipped = report.skipped;
            }
            None => {
                ok = false;
                skipped.push("no `path` given".to_string());
            }
        },
        "install" | "text" => match request.get("text").and_then(|v| v.as_str()) {
            Some(text) => match pack::install_text(text, tag_arg) {
                Ok(tag) => loaded.push(tag),
                Err(reason) => {
                    ok = false;
                    skipped.push(reason);
                }
            },
            None => {
                ok = false;
                skipped.push("no `text` given".to_string());
            }
        },
        "remove" | "uninstall" => match tag_arg {
            Some(tag) => {
                ok = pack::remove(tag);
                if !ok {
                    skipped.push(format!("{tag} is not a loaded language pack"));
                }
            }
            None => {
                ok = false;
                skipped.push("no `tag` given".to_string());
            }
        },
        "clear" => pack::clear(),
        // "list" and anything unrecognised: report state without mutating it.
        _ => {}
    }

    let required: Vec<String> = crate::i18n::all_keys().into_iter().collect();
    let packs: Vec<serde_json::Value> = pack::tags()
        .into_iter()
        .filter_map(|t| pack::describe(&t, &required))
        .collect();

    serde_json::json!({
        "action": action,
        "ok": ok,
        "loaded": loaded,
        "skipped": skipped,
        "packs": packs,
        "count": pack::count(),
        "active": pack::active(),
        "current": crate::i18n::current_language_tag(),
        "limits": { "max_packs": pack::MAX_PACKS, "max_bytes": pack::MAX_PACK_BYTES },
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

/// `RustBridge.i18nFormat(requestJson): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_i18nFormat(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    auth_ffi!({ i18n_json_call(&mut env, &request, i18n_format_json) })
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

/// `RustBridge.i18nLanguagePacks(requestJson): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_i18nLanguagePacks(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    auth_ffi!({ i18n_json_call(&mut env, &request, i18n_language_packs_json) })
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
    fn format_json_renders_every_kind_in_the_requested_language() {
        let _g = lock();
        let f =
            |req: serde_json::Value| i18n_format_json(&req)["text"].as_str().unwrap().to_string();

        assert_eq!(
            f(json!({ "kind": "bytes", "value": 1536, "language": "en" })),
            "1.5 KB"
        );
        assert_eq!(
            f(json!({ "kind": "rate", "value": 1258291, "language": "en" })),
            "1.2 MB/s"
        );
        assert_eq!(
            f(json!({ "kind": "rate", "value": 1258291, "language": "zh-CN" })),
            "1.2 MB/\u{79d2}"
        );
        assert_eq!(
            f(json!({ "kind": "int", "value": 1234567, "language": "en" })),
            "1,234,567"
        );
        assert_eq!(
            f(json!({ "kind": "decimal", "value": 1.25, "digits": 1, "language": "en" })),
            "1.3"
        );
        assert_eq!(
            f(json!({ "kind": "percent", "value": 42.5, "digits": 1, "language": "en" })),
            "42.5%"
        );
        assert_eq!(
            f(json!({ "kind": "ratio", "value": 1, "total": 4, "digits": 0, "language": "en" })),
            "25%"
        );
        assert_eq!(
            f(json!({ "kind": "duration", "value": 200, "language": "en" })),
            "3 minutes 20 seconds"
        );
        assert_eq!(
            f(json!({ "kind": "duration", "value": 200, "parts": 1, "language": "en" })),
            "3 minutes"
        );
        assert_eq!(
            f(json!({ "kind": "eta", "value": 200, "language": "zh-CN" })),
            "\u{5269}\u{4f59} 3 \u{5206} 20 \u{79d2}"
        );
        assert_eq!(
            f(json!({ "kind": "relative", "value": 120, "language": "en" })),
            "2 minutes ago"
        );
        assert_eq!(
            f(json!({ "kind": "fps", "value": 59.94, "language": "en" })),
            "59.9 FPS"
        );
        assert_eq!(
            f(
                json!({ "kind": "byte_progress", "value": 1048576, "total": 4194304, "language": "en" })
            ),
            "1.0 MB / 4.0 MB"
        );

        // Every advertised kind is actually handled.
        for kind in i18n_format_kinds() {
            let out = i18n_format_json(&json!({ "kind": kind, "value": 1, "total": 2 }));
            assert_eq!(out["supported"], true, "{kind}");
            assert!(!out["text"].as_str().unwrap().is_empty(), "{kind}");
        }
    }

    #[test]
    fn format_json_degrades_instead_of_failing() {
        let _g = lock();
        // Unknown kind: still a number, flagged unsupported (version skew).
        let out = i18n_format_json(&json!({ "kind": "quaternions", "value": 42 }));
        assert_eq!(out["supported"], false);
        assert_eq!(out["text"], "42");
        // Null / empty request: defaults to `int` of 0, never a panic.
        let out = i18n_format_json(&serde_json::Value::Null);
        assert_eq!(out["kind"], "int");
        assert_eq!(out["text"], "0");
        // Kind casing / padding is tolerated.
        assert_eq!(
            i18n_format_json(&json!({ "kind": "  BYTES  ", "value": 1024, "language": "en" }))
                ["text"],
            "1.0 KB"
        );
        // Long values may arrive as strings (Kotlin Long > 2^53 safety).
        assert_eq!(
            i18n_format_json(&json!({ "kind": "bytes", "value": "1536", "language": "en" }))
                ["text"],
            "1.5 KB"
        );
        // A negative byte count clamps to 0 rather than wrapping to 18 EB.
        assert_eq!(
            i18n_format_json(&json!({ "kind": "bytes", "value": -5, "language": "en" }))["text"],
            "0 B"
        );
        // Absurd precision is clamped, not honoured.
        let wide = i18n_format_json(&json!({
            "kind": "decimal", "value": 1.5, "digits": 9_999_999u64, "language": "en"
        }));
        assert!(wide["text"].as_str().unwrap().len() < 32);
        // Non-numeric junk formats as zero instead of erroring.
        assert_eq!(
            i18n_format_json(&json!({ "kind": "bytes", "value": "not-a-number" }))["text"],
            "0 B"
        );
        // Non-finite input never leaks "NaN"/"inf" into the UI.
        let nan = i18n_format_json(&json!({ "kind": "fps", "value": "NaN", "language": "en" }));
        assert!(!nan["text"].as_str().unwrap().to_lowercase().contains("nan"));
    }

    #[test]
    fn format_json_follows_the_current_language_when_none_is_given() {
        let _g = lock();
        let restore = crate::i18n::current_language();
        crate::i18n::set_language(Language::En);
        assert_eq!(
            i18n_format_json(&json!({ "kind": "bytes", "value": 1024 }))["text"],
            "1.0 KB"
        );
        crate::i18n::set_language(Language::ZhCn);
        assert_eq!(
            i18n_format_json(&json!({ "kind": "eta", "value": 30 }))["language"],
            "zh-CN"
        );
        crate::i18n::set_language(restore);
    }

    #[test]
    fn language_packs_load_select_and_unload_over_the_ffi() {
        let _g = lock();
        crate::i18n::pack::clear();
        let restore = crate::i18n::current_language();

        // Nothing loaded yet.
        let out = i18n_language_packs_json(&json!({ "action": "list" }));
        assert_eq!(out["count"], 0);
        assert_eq!(out["active"], serde_json::Value::Null);
        assert_eq!(out["limits"]["max_packs"], crate::i18n::pack::MAX_PACKS);

        // Install a pack from an in-memory document.
        let out = i18n_language_packs_json(&json!({
            "action": "install",
            "text": "_meta.tag = ja\n_meta.native_name = \u{65e5}\u{672c}\u{8a9e}\nnav.home = \u{30db}\u{30fc}\u{30e0}\n",
        }));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["loaded"], json!(["ja"]));
        assert_eq!(out["count"], 1);
        assert_eq!(out["packs"][0]["native_name"], "\u{65e5}\u{672c}\u{8a9e}");
        assert_eq!(out["packs"][0]["dynamic"], true);

        // It is now a first-class picker row.
        let langs = i18n_languages_json();
        assert_eq!(langs["pack_count"], 1);
        let listed = langs["languages"].as_array().unwrap();
        assert_eq!(listed.len(), 4);
        assert!(listed
            .iter()
            .any(|l| l["tag"] == "ja" && l["dynamic"] == true));

        // Selecting it by tag works, and reports the pack as effective.
        let set = i18n_set_language_json(&json!({ "tag": "ja" }));
        assert_eq!(set["tag_effective"], "ja");
        assert_eq!(set["dynamic"], true);
        assert_eq!(crate::i18n::current_language_tag(), "ja");

        // Translate + bundle resolve through the pack, falling back to Chinese.
        let t = i18n_translate_json(&json!({ "key": "nav.home", "language": "ja" }));
        assert_eq!(t["value"], "\u{30db}\u{30fc}\u{30e0}");
        assert_eq!(t["dynamic"], true);
        let t = i18n_translate_json(&json!({ "key": "nav.accounts", "language": "ja" }));
        assert_eq!(
            t["value"], "\u{8d26}\u{6237}",
            "untranslated -> Chinese, not a key"
        );
        let b = i18n_bundle_json(&json!({ "language": "ja" }));
        assert_eq!(b["language"], "ja");
        assert_eq!(b["dynamic"], true);
        assert_eq!(b["plural"], "other_only");
        let msgs = b["messages"].as_object().unwrap();
        assert_eq!(msgs["nav.home"], "\u{30db}\u{30fc}\u{30e0}");
        // Same key set as a built-in bundle: the UI never sees a hole.
        let zh = i18n_bundle_json(&json!({ "language": "zh-CN" }));
        assert_eq!(msgs.len(), zh["messages"].as_object().unwrap().len());
        // Value formatting follows the pack scope too.
        let f = i18n_format_json(&json!({ "kind": "bytes", "value": 1536, "language": "ja" }));
        assert_eq!(f["dynamic"], true);
        assert_eq!(f["language"], "ja");

        // Unloading the *active* pack must revert, not dangle.
        let out = i18n_language_packs_json(&json!({ "action": "remove", "tag": "ja" }));
        assert_eq!(out["ok"], true);
        assert_eq!(out["count"], 0);
        assert_eq!(out["active"], serde_json::Value::Null);
        assert_eq!(crate::i18n::current_language_tag(), "zh-CN");

        crate::i18n::pack::clear();
        crate::i18n::set_language(restore);
    }

    #[test]
    fn language_packs_degrade_instead_of_failing() {
        let _g = lock();
        crate::i18n::pack::clear();

        // Missing arguments are reported, never panics.
        for req in [
            json!({ "action": "load" }),
            json!({ "action": "install" }),
            json!({ "action": "remove" }),
        ] {
            let out = i18n_language_packs_json(&req);
            assert_eq!(out["ok"], false, "{req}");
            assert!(!out["skipped"].as_array().unwrap().is_empty(), "{req}");
        }
        // A tag that collides with a built-in is refused with a readable reason.
        let out = i18n_language_packs_json(&json!({
            "action": "install", "text": "nav.home = Home\n", "tag": "en"
        }));
        assert_eq!(out["ok"], false);
        assert!(
            out["skipped"][0].as_str().unwrap().contains("built-in"),
            "{out}"
        );
        // A non-existent directory loads nothing and is not an error state.
        let out = i18n_language_packs_json(&json!({
            "action": "load", "path": "/definitely/not/here/rc-i18n"
        }));
        assert_eq!(out["loaded"], json!([]));
        assert_eq!(out["count"], 0);
        // Unknown action / null request: report state, mutate nothing.
        assert_eq!(
            i18n_language_packs_json(&json!({ "action": "explode" }))["count"],
            0
        );
        let out = i18n_language_packs_json(&serde_json::Value::Null);
        assert_eq!(out["action"], "list");
        assert_eq!(out["ok"], true);
        // Removing something that was never there.
        let out = i18n_language_packs_json(&json!({ "action": "remove", "tag": "ko" }));
        assert_eq!(out["ok"], false);

        // An unknown `language` tag still renders (Chinese-first), never errors.
        let t = i18n_translate_json(&json!({ "key": "nav.home", "language": "ja" }));
        assert_eq!(t["dynamic"], false);
        assert_eq!(t["value"], "\u{4e3b}\u{9875}");

        crate::i18n::pack::clear();
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

#[cfg(test)]
mod translate_tests {
    use super::*;

    /// The translation FFI round-trips: a known mod id is served from the
    /// built-in dictionary; an unknown phrase is processed by the term
    /// substitutor; the result always has a non-empty translated field.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn translate_ffi_round_trips_known_mod() {
        // Force an offline service so the test never depends on the
        // gateway.
        let _ = translate_init_json(&json!({
            "force_offline": true,
            "cache_root": tempfile::tempdir().unwrap().path().to_string_lossy(),
            "default_mode": "hybrid",
        }));

        let out = translate_json(&json!({
            "text": "sodium",
            "target": "zh-CN",
        }))
        .unwrap();
        assert_eq!(out["source"], "dictionary");
        assert_eq!(out["translated"], "钠");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn translate_ffi_handles_unknown_phrase() {
        let _ = translate_init_json(&json!({
            "force_offline": true,
            "cache_root": tempfile::tempdir().unwrap().path().to_string_lossy(),
            "default_mode": "hybrid",
        }));

        let out = translate_json(&json!({
            "text": "Improves FPS and render distance for the server.",
            "target": "zh-CN",
        }))
        .unwrap();
        // Offline + hybrid ⇒ term substitution kicks in.
        assert!(out["translated"].as_str().unwrap().contains("帧率"));
        assert!(out["translated"].as_str().unwrap().contains("渲染距离"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn translate_ffi_batch_preserves_order() {
        let _ = translate_init_json(&json!({
            "force_offline": true,
            "cache_root": tempfile::tempdir().unwrap().path().to_string_lossy(),
            "default_mode": "hybrid",
        }));

        let reqs = json!([
            { "text": "sodium",    "target": "zh-CN" },
            { "text": "iris",      "target": "zh-CN" },
            { "text": "fabric-api", "target": "zh-CN" },
        ]);
        let out = translate_batch_json(&reqs).unwrap();
        let arr = out.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["translated"], "钠");
        assert_eq!(arr[1]["translated"], "虹膜");
        assert_eq!(arr[2]["translated"], "Fabric API");
    }

    #[test]
    fn translate_languages_json_lists_every_target() {
        let out = translate_languages_json();
        let langs = out["languages"].as_array().unwrap();
        assert!(langs.iter().any(|l| l["tag"] == "auto"));
        assert!(langs.iter().any(|l| l["tag"] == "zh-CN"));
        assert!(langs.iter().any(|l| l["tag"] == "zh-Hant"));
        assert!(langs.iter().any(|l| l["tag"] == "en"));
        let modes = out["modes"].as_array().unwrap();
        assert!(modes.iter().any(|m| m == "online"));
        assert!(modes.iter().any(|m| m == "offline"));
        assert!(modes.iter().any(|m| m == "hybrid"));
    }

    #[test]
    fn translate_ffi_bad_request_returns_error_json() {
        // An unparseable request returns an error JSON, not a panic.
        // The unit-tested pure core is what catches the bad payload.
        let raw = serde_json::from_str::<serde_json::Value>("{}").unwrap();
        let res = translate_json(&raw);
        // An empty `{ "text": "" }` is valid (passthrough), so we need
        // a clearly-bad payload to test the error path.
        let raw = serde_json::json!({ "text": 42 });
        let res = translate_json(&raw);
        assert!(res.is_err(), "a number in `text` must be rejected");
    }

    // === Task 16: complete and auto-updating game-version list ===============
    //
    // The JNI exports are thin wrappers around [`crate::game::version_list`];
    // the heavy lifting is unit-tested in `version_list.rs` already. Here we
    // only cover the few pure helpers that bridge the JSON envelopes.

    #[test]
    fn version_list_envelope_carries_manifest_groups_and_info() {
        // Build a tiny manifest and confirm the envelope serialises every
        // documented key. This is the shape the Compose UI consumes.
        let manifest = crate::game::VersionManifest {
            latest: crate::game::manifest::Latest {
                release: "1.20.4".to_string(),
                snapshot: "24w03a".to_string(),
            },
            versions: vec![
                crate::game::manifest::VersionEntry {
                    id: "1.20.4".to_string(),
                    kind: "release".to_string(),
                    url: "https://x/1.20.4.json".to_string(),
                    sha1: None,
                    time: None,
                    release_time: None,
                },
                crate::game::manifest::VersionEntry {
                    id: "24w03a".to_string(),
                    kind: "snapshot".to_string(),
                    url: "https://x/24w03a.json".to_string(),
                    sha1: None,
                    time: None,
                    release_time: None,
                },
            ],
        };
        let groups = crate::game::version_list::VersionGroups::from(&manifest);
        let info = crate::game::version_list::VersionListInfo {
            fresh: true,
            fetched_at_unix: Some(1_700_000_000),
            stale_fallback: false,
            offline_only: false,
            total: 2,
            groups: groups.counts(),
        };
        let env = version_list_envelope(&manifest, info.clone(), &groups);
        assert!(env.get("manifest").is_some());
        assert!(env.get("info").is_some());
        assert!(env.get("groups").is_some());
        // Every bucket key is present even when empty.
        for k in [
            "release",
            "snapshot",
            "pre_release",
            "old_alpha",
            "old_beta",
            "special",
        ] {
            assert!(env["groups"].get(k).is_some(), "missing group {k}");
        }
        // Counts match the manifest length.
        let info_obj = &env["info"];
        assert_eq!(info_obj["fresh"], serde_json::json!(true));
        assert_eq!(info_obj["total"], serde_json::json!(2));
        assert_eq!(info_obj["stale_fallback"], serde_json::json!(false));
    }

    #[test]
    fn version_list_info_serialises_to_expected_keys() {
        // The Kotlin typed wrapper reads `info` directly; make sure every
        // field is serialisable and round-trips back to the same struct.
        let info = crate::game::version_list::VersionListInfo {
            fresh: false,
            fetched_at_unix: None,
            stale_fallback: true,
            offline_only: false,
            total: 10,
            groups: std::collections::BTreeMap::new(),
        };
        let s = serde_json::to_string(&info).expect("serialise info");
        let back: crate::game::version_list::VersionListInfo =
            serde_json::from_str(&s).expect("parse info");
        assert_eq!(info, back);
    }

    #[test]
    fn version_list_cache_offline_manifest_is_nonempty() {
        // The Kotlin UI's offline path starts from this manifest, so the
        // built-in DB must always provide enough entries to populate the
        // picker (release/snapshot/special buckets all need at least one row).
        let offline = crate::game::version_list::VersionListCache::offline_manifest();
        let groups = crate::game::version_list::VersionGroups::from(&offline);
        assert!(!groups.release.is_empty(), "offline list lacks releases");
        assert!(
            !groups.special.is_empty(),
            "offline list lacks special / modded entries"
        );
        assert!(
            offline.versions.len() > 100,
            "offline list should be populated from FCL parity DB"
        );
    }
}

#[cfg(test)]
mod modpack_ffi_tests {
    use super::*;

    /// Round-trip the modpack inspector: a Modrinth index text goes in,
    /// the normalised spec JSON comes out with the right flavour.
    #[test]
    fn modpack_inspect_returns_normalised_spec() {
        let json = r#"{"formatVersion":1,"game":"minecraft","name":"Sample","dependencies":{"minecraft":"1.20.1","fabric-loader":"0.16.0"},"files":[]}"#;
        let manifest = crate::mods::modpack::detect_and_parse(json, Some("https://e/x")).unwrap();
        let serialised = serde_json::to_string(&manifest).unwrap();
        assert!(serialised.contains("\"fabric\""));
        assert!(serialised.contains("\"1.20.1\""));
    }

    /// Detect+parse from raw manifest text returns the same envelope.
    #[test]
    fn modpack_detect_handles_all_three_flavours() {
        let modrinth = r#"{"formatVersion":1,"game":"minecraft","name":"x","dependencies":{"minecraft":"1.20.1"},"files":[]}"#;
        let curse = r#"{"manifestType":"minecraftModpack","manifestVersion":1,"name":"x","minecraft":{"version":"1.20.1","modLoaders":[]},"files":[]}"#;
        let mmc = r#"{"formatVersion":1,"name":"x","mcVersion":"1.20.1","mods":[]}"#;

        let a = crate::mods::modpack::detect_and_parse(modrinth, None).unwrap();
        let b = crate::mods::modpack::detect_and_parse(curse, None).unwrap();
        let c = crate::mods::modpack::detect_and_parse(mmc, None).unwrap();
        assert_eq!(
            a.spec().flavour,
            crate::mods::modpack::ModpackFlavour::Modrinth
        );
        assert_eq!(
            b.spec().flavour,
            crate::mods::modpack::ModpackFlavour::CurseForge
        );
        assert_eq!(
            c.spec().flavour,
            crate::mods::modpack::ModpackFlavour::MultiMc
        );
    }

    /// Helper: build a `build_importer_from_config` from a config JSON.
    #[tokio::test(flavor = "current_thread")]
    async fn build_importer_rejects_missing_instances_root() {
        let result = build_importer_from_config("{}");
        assert!(result.is_err());
    }
}

// === File manager FFI (task 19) ============================================
//
// JSON-in / JSON-out bridge for the in-app small file manager. Every entry
// point:
//   * unwraps a single `String` argument containing a JSON object,
//   * delegates to a pure-Rust helper in `crate::fs_ops`,
//   * returns either the typed JSON success payload or
//     `{"error": "...", "kind": "..."}` on failure.
//
// Path-traversal is prevented by `fs_ops::resolve_under_roots`, which
// canonicalises both the target path and the configured allowed roots and
// rejects anything that escapes them. The UI therefore only ever sees
// operations that were *explicitly* allowed for the current screen (the
// per-instance file manager, the world manager, the resource browser, ...).

use crate::fs_ops as file_ops;

/// `RustBridge.fsListDir(requestJson): String`
///
/// `requestJson` = `{ "path": String, "roots": [String] }`.
/// Returns the [`fs_ops::FsListing`] JSON, or an error envelope.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_fsListDir(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request"),
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad request: {e}")),
        };
        let out = file_ops::list_dir(parsed)
            .map(serde_json::to_value)
            .and_then(|v| v.map_err(RcError::from));
        rc_to_json(&mut env, out)
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.fsMkdir(requestJson): String`
///
/// `requestJson` = `{ "parent": String, "name": String, "roots": [String] }`.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_fsMkdir(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request"),
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad request: {e}")),
        };
        let out = file_ops::mkdir(parsed)
            .map(serde_json::to_value)
            .and_then(|v| v.map_err(RcError::from));
        rc_to_json(&mut env, out)
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.fsCopy(requestJson): String`
///
/// `requestJson` = `{ "source": String, "destination": String, "roots":
/// [String] }`. Both paths must resolve under an allowed root; the
/// destination must not already exist.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_fsCopy(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request"),
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad request: {e}")),
        };
        let out = file_ops::copy_path(parsed)
            .map(serde_json::to_value)
            .and_then(|v| v.map_err(RcError::from));
        rc_to_json(&mut env, out)
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.fsMove(requestJson): String`
///
/// `requestJson` = `{ "source": String, "destination": String, "roots":
/// [String], "confirm": Bool }`. With `confirm: false` and an existing
/// destination, returns an `FsOpPreview` JSON so the UI can show a
/// confirmation dialog before applying the move.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_fsMove(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request"),
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad request: {e}")),
        };
        let out = file_ops::move_path(parsed);
        rc_to_json(&mut env, out)
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.fsRename(requestJson): String`
///
/// `requestJson` = `{ "path": String, "new_name": String, "roots":
/// [String] }`. The new name is checked for `..` / absolute / drive-prefix
/// components.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_fsRename(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request"),
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad request: {e}")),
        };
        let out = file_ops::rename_path(parsed)
            .map(serde_json::to_value)
            .and_then(|v| v.map_err(RcError::from));
        rc_to_json(&mut env, out)
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.fsDelete(requestJson): String`
///
/// `requestJson` = `{ "paths": [String], "roots": [String], "confirm":
/// Bool }`. Without `confirm: true`, returns an `FsOpPreview` so the UI can
/// ask the user to confirm the deletion.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_fsDelete(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request"),
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad request: {e}")),
        };
        let out = file_ops::delete_paths(parsed);
        rc_to_json(&mut env, out)
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.fsExtractZip(requestJson): String`
///
/// `requestJson` = `{ "archive": String, "destination": String, "roots":
/// [String] }`. Used by the import buttons on the mod / resource-pack /
/// shader-pack browsers.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_fsExtractZip(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request"),
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad request: {e}")),
        };
        let out = file_ops::extract_zip(parsed)
            .map(serde_json::to_value)
            .and_then(|v| v.map_err(RcError::from));
        rc_to_json(&mut env, out)
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.fsImportBytes(requestJson): String`
///
/// `requestJson` = `{ "destination": String, "data_b64": String, "roots":
/// [String] }`. Used to import a file from a `content://` URI: the Android
/// side streams the bytes, base64-encodes them, and hands them to the core
/// to write under the allowed root.
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_fsImportBytes(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request"),
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad request: {e}")),
        };
        let out = file_ops::import_bytes(parsed)
            .map(serde_json::to_value)
            .and_then(|v| v.map_err(RcError::from));
        rc_to_json(&mut env, out)
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

#[cfg(test)]
mod fs_ops_ffi_tests {
    use super::*;
    use std::io::Write as _;
    use tempfile::tempdir;

    /// Each public fs_ops helper round-trips through the same JSON shape the
    /// JNI surface produces. The Kotlin side parses the same field names, so a
    /// drift here would break the Compose file manager.
    #[test]
    fn list_dir_round_trip() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.txt")).unwrap();
        f.write_all(b"x").unwrap();
        let req = serde_json::json!({
            "path": dir.path().to_string_lossy(),
            "roots": [dir.path().to_string_lossy()],
        });
        let v: serde_json::Value = file_ops::list_dir(req)
            .map(serde_json::to_value)
            .unwrap()
            .unwrap();
        assert_eq!(
            v["path"].as_str().unwrap(),
            dir.path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        );
        assert_eq!(v["entries"].as_array().unwrap().len(), 2);
        assert_eq!(v["dir_count"], 1);
        assert_eq!(v["file_count"], 1);
        // The Kotlin side reads `kind` as one of file/dir/link/other.
        let kinds: Vec<&str> = v["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["kind"].as_str().unwrap())
            .collect();
        assert!(kinds.contains(&"dir"));
        assert!(kinds.contains(&"file"));
    }

    #[test]
    fn delete_preview_shape() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        let req = serde_json::json!({
            "paths": [f.to_string_lossy()],
            "roots": [dir.path().to_string_lossy()],
        });
        let v: serde_json::Value = file_ops::delete_paths(req).unwrap();
        assert_eq!(v["op"], "delete");
        assert_eq!(v["has_directories"], false);
        assert_eq!(v["targets"].as_array().unwrap().len(), 1);
        assert_eq!(v["total_bytes"], 1);
        assert!(f.exists(), "preview must not delete");
    }
}

// === Crash report management FFI (task 24) ===================================
//
// Pure JSON helpers + panic-safe JNI wrappers that give the Compose UI access to
// the persisted crash-log store and the process-wide log ring that backs it.
// These complete the crash-report pipeline: `launchDiagnose` (above) classifies
// a single finished session, and `crashListLogs` / `crashInstallReporter` /
// `crashPruneLogs` let the crash history screen enumerate, persist and prune
// the accumulated reports while `crashRecentLogs` feeds the live diagnostics
// panel.

/// `RustBridge.crashListLogs(requestJson): String`
///
/// `requestJson` = `{ "dir": String }` — the crash directory (usually
/// `<data_root>/crash/`). Returns `{ "ok": true, "logs": [CrashLog, ...],
/// "count": N }` or `{ "ok": false, "error": "..." }`.
///
/// Each `CrashLog` serialises to `{ id, timestamp, kind, message, backtrace?,
/// logs: [{ts, level, line}], context? }`. The `context` field already carries
/// the full `CrashReport` verdict (category, evidence, recovery, device_info)
/// produced by `LaunchEngine`, so the Kotlin screen can render a compact list
/// without a second JNI call.
pub fn crash_list_logs_json(request: &serde_json::Value) -> RcResult<serde_json::Value> {
    let dir = match request.get("dir").and_then(|v| v.as_str()) {
        Some(p) => PathBuf::from(p),
        None => return Err(crate::RcError::Launch("missing `dir`".into())),
    };
    let logs = crate::robust::list_crash_logs(&dir)?;
    let json_logs: Vec<serde_json::Value> = serde_json::to_value(&logs)
        .map(|v| v.as_array().unwrap_or(&Vec::new()).clone())
        .unwrap_or_default();
    Ok(json!({
        "ok": true,
        "count": json_logs.len(),
        "logs": json_logs,
    }))
}

/// `RustBridge.crashRecentLogs(requestJson): String`
///
/// `requestJson` = `{ "n": Int? }` (default 200). Returns
/// `{ "ok": true, "logs": [{ts, level, line}, ...] }` — the most recent `n`
/// lines captured by the process-wide log ring, newest first. Used by the
/// diagnostics card and the crash snapshot (tasks 10, 19, 24).
pub fn crash_recent_logs_json(request: &serde_json::Value) -> serde_json::Value {
    let n = request.get("n").and_then(|v| v.as_u64()).unwrap_or(200) as usize;
    let logs = crate::robust::recent_logs(n);
    json!({
        "ok": true,
        "n": logs.len(),
        "logs": logs,
    })
}

/// `RustBridge.crashInstallReporter(requestJson): String`
///
/// `requestJson` = `{ "data_root": String }`. Installs the process-wide panic
/// hook that writes crash logs under `<data_root>/crash/` and emits an
/// `error` event on the bus (task 10). Idempotent — a second call returns
/// `{"ok": true, "installed": false, "already_installed": true}`.
pub fn crash_install_reporter_json(request: &serde_json::Value) -> serde_json::Value {
    let data_root = match request.get("data_root").and_then(|v| v.as_str()) {
        Some(p) => PathBuf::from(p),
        None => return json!({ "ok": false, "error": "missing `data_root`", "installed": false }),
    };
    let installed = crate::robust::install_crash_reporter(data_root);
    json!({
        "ok": true,
        "installed": installed,
        "already_installed": !installed,
    })
}

/// `RustBridge.crashPruneLogs(requestJson): String`
///
/// `requestJson` = `{ "dir": String, "keep": Int? }` (default 50). Deletes the
/// oldest crash logs beyond `keep`, returning `{"ok": true, "removed": N}`.
pub fn crash_prune_logs_json(request: &serde_json::Value) -> RcResult<serde_json::Value> {
    let dir = match request.get("dir").and_then(|v| v.as_str()) {
        Some(p) => PathBuf::from(p),
        None => return Err(crate::RcError::Launch("missing `dir`".into())),
    };
    let keep = request.get("keep").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
    let removed = crate::robust::prune_crash_logs(&dir, keep)?;
    Ok(json!({
        "ok": true,
        "removed": removed,
        "remaining": crate::robust::list_crash_logs(&dir)?.len() as u64,
    }))
}

/// `RustBridge.crashListLogs(requestJson): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_crashListLogs(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request"),
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad request: {e}")),
        };
        rc_to_json(&mut env, crash_list_logs_json(&value))
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.crashRecentLogs(requestJson): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_crashRecentLogs(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request"),
        };
        let value: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
        jstr(&mut env, &crash_recent_logs_json(&value).to_string())
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.crashInstallReporter(requestJson): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_crashInstallReporter(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request"),
        };
        let value: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
        jstr(&mut env, &crash_install_reporter_json(&value).to_string())
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

/// `RustBridge.crashPruneLogs(requestJson): String`
#[no_mangle]
pub extern "system" fn Java_com_rc_launcher_core_RustBridge_crashPruneLogs(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = match read_input(&mut env, &request) {
            Some(s) => s,
            None => return err_json(&mut env, "missing request"),
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return err_json(&mut env, &format!("bad request: {e}")),
        };
        rc_to_json(&mut env, crash_prune_logs_json(&value))
    }));
    match built {
        Ok(s) => s,
        Err(_) => std::ptr::null_mut(),
    }
}

#[cfg(test)]
mod crash_ffi_tests {
    use super::*;
    use tempfile::tempdir;

    /// `crashListLogs` round-trips a persisted CrashLog as JSON with all the
    /// fields the Compose crash-history screen needs.
    #[test]
    fn crash_list_logs_round_trips() {
        let dir = tempdir().unwrap();
        let report = crate::robust::CrashLog::new("out_of_memory", "the game ran out of memory")
            .with_context(serde_json::json!({
                "category": "out_of_memory",
                "evidence": ["java.lang.OutOfMemoryError: Java heap space"],
                "device_info": { "abi": "arm64-v8a", "renderer": "gl4es" },
                "actions": ["copy_details", "switch_renderer"],
                "recovery": { "description": "re_download_natives", "suggest_proxy": true },
            }));
        crate::robust::write_crash_log(dir.path(), &report).unwrap();

        let req = json!({ "dir": dir.path().to_string_lossy() });
        let out = crash_list_logs_json(&req).unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(out["count"], 1);
        let first = out["logs"].as_array().unwrap().first().unwrap();
        assert_eq!(first["kind"], "out_of_memory");
        assert_eq!(first["message"], "the game ran out of memory");
        // The context carries the full crash verdict.
        assert_eq!(first["context"]["category"], "out_of_memory");
        assert!(first["context"]["recovery"]["suggest_proxy"]
            .as_bool()
            .unwrap());
    }

    /// `crashRecentLogs` degrades gracefully when the ring is empty.
    #[test]
    fn crash_recent_logs_empty_degrades_gracefully() {
        // The global ring may have content from other tests; we just assert
        // the shape is correct and never panics.
        let out = crash_recent_logs_json(&json!({}));
        assert_eq!(out["ok"], true);
        assert!(out["logs"].is_array());
    }

    /// `crashInstallReporter` without `data_root` reports a clean error.
    /// (We deliberately do not test the *success* path here: `install_crash_reporter`
    /// uses a process-global `OnceLock` that the `robust::reporter` test suite
    /// already exercises; calling it twice from different modules causes
    /// nondeterministic failures.)
    #[test]
    fn crash_install_reporter_missing_dir_is_error() {
        let out = crash_install_reporter_json(&json!({}));
        assert_eq!(out["ok"], false);
        assert!(out["error"].as_str().unwrap().contains("data_root"));
    }

    /// `crashListLogs` without `dir` is a Launch error.
    #[test]
    fn crash_list_logs_missing_dir_is_error() {
        let result = crash_list_logs_json(&json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing `dir`"));
    }

    /// `crashPruneLogs` removes the oldest reports beyond `keep`.
    #[test]
    fn crash_prune_logs_removes_oldest() {
        let dir = tempdir().unwrap();
        // Write three crash logs.
        for i in 0..3 {
            let mut r = crate::robust::CrashLog::new("panic", format!("crash {i}"));
            r.timestamp = 1000 + i as u64;
            crate::robust::write_crash_log(dir.path(), &r).unwrap();
        }
        let req = json!({ "dir": dir.path().to_string_lossy(), "keep": 1 });
        let out = crash_prune_logs_json(&req).unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(out["removed"], 2);
        assert_eq!(out["remaining"], 1);
    }

    /// `crashListLogs` on a non-existent directory returns an IO error,
    /// not a panic.
    #[test]
    fn crash_list_logs_missing_dir_is_error_not_panic() {
        let req = json!({ "dir": "/no/such/crash/dir/here" });
        let result = crash_list_logs_json(&req);
        assert!(result.is_err());
    }

    /// `crashRecentLogs` with an explicit `n` honours the limit.
    #[test]
    fn crash_recent_logs_honours_n() {
        crate::robust::record_log("error", "marker-line-for-recent-logs-test");
        let out = crash_recent_logs_json(&json!({ "n": 3 }));
        assert!(out["logs"].as_array().unwrap().len() <= 3);
    }
}
