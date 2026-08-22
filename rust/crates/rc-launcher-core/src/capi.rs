//! C ABI surface of the RC Launcher core (task 10).
//!
//! This module is the single source of truth for `rc_launcher.h`, produced by
//! `cbindgen` (see `cbindgen.toml` at the crate root). It deliberately mirrors
//! MCTier's `libeasytier_ffi.so`: a flat, `#[repr(C)]`-friendly C API with
//! function-pointer callbacks, consumed by a *thin* JNI wrapper (`ffi`) and by
//! any other native consumer (Unity/Unreal plugins, CLI tools, tests). All
//! long-running work is asynchronous and reports through an event callback — it
//! never blocks the caller.
//!
//! Only this file is scanned by `cbindgen`, so the generated header contains
//! exactly the C contract and no JNI/serde internals.

use std::ffi::{c_char, c_int, c_void, CStr, CString};

use crate::event::{self, Event};
use crate::jobs;

/// Callback invoked for every published event.
///
/// `json` is a NUL-terminated UTF-8 string (owned by the callee for the
/// duration of the call); `userdata` is the pointer passed to
/// [`rc_event_bus_subscribe`] so the C side can recover its own context (the
/// classic `void*` userdata idiom).
pub type RcEventCallback = unsafe extern "C" fn(*const c_char, *mut c_void);

/// C implementation of [`event::EventSink`] that forwards events to a C
/// function pointer. `userdata` is passed back on every call.
struct CEventSink {
    cb: RcEventCallback,
    userdata: *mut c_void,
}

// SAFETY: `cb` is a `extern "C"` function pointer (always safe to copy/call
// across the FFI). `userdata` is a raw pointer the C side owns and must keep
// alive for the lifetime of the subscription. The sink is only ever invoked
// from Rust threads, so we assert `Send + Sync` to store it in the global bus.
unsafe impl Send for CEventSink {}
unsafe impl Sync for CEventSink {}

impl event::EventSink for CEventSink {
    fn emit(&self, event: &Event) {
        let json = match CString::new(event.to_json()) {
            Ok(c) => c,
            // A NUL byte cannot appear in valid event JSON; bail if it ever does.
            Err(_) => return,
        };
        // SAFETY: `cb` is a `unsafe extern "C"` function pointer supplied by the
        // C consumer; `json` is a valid NUL-terminated C string for the duration
        // of the call and `userdata` is the pointer the consumer registered.
        unsafe { (self.cb)(json.as_ptr(), self.userdata) };
    }
}

/// Subscribe a C event callback. `cb` is invoked with a NUL-terminated JSON
/// string and the `userdata` pointer for every published event. Returns `1` if
/// a previous callback was replaced, `0` otherwise.
///
/// # Safety
/// `cb` must be a valid `extern "C"` function pointer. `userdata` must outlive
/// the subscription (until [`rc_event_bus_unsubscribe`] is called).
#[no_mangle]
pub unsafe extern "C" fn rc_event_bus_subscribe(
    cb: RcEventCallback,
    userdata: *mut c_void,
) -> c_int {
    let sink: ArcEventSink = std::sync::Arc::new(CEventSink { cb, userdata });
    if event::subscribe(sink) {
        1
    } else {
        0
    }
}

// Helper alias to keep the coercion readable; `Arc<CEventSink>` →
// `Arc<dyn event::EventSink>` is performed at the `subscribe` call site.
type ArcEventSink = std::sync::Arc<CEventSink>;

/// Remove the current C event callback. Subsequent publishes become no-ops.
#[no_mangle]
pub extern "C" fn rc_event_bus_unsubscribe() {
    event::unsubscribe();
}

/// Whether a callback is currently subscribed (`1` / `0`).
#[no_mangle]
pub extern "C" fn rc_event_bus_has_sink() -> c_int {
    if event::has_sink() {
        1
    } else {
        0
    }
}

/// Publish a pre-serialised JSON event from the C side (e.g. to replay a log).
/// Returns `1` on success, `0` if `json` is null or not valid event JSON.
///
/// # Safety
/// `json` must be a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn rc_event_bus_publish(json: *const c_char) -> c_int {
    if json.is_null() {
        return 0;
    }
    let cstr = match CStr::from_ptr(json).to_str() {
        Ok(s) => s,
        Err(_) => return 0,
    };
    if event::publish_json(cstr) {
        1
    } else {
        0
    }
}

/// C mirror of the JNI `runAsync`: spawn a background job that reports through
/// the event bus and returns immediately. `spec_json` is the same JSON accepted
/// by the JNI `runAsync`. Returns a NUL-terminated JSON string
/// `{ "ok": bool, "scope": string }`; the caller must free it with
/// [`rc_string_free`].
///
/// # Safety
/// `spec_json` must be a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn rc_run_async(spec_json: *const c_char) -> *mut c_char {
    let empty = || CString::new("{\"ok\":false,\"error\":\"null spec\"}").unwrap_or_default();
    if spec_json.is_null() {
        return empty().into_raw();
    }
    let cstr = match CStr::from_ptr(spec_json).to_str() {
        Ok(s) => s,
        Err(_) => {
            return CString::new("{\"ok\":false,\"error\":\"bad utf8\"}")
                .unwrap_or_default()
                .into_raw()
        }
    };
    let spec: serde_json::Value = match serde_json::from_str(cstr) {
        Ok(v) => v,
        Err(e) => {
            return CString::new(format!("{{\"ok\":false,\"error\":\"{e}\"}}"))
                .unwrap_or_default()
                .into_raw()
        }
    };
    match jobs::spawn_job(&spec) {
        Ok(v) => CString::new(v.to_string()).unwrap_or_default().into_raw(),
        Err(e) => CString::new(format!("{{\"ok\":false,\"error\":\"{e}\"}}"))
            .unwrap_or_default()
            .into_raw(),
    }
}

/// Cancel a running async job by scope. Returns `1` if a job was found (and will
/// observe the flag on its next poll), `0` otherwise.
///
/// # Safety
/// `scope` must be a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn rc_cancel_async(scope: *const c_char) -> c_int {
    if scope.is_null() {
        return 0;
    }
    let cstr = match CStr::from_ptr(scope).to_str() {
        Ok(s) => s,
        Err(_) => return 0,
    };
    if jobs::cancel_job(cstr) {
        1
    } else {
        0
    }
}

// === Internationalisation (task 20) ========================================
//
// The C mirror of the i18n JNI surface, so non-JNI consumers (a CLI, a Unity /
// Unreal plugin, integration tests) get the *same* translated copy as the app.
// Every returned string is heap-allocated JSON that the caller frees with
// [`rc_string_free`].

/// Helper: run a JSON-in / JSON-out i18n call from a C string.
///
/// A null / non-UTF-8 / unparseable argument degrades to JSON `null`, letting
/// each helper apply its documented defaults instead of failing.
unsafe fn i18n_call(
    request_json: *const c_char,
    f: impl FnOnce(&serde_json::Value) -> serde_json::Value,
) -> *mut c_char {
    let value = if request_json.is_null() {
        serde_json::Value::Null
    } else {
        CStr::from_ptr(request_json)
            .to_str()
            .ok()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::Value::Null)
    };
    CString::new(f(&value).to_string())
        .unwrap_or_default()
        .into_raw()
}

/// The shipped language catalogue as JSON (see `ffi::i18n_languages_json`).
/// Free the result with [`rc_string_free`].
#[no_mangle]
pub extern "C" fn rc_i18n_languages() -> *mut c_char {
    CString::new(crate::ffi::i18n_languages_json().to_string())
        .unwrap_or_default()
        .into_raw()
}

/// The current UI language tag (e.g. `zh-CN`). Free with [`rc_string_free`].
#[no_mangle]
pub extern "C" fn rc_i18n_current_language() -> *mut c_char {
    CString::new(crate::i18n::current_language().tag())
        .unwrap_or_default()
        .into_raw()
}

/// Switch the UI language. `request_json` = `{ "tag": "en" }` or
/// `{ "preferred": ["zh-Hant-TW", "en"] }`; unknown values resolve to the
/// Chinese-first base locale. Returns the applied language as JSON.
///
/// # Safety
/// `request_json` must be a NUL-terminated UTF-8 string (or null).
#[no_mangle]
pub unsafe extern "C" fn rc_i18n_set_language(request_json: *const c_char) -> *mut c_char {
    i18n_call(request_json, crate::ffi::i18n_set_language_json)
}

/// Translate one key. `request_json` = `{ "key": ..., "language"?: ...,
/// "args"?: {...}, "count"?: n }`. Returns `{ key, value, language, missing }`.
///
/// # Safety
/// `request_json` must be a NUL-terminated UTF-8 string (or null).
#[no_mangle]
pub unsafe extern "C" fn rc_i18n_translate(request_json: *const c_char) -> *mut c_char {
    i18n_call(request_json, crate::ffi::i18n_translate_json)
}

/// The whole resolved catalogue of a language as JSON.
///
/// # Safety
/// `request_json` must be a NUL-terminated UTF-8 string (or null).
#[no_mangle]
pub unsafe extern "C" fn rc_i18n_bundle(request_json: *const c_char) -> *mut c_char {
    i18n_call(request_json, crate::ffi::i18n_bundle_json)
}

/// Catalogue health report as JSON (missing keys, placeholder drift, ...).
#[no_mangle]
pub extern "C" fn rc_i18n_diagnostics() -> *mut c_char {
    CString::new(crate::ffi::i18n_diagnostics_json().to_string())
        .unwrap_or_default()
        .into_raw()
}

/// Install / clear a runtime translation overlay.
///
/// # Safety
/// `request_json` must be a NUL-terminated UTF-8 string (or null).
#[no_mangle]
pub unsafe extern "C" fn rc_i18n_overlay(request_json: *const c_char) -> *mut c_char {
    i18n_call(request_json, crate::ffi::i18n_overlay_json)
}

/// Free a string returned by [`rc_run_async`]. Passing a null pointer is a
/// no-op.
///
/// # Safety
/// `s` must be a pointer returned by [`rc_run_async`] (or null).
#[no_mangle]
pub unsafe extern "C" fn rc_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    // A C callback that records every event JSON it receives.
    struct Log {
        count: AtomicUsize,
        last: Mutex<Option<String>>,
    }

    unsafe extern "C" fn sink(ptr: *const c_char, ud: *mut c_void) {
        let log = &*(ud as *const Log);
        if !ptr.is_null() {
            let s = CStr::from_ptr(ptr).to_str().unwrap_or("").to_string();
            log.count.fetch_add(1, Ordering::SeqCst);
            *log.last.lock().unwrap() = Some(s);
        }
    }

    #[test]
    fn c_api_subscribe_and_publish_roundtrip() {
        let _lock = crate::event::GLOBAL_BUS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        event::unsubscribe();
        let log = Log {
            count: AtomicUsize::new(0),
            last: Mutex::new(None),
        };
        unsafe {
            rc_event_bus_subscribe(sink, (&log as *const Log) as *mut c_void);
            assert_eq!(rc_event_bus_has_sink(), 1);
            let raw = event::Event::error("c", "boom").to_json();
            let c = CString::new(raw).unwrap();
            assert_eq!(rc_event_bus_publish(c.as_ptr()), 1);
        }
        assert_eq!(log.count.load(Ordering::SeqCst), 1);
        let last = log.last.lock().unwrap();
        let v: serde_json::Value = serde_json::from_str(last.as_ref().unwrap()).unwrap();
        assert_eq!(v["kind"], "error");
        assert_eq!(v["message"], "boom");
        assert_eq!(v["scope"], "c");
        rc_event_bus_unsubscribe();
        assert_eq!(rc_event_bus_has_sink(), 0);
    }

    #[test]
    fn c_api_run_async_returns_scope_json() {
        let _lock = crate::event::GLOBAL_BUS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        event::unsubscribe();
        let log = Log {
            count: AtomicUsize::new(0),
            last: Mutex::new(None),
        };
        unsafe {
            rc_event_bus_subscribe(sink, (&log as *const Log) as *mut c_void);
            let spec = CString::new("{\"scope\":\"c1\",\"label\":\"X\",\"steps\":1}").unwrap();
            let out = rc_run_async(spec.as_ptr());
            assert!(!out.is_null());
            let s = CStr::from_ptr(out).to_str().unwrap().to_string();
            assert!(s.contains("\"ok\":true"));
            assert!(s.contains("c1"));
            rc_string_free(out);
            std::thread::sleep(std::time::Duration::from_millis(200));
            rc_event_bus_unsubscribe();
        }
        assert!(log.count.load(Ordering::SeqCst) >= 1);
    }

    // --- i18n C surface (task 20) ---------------------------------------

    /// Read + free a `*mut c_char` returned by the C API.
    unsafe fn take(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null());
        let s = CStr::from_ptr(ptr).to_str().unwrap().to_string();
        rc_string_free(ptr);
        s
    }

    #[test]
    fn c_api_i18n_lists_and_translates() {
        let _g = crate::i18n::GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            let langs: serde_json::Value =
                serde_json::from_str(&take(rc_i18n_languages())).unwrap();
            assert_eq!(langs["base"], "zh-CN");
            assert_eq!(langs["languages"].as_array().unwrap().len(), 3);

            let req = CString::new(r#"{"key":"nav.home","language":"en"}"#).unwrap();
            let out: serde_json::Value =
                serde_json::from_str(&take(rc_i18n_translate(req.as_ptr()))).unwrap();
            assert_eq!(out["value"], "Home");

            // Null / malformed input must not crash and must not translate.
            let out: serde_json::Value =
                serde_json::from_str(&take(rc_i18n_translate(std::ptr::null()))).unwrap();
            assert_eq!(out["missing"], true);
            let bad = CString::new("{not json").unwrap();
            let out: serde_json::Value =
                serde_json::from_str(&take(rc_i18n_translate(bad.as_ptr()))).unwrap();
            assert_eq!(out["missing"], true);

            let diag: serde_json::Value =
                serde_json::from_str(&take(rc_i18n_diagnostics())).unwrap();
            assert_eq!(diag["base"], "zh-CN");
        }
    }

    #[test]
    fn c_api_i18n_switches_language_and_overlays() {
        let _g = crate::i18n::GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let restore = crate::i18n::current_language();
        unsafe {
            let req = CString::new(r#"{"tag":"zh-Hant"}"#).unwrap();
            let out: serde_json::Value =
                serde_json::from_str(&take(rc_i18n_set_language(req.as_ptr()))).unwrap();
            assert_eq!(out["tag"], "zh-Hant");
            assert_eq!(take(rc_i18n_current_language()), "zh-Hant");

            let b = CString::new(r#"{"language":"zh-Hant"}"#).unwrap();
            let bundle: serde_json::Value =
                serde_json::from_str(&take(rc_i18n_bundle(b.as_ptr()))).unwrap();
            assert_eq!(bundle["messages"]["nav.home"], "主頁");

            let ov = CString::new(r#"{"action":"install","language":"en","text":"nav.home = C"}"#)
                .unwrap();
            let out: serde_json::Value =
                serde_json::from_str(&take(rc_i18n_overlay(ov.as_ptr()))).unwrap();
            assert_eq!(out["installed"], 1);
            let clear = CString::new(r#"{"action":"clear"}"#).unwrap();
            let out: serde_json::Value =
                serde_json::from_str(&take(rc_i18n_overlay(clear.as_ptr()))).unwrap();
            assert_eq!(out["overlay_active"], false);

            // Freeing a null pointer is a documented no-op.
            rc_string_free(std::ptr::null_mut());
        }
        crate::i18n::set_language(restore);
    }
}
