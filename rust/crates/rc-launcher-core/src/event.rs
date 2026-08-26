//! Cross-language event bus for the FFI / JNI bridge (task 10).
//!
//! The Rust core is a native library loaded into the Android JVM. Long-running
//! work (downloads, game launch, authentication, …) must never block the
//! calling thread, so it runs on Rust worker threads and pushes *structured
//! events* back to Kotlin through an [`EventSink`].
//!
//! Kotlin subscribes exactly once via `RustBridge.eventBusSubscribe(...)` and
//! then receives a stream of JSON events on a background thread. This mirrors
//! the MCTier bridge, where the Rust core (`libeasytier_ffi.so`) exposes a C
//! ABI that a thin JNI wrapper (`libeasytier_android_jni.so` /
//! `EasyTierJNI.kt`) forwards to Kotlin — **Rust core → C-ABI FFI → JNI
//! wrapper → Kotlin**. We follow the same two-layer shape: a C-ABI surface
//! (`capi`, consumed by `cbindgen` to produce `rc_launcher.h`) plus the JNI
//! functions in `ffi`.
//!
//! Design goals (task 10):
//! * **Async callbacks** — jobs are started fire-and-forget (`runAsync`) and
//!   report exclusively through the bus; the caller is never blocked.
//! * **Progress events (event bus)** — every subsystem funnels its progress,
//!   logs, lifecycle and errors into the single bus.
//! * **Error passthrough** — failures are delivered as `error` events (not only
//!   as thrown exceptions) so the UI can render them without blocking.
//! * **Thread safety & zero-copy** — the sink is held behind `Arc<Mutex<…>>`;
//!   events are emitted *after* releasing the lock so a slow/blocking sink (or
//!   one that re-enters the bus) cannot deadlock a publisher; each event is a
//!   flat, JSON-serialisable struct handed across JNI as a single `String`, so
//!   the Kotlin side decodes it once into a typed `RcEvent` (no field-by-field
//!   marshalling).

use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Logical category of an event. Mirrors the FCL `event` package taxonomy so
/// the Kotlin side can pattern-match on a closed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// Progress update (download / extraction / install).
    Progress,
    /// A log line emitted by the supervised game process or the core itself.
    Log,
    /// Lifecycle transition (`started` / `completed` / `cancelled` / …).
    Lifecycle,
    /// An error on the Rust side. Delivered as an event (not just a thrown
    /// exception) so the bus is the single channel for both success and
    /// failure.
    Error,
    /// A free-form status / health ping.
    Status,
}

impl EventKind {
    /// Stable string used in the JSON `kind` field (snake_case).
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::Progress => "progress",
            EventKind::Log => "log",
            EventKind::Lifecycle => "lifecycle",
            EventKind::Error => "error",
            EventKind::Status => "status",
        }
    }
}

/// A single event that crosses the FFI boundary.
///
/// It is intentionally a flat, JSON-serialisable struct so it can be passed
/// across JNI as one `String` (no field-by-field marshalling → near zero-copy
/// on the Kotlin side, where it is decoded once into a `RcEvent`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Monotonic sequence number assigned by the bus (per sink lifetime).
    pub seq: u64,
    /// Event category.
    pub kind: EventKind,
    /// Human readable message (already localised where possible on the Rust
    /// side; the Kotlin side may localise further via `kind`).
    pub message: String,
    /// Correlation id so the UI can match events to a started job (a download
    /// batch, a launch, …). Defaults to `"global"`.
    #[serde(default = "default_scope")]
    pub scope: String,
    /// Optional structured payload (progress fractions, log level, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// Optional machine-readable error code (only meaningful for `error`
    /// events and terminal `lifecycle` events). Lets the Kotlin/Compose
    /// side branch on *what* failed without string-matching the
    /// human-readable `message` (task 10: structured error passthrough
    /// across the JNI/C boundary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

fn default_scope() -> String {
    "global".to_string()
}

impl Event {
    /// Build a `progress` event. `total` may be `None` when the size is not yet
    /// known; the `fraction` field is then omitted.
    pub fn progress(
        scope: &str,
        message: impl Into<String>,
        downloaded: u64,
        total: Option<u64>,
    ) -> Self {
        let fraction = total
            .filter(|t| *t > 0)
            .map(|t| downloaded as f64 / t as f64);
        let data = json!({
            "downloaded": downloaded,
            "total": total,
            "fraction": fraction,
        });
        Event {
            seq: 0,
            kind: EventKind::Progress,
            message: message.into(),
            scope: scope.to_string(),
            code: None,
            data: Some(data),
        }
    }

    /// Build a `log` event with a `level` field in the payload.
    pub fn log(scope: &str, level: &str, line: impl Into<String>) -> Self {
        Event {
            seq: 0,
            kind: EventKind::Log,
            message: line.into(),
            scope: scope.to_string(),
            code: None,
            data: Some(json!({ "level": level })),
        }
    }

    /// Build a `lifecycle` event with a `phase` field in the payload.
    pub fn lifecycle(scope: &str, phase: &str, message: impl Into<String>) -> Self {
        Event {
            seq: 0,
            kind: EventKind::Lifecycle,
            message: message.into(),
            scope: scope.to_string(),
            code: None,
            data: Some(json!({ "phase": phase })),
        }
    }

    /// Build an `error` event.
    pub fn error(scope: &str, message: impl Into<String>) -> Self {
        Event {
            seq: 0,
            kind: EventKind::Error,
            message: message.into(),
            scope: scope.to_string(),
            code: None,
            data: None,
        }
    }

    /// Build a `status` event with an arbitrary JSON payload.
    pub fn status(scope: &str, message: impl Into<String>, data: Value) -> Self {
        Event {
            seq: 0,
            kind: EventKind::Status,
            message: message.into(),
            scope: scope.to_string(),
            code: None,
            data: Some(data),
        }
    }

    /// Build an `error` event that also carries a machine-readable `code`
    /// (e.g. `"download_failed"`, `"io_error"`). The Kotlin/Compose side can
    /// branch on `code` instead of parsing `message` (task 10: structured error
    /// passthrough across the JNI/C boundary).
    pub fn error_with_code(
        scope: &str,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Event {
            seq: 0,
            kind: EventKind::Error,
            message: message.into(),
            scope: scope.to_string(),
            code: Some(code.into()),
            data: None,
        }
    }

    /// Build a `lifecycle` event whose payload carries a structured `result`
    /// (e.g. a download summary). Async jobs report their outcome exclusively
    /// through the bus, so the result travels here rather than as a return value
    /// (task 10: async callback + progress event bus).
    pub fn lifecycle_with_result(
        scope: &str,
        phase: &str,
        message: impl Into<String>,
        result: Value,
    ) -> Self {
        Event {
            seq: 0,
            kind: EventKind::Lifecycle,
            message: message.into(),
            scope: scope.to_string(),
            code: None,
            data: Some(json!({ "phase": phase, "result": result })),
        }
    }

    /// Serialise to the compact JSON string handed to the JNI/Kotlin side.
    ///
    /// `unwrap` is safe: `Event` only contains `String`/`u64`/`EventKind`/
    /// `Option<Value>` — all of which serialise infallibly.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("event serialisation")
    }
}

/// Trait implemented by whatever delivers events to the outside world.
///
/// The JNI layer implements it by calling back into Kotlin; unit tests
/// implement it with an in-memory collector. Hiding the sink behind a trait
/// object keeps the bus fully testable without a JVM.
pub trait EventSink: Send + Sync {
    /// Deliver one event. Implementations must be non-blocking and must never
    /// panic across the FFI boundary (the JNI impl swallows errors and the bus
    /// itself catches them).
    fn emit(&self, event: &Event);
}

/// A self-contained event bus.
///
/// The FFI layer uses the process-wide singleton returned by [`event_bus`];
/// tests instantiate their own `EventBus` so they never touch the global state.
pub struct EventBus {
    sink: Mutex<Option<Arc<dyn EventSink>>>,
    seq: Mutex<u64>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    /// Create an empty bus with no subscriber.
    pub fn new() -> Self {
        EventBus {
            sink: Mutex::new(None),
            seq: Mutex::new(0),
        }
    }

    /// Subscribe a sink. Replaces any previously registered sink. Returns `true`
    /// if a sink was already present (i.e. this call replaced it).
    pub fn subscribe(&self, sink: Arc<dyn EventSink>) -> bool {
        let mut g = self.sink.lock().unwrap_or_else(|e| e.into_inner());
        let replaced = g.is_some();
        *g = Some(sink);
        replaced
    }

    /// Remove the current sink. Subsequent [`publish`](Self::publish) calls
    /// become no-ops.
    pub fn unsubscribe(&self) {
        *self.sink.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Whether a sink is currently subscribed.
    pub fn has_sink(&self) -> bool {
        self.sink
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    /// Publish an event. Assigns the next sequence number, then delivers it to
    /// the sink **without** holding the sink lock (so a slow/blocking sink — or
    /// one that re-enters the bus — cannot deadlock the publisher). A panic in
    /// the sink is caught so a misbehaving subscriber can never take down the
    /// core (task 19).
    pub fn publish(&self, mut event: Event) {
        let sink = {
            let mut seq = self.seq.lock().unwrap_or_else(|e| e.into_inner());
            event.seq = *seq;
            *seq += 1;
            self.sink.lock().unwrap_or_else(|e| e.into_inner()).clone()
        };
        if let Some(sink) = sink {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink.emit(&event)));
        }
    }

    /// Publish a pre-serialised JSON event (used by the FFI boundary so the
    /// Kotlin/C side can inject events too, e.g. to replay a log). Returns
    /// `false` if `json_str` could not be parsed into an [`Event`].
    pub fn publish_json(&self, json_str: &str) -> bool {
        match serde_json::from_str::<Event>(json_str) {
            Ok(e) => {
                self.publish(e);
                true
            }
            Err(_) => false,
        }
    }

    /// Convenience: build and publish a `progress` event.
    pub fn publish_progress(
        &self,
        scope: &str,
        message: impl Into<String>,
        downloaded: u64,
        total: Option<u64>,
    ) {
        self.publish(Event::progress(scope, message, downloaded, total));
    }

    /// Convenience: build and publish a `log` event.
    pub fn publish_log(&self, scope: &str, level: &str, line: impl Into<String>) {
        self.publish(Event::log(scope, level, line));
    }

    /// Convenience: build and publish a `lifecycle` event.
    pub fn publish_lifecycle(&self, scope: &str, phase: &str, message: impl Into<String>) {
        self.publish(Event::lifecycle(scope, phase, message));
    }

    /// Convenience: build and publish an `error` event.
    pub fn publish_error(&self, scope: &str, message: impl Into<String>) {
        self.publish(Event::error(scope, message));
    }
}

/// Process-wide event bus used by the FFI/JNI and C-ABI layers. There is a
/// single sink for the whole process (the Kotlin side subscribes once), so the
/// free functions below delegate to this singleton.
static BUS: OnceLock<EventBus> = OnceLock::new();

/// Obtain the process-wide [`EventBus`].
pub fn event_bus() -> &'static EventBus {
    BUS.get_or_init(EventBus::new)
}

/// Subscribe a sink on the global bus. See [`EventBus::subscribe`].
pub fn subscribe(sink: Arc<dyn EventSink>) -> bool {
    event_bus().subscribe(sink)
}

/// Remove the current sink on the global bus. See [`EventBus::unsubscribe`].
pub fn unsubscribe() {
    event_bus().unsubscribe();
}

/// Whether a sink is currently subscribed on the global bus.
pub fn has_sink() -> bool {
    event_bus().has_sink()
}

/// Publish an event on the global bus. See [`EventBus::publish`].
pub fn publish(event: Event) {
    event_bus().publish(event);
}

/// Publish a pre-serialised JSON event on the global bus. See
/// [`EventBus::publish_json`].
pub fn publish_json(json_str: &str) -> bool {
    event_bus().publish_json(json_str)
}

/// Convenience: build and publish a `progress` event on the global bus.
pub fn publish_progress(
    scope: &str,
    message: impl Into<String>,
    downloaded: u64,
    total: Option<u64>,
) {
    event_bus().publish_progress(scope, message, downloaded, total);
}

/// Convenience: build and publish a `log` event on the global bus.
pub fn publish_log(scope: &str, level: &str, line: impl Into<String>) {
    event_bus().publish_log(scope, level, line);
}

/// Convenience: build and publish a `lifecycle` event on the global bus.
pub fn publish_lifecycle(scope: &str, phase: &str, message: impl Into<String>) {
    event_bus().publish_lifecycle(scope, phase, message);
}

/// Convenience: build and publish an `error` event on the global bus.
pub fn publish_error(scope: &str, message: impl Into<String>) {
    event_bus().publish_error(scope, message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// In-memory sink used by the unit tests.
    #[derive(Default)]
    struct Collector {
        events: Mutex<Vec<Event>>,
        count: AtomicUsize,
    }

    impl EventSink for Collector {
        fn emit(&self, e: &Event) {
            self.count.fetch_add(1, Ordering::SeqCst);
            self.events.lock().unwrap().push(e.clone());
        }
    }

    #[test]
    fn local_bus_is_isolated_from_global() {
        // A local bus never touches the global singleton, so concurrent tests
        // cannot interfere with each other.
        let bus = EventBus::new();
        let c = Arc::new(Collector::default());
        assert!(!bus.subscribe(c.clone()));
        assert!(bus.has_sink());

        bus.publish(Event::status("g", "hi", json!({ "k": 1 })));
        bus.publish(Event::progress("dl", "x", 5, Some(10)));

        let g = c.events.lock().unwrap();
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].seq, 0);
        assert_eq!(g[1].seq, 1);
        assert_eq!(g[1].kind, EventKind::Progress);
        assert_eq!(g[1].scope, "dl");
        assert_eq!(g[1].data.as_ref().unwrap()["fraction"], 0.5);
        drop(g);

        bus.unsubscribe();
        assert!(!bus.has_sink());
        bus.publish(Event::status("g", "lost", json!({})));
        assert_eq!(c.count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn replace_sink_reports_previous() {
        let bus = EventBus::new();
        let a = Arc::new(Collector::default());
        let b = Arc::new(Collector::default());
        assert!(!bus.subscribe(a.clone()));
        assert!(bus.subscribe(b.clone()));
    }

    #[test]
    fn publish_json_roundtrip_and_bad_input() {
        let bus = EventBus::new();
        let c = Arc::new(Collector::default());
        bus.subscribe(c.clone());
        let raw = Event::error("s", "boom").to_json();
        assert!(bus.publish_json(&raw));
        assert!(!bus.publish_json("not json at all"));
        let g = c.events.lock().unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].kind, EventKind::Error);
        assert_eq!(g[0].message, "boom");
    }

    #[test]
    fn panic_in_sink_does_not_propagate() {
        struct Boom;
        impl EventSink for Boom {
            fn emit(&self, _e: &Event) {
                panic!("sink panicked");
            }
        }
        let bus = EventBus::new();
        bus.subscribe(Arc::new(Boom));
        // Must not panic out of `publish`.
        bus.publish(Event::status("g", "x", json!({})));
        bus.unsubscribe();
    }

    #[test]
    fn structured_error_and_result_serialise_across_boundary() {
        let bus = EventBus::new();
        let c = Arc::new(Collector::default());
        bus.subscribe(c.clone());

        // Machine-readable code travels on the error event...
        bus.publish(Event::error_with_code("s", "download_failed", "boom"));
        // ...and the structured outcome travels on the lifecycle:completed event.
        bus.publish(Event::lifecycle_with_result(
            "s",
            "completed",
            "done",
            json!({ "succeeded": 1, "tasks": 1 }),
        ));

        let g = c.events.lock().unwrap();
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].kind, EventKind::Error);
        assert_eq!(g[0].code.as_deref(), Some("download_failed"));
        // The Kotlin/C side decodes `code` straight from the JSON string.
        let ej = g[0].to_json();
        assert!(ej.contains("code"));
        assert!(ej.contains("download_failed"));
        assert_eq!(g[1].kind, EventKind::Lifecycle);
        assert_eq!(g[1].data.as_ref().unwrap()["result"]["succeeded"], 1);
    }
}

/// Serialises every test that mutates the *global* bus, so the parallel test
/// runner cannot interleave subscriptions (the global bus is a singleton, like
/// the real JVM sink). Tests that use a local [`EventBus`] do not need it.
#[cfg(test)]
pub(crate) static GLOBAL_BUS_TEST_LOCK: Mutex<()> = Mutex::new(());
