//! Shared async job runner reporting through the event bus (task 10).
//!
//! Both the JNI bridge ([`crate::ffi`]) and the C-ABI ([`crate::capi`]) delegate
//! here, so there is exactly one implementation of "fire-and-forget job that
//! streams progress + lifecycle + error events to the bus". Keeping the job
//! logic out of the FFI wrappers also makes it unit-testable without a JVM.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};

use crate::error::{RcError, RcResult};
use crate::event::{self, Event};

/// Multi-threaded tokio runtime shared by every async FFI entry point. A single
/// long-lived runtime (vs. a new one per call) keeps latency low and lets the
/// spawned jobs outlive the JNI/C call that started them.
pub(crate) fn job_runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("init job runtime")
    })
}

/// Per-scope cancellation flags. Set by [`cancel_job`], polled by running jobs.
static CANCELS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn cancels() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    CANCELS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Drops the cancellation flag for `scope` when the owning job finishes, so the
/// registry cannot grow without bound over a long session.
struct ScopeGuard {
    scope: String,
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        if let Ok(mut g) = cancels().lock() {
            g.remove(&self.scope);
        }
    }
}

/// Spawn a background job described by `spec` (the same JSON accepted by the
/// JNI `runAsync` and the C `rc_run_async`):
///
/// ```json
/// { "scope": "job", "label": "Download", "steps": 5,
///   "fail_at": null, "delay_ms": 0 }
/// ```
///
/// Emits `lifecycle:started`, a `progress` event per step, then either
/// `lifecycle:completed` or `error` (when `fail_at` triggers) or
/// `lifecycle:cancelled` (when [`cancel_job`] is called). Returns
/// `{ "ok": true, "scope": <scope> }`.
pub fn spawn_job(spec: &Value) -> RcResult<Value> {
    let scope = spec
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("job")
        .to_string();
    let label = spec
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("job")
        .to_string();
    let steps = spec
        .get("steps")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1);
    let fail_at = spec.get("fail_at").and_then(|v| v.as_u64());
    let delay_ms = spec.get("delay_ms").and_then(|v| v.as_u64()).unwrap_or(0);

    if scope.is_empty() {
        return Err(RcError::Other("job scope must not be empty".into()));
    }

    // Register a cancellation flag for this scope.
    let cancel = Arc::new(AtomicBool::new(false));
    cancels()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(scope.clone(), cancel.clone());

    let guard = ScopeGuard {
        scope: scope.clone(),
    };

    event::publish(Event::lifecycle(
        &scope,
        "started",
        format!("{label} started"),
    ));

    let scope_moved = scope.clone();
    job_runtime().spawn(async move {
        // `guard` is moved in; it removes the registry entry when the job ends
        // (on every return path below).
        let _guard = guard;

        for i in 1..=steps {
            if cancel.load(Ordering::SeqCst) {
                event::publish(Event::lifecycle(
                    &scope_moved,
                    "cancelled",
                    format!("{label} cancelled"),
                ));
                return;
            }
            if let Some(f) = fail_at {
                if i == f {
                    event::publish(Event::error(
                        &scope_moved,
                        format!("{label} failed at step {i}"),
                    ));
                    return;
                }
            }
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            event::publish(Event::progress(
                &scope_moved,
                format!("{label} step {i}"),
                i,
                Some(steps),
            ));
        }
        event::publish(Event::lifecycle(
            &scope_moved,
            "completed",
            format!("{label} completed"),
        ));
    });

    Ok(json!({ "ok": true, "scope": scope }))
}

/// Signal cancellation for a running job by scope. Returns `true` if a matching
/// job was found (and will observe the flag on its next poll).
pub fn cancel_job(scope: &str) -> bool {
    let g = cancels().lock().unwrap_or_else(|e| e.into_inner());
    match g.get(scope) {
        Some(flag) => {
            flag.store(true, Ordering::SeqCst);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventKind, EventSink};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration as StdDuration;

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

    fn drain(c: &Arc<Collector>) -> Vec<Event> {
        // Give the tokio worker time to run the job to completion.
        std::thread::sleep(StdDuration::from_millis(300));
        c.events.lock().unwrap().clone()
    }

    #[test]
    fn job_emits_started_progress_completed() {
        let _lock = crate::event::GLOBAL_BUS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        event::unsubscribe();
        let c = Arc::new(Collector::default());
        event::subscribe(c.clone());

        let spec = json!({ "scope": "j1", "label": "Work", "steps": 3 });
        let out = spawn_job(&spec).unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(out["scope"], "j1");

        let events = drain(&c);
        assert!(events.iter().any(|e| e.kind == EventKind::Lifecycle
            && e.data
                .as_ref()
                .map(|d| d["phase"] == "started")
                .unwrap_or(false)));
        let progress: Vec<&Event> = events
            .iter()
            .filter(|e| e.kind == EventKind::Progress)
            .collect();
        assert_eq!(progress.len(), 3);
        assert_eq!(
            progress.last().unwrap().data.as_ref().unwrap()["fraction"],
            1.0
        );
        assert!(events.iter().any(|e| e.kind == EventKind::Lifecycle
            && e.data
                .as_ref()
                .map(|d| d["phase"] == "completed")
                .unwrap_or(false)));
        event::unsubscribe();
    }

    #[test]
    fn job_emits_error_on_fail_at() {
        let _lock = crate::event::GLOBAL_BUS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        event::unsubscribe();
        let c = Arc::new(Collector::default());
        event::subscribe(c.clone());
        let spec = json!({ "scope": "j2", "label": "Boom", "steps": 5, "fail_at": 2 });
        let _ = spawn_job(&spec).unwrap();
        let events = drain(&c);
        assert!(events.iter().any(|e| e.kind == EventKind::Error));
        assert!(!events.iter().any(|e| e.kind == EventKind::Lifecycle
            && e.data
                .as_ref()
                .map(|d| d["phase"] == "completed")
                .unwrap_or(false)));
        event::unsubscribe();
    }

    #[test]
    fn cancel_stops_the_job() {
        let _lock = crate::event::GLOBAL_BUS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        event::unsubscribe();
        let c = Arc::new(Collector::default());
        event::subscribe(c.clone());
        let spec = json!({ "scope": "j3", "label": "Long", "steps": 50, "delay_ms": 5 });
        let _ = spawn_job(&spec).unwrap();
        // Cancel almost immediately.
        assert!(cancel_job("j3"));
        let events = drain(&c);
        assert!(events.iter().any(|e| e.kind == EventKind::Lifecycle
            && e.data
                .as_ref()
                .map(|d| d["phase"] == "cancelled")
                .unwrap_or(false)));
        event::unsubscribe();
    }

    #[test]
    fn cancel_unknown_scope_is_false() {
        let _lock = crate::event::GLOBAL_BUS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        event::unsubscribe();
        assert!(!cancel_job("never-existed"));
    }
}
