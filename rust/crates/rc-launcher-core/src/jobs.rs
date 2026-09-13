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

use crate::download::{DownloadManager, DownloadOptions, DownloadTask, HttpSource};
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
    // Task 10: real download jobs are dispatched to the download manager
    // (task 2) and reported through the same event bus as the demo jobs
    // below, so the FFI/JNI async-callback surface is no longer demo-only.
    let kind = spec
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("demo")
        .to_string();
    if kind == "download" {
        return spawn_download_job(spec, None);
    }

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
                    event::publish(Event::error_with_code(
                        &scope_moved,
                        "job_failed",
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
        event::publish(Event::lifecycle_with_result(
            &scope_moved,
            "completed",
            format!("{label} completed"),
            json!({ "steps": steps }),
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

/// Spawn a real **download** job (task 2 ⇄ task 10 integration): a batch of
/// [`DownloadTask`]s pulled from `spec.tasks` is driven by the resumable
/// [`DownloadManager`]. Progress, logs and lifecycle are streamed to the event
/// bus, so the caller (Kotlin/Compose or a C consumer) learns the outcome
/// exclusively through the bus — never by blocking on a return value. This is the
/// concrete "async callback + progress event bus" half of task 10.
///
/// `spec.tasks[i]` = `{ "url", "dest", "size"?, "sha1"?, "md5"?, "mirrors"? }`.
/// `source` lets tests inject an in-memory backend; pass `None` at runtime to
/// build the default `reqwest` backend (which inherits the task-3 mirror / DoH /
/// proxy stack through [`crate::net`]).
pub fn spawn_download_job(spec: &Value, source: Option<Arc<dyn HttpSource>>) -> RcResult<Value> {
    let scope = spec
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("download")
        .to_string();
    let label = spec
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("download")
        .to_string();
    if scope.is_empty() {
        return Err(RcError::Other("job scope must not be empty".into()));
    }

    let tasks_val = spec
        .get("tasks")
        .and_then(|v| v.as_array())
        .ok_or_else(|| RcError::Other("download job requires a 'tasks' array".into()))?;
    if tasks_val.is_empty() {
        return Err(RcError::Other(
            "download job 'tasks' must not be empty".into(),
        ));
    }

    let mut tasks = Vec::with_capacity(tasks_val.len());
    for t in tasks_val {
        let url = t
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RcError::Other("download task missing 'url'".into()))?;
        let dest = t
            .get("dest")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RcError::Other("download task missing 'dest'".into()))?;
        let mut dt = DownloadTask::new(url, dest);
        if let Some(s) = t.get("size").and_then(|v| v.as_u64()) {
            dt = dt.with_size(s);
        }
        if let Some(s) = t.get("sha1").and_then(|v| v.as_str()) {
            dt = dt.with_sha1(s);
        }
        if let Some(s) = t.get("md5").and_then(|v| v.as_str()) {
            dt = dt.with_md5(s);
        }
        if let Some(arr) = t.get("mirrors").and_then(|v| v.as_array()) {
            for m in arr {
                if let Some(s) = m.as_str() {
                    dt = dt.with_mirror(s);
                }
            }
        }
        tasks.push(dt);
    }

    let mut opts = DownloadOptions::default();
    if let Some(c) = spec.get("concurrency").and_then(|v| v.as_u64()) {
        opts.max_batch_concurrency = c.max(1) as usize;
    }
    // Task 30: enhanced download options for large modpack / resource-pack
    // downloads on weak China-mainland networks.
    if let Some(chunk) = spec.get("chunk_concurrency").and_then(|v| v.as_u64()) {
        opts.concurrency = chunk.max(1) as usize;
    }
    if let Some(bs) = spec.get("chunk_size").and_then(|v| v.as_u64()) {
        opts.chunk_size = bs.max(1024);
    }
    if let Some(max_retries) = spec.get("max_retries").and_then(|v| v.as_u64()) {
        opts.max_retries = max_retries as u32;
    }
    if let Some(rate) = spec.get("max_bytes_per_second").and_then(|v| v.as_u64()) {
        opts.max_bytes_per_second = Some(rate);
    }
    if let Some(retries) = spec.get("max_checksum_retries").and_then(|v| v.as_u64()) {
        opts.max_checksum_retries = retries as u32;
    }
    if let Some(seq) = spec.get("sequential") {
        if seq.as_bool().unwrap_or(false) {
            opts.sequential = true;
        }
    }

    let progress_scope = scope.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    cancels()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(scope.clone(), cancel.clone());
    let _guard = ScopeGuard {
        scope: scope.clone(),
    };
    let mgr = match source {
        Some(s) => DownloadManager::new(s, opts),
        None => DownloadManager::with_default_source(opts)?,
    }
    .with_progress(Arc::new(move |p: &crate::download::Progress| {
        let status_str = match p.status {
            crate::download::DownloadStatus::Running => "running",
            crate::download::DownloadStatus::Paused => "paused",
            crate::download::DownloadStatus::Completed => "completed",
            crate::download::DownloadStatus::Failed => "failed",
        };
        event::publish_progress_with_meta(
            &progress_scope, &p.id, p.downloaded, p.total, p.speed_bps, status_str,
        );
    }))
    // Wire the existing job-level cancel flag into the download manager so
    // [cancel_job] / `cancelAsync` pauses the download mid-flight (task 30).
    .with_cancel(cancel);

    event::publish(Event::lifecycle(
        &scope,
        "started",
        format!("{label} started ({} task(s))", tasks.len()),
    ));

    let scope_moved = scope.clone();
    let label_moved = label.clone();
    job_runtime().spawn(async move {
        let _guard = _guard;
        let summaries = mgr.download_all_concurrent(&tasks).await;
        let mut succeeded = 0u64;
        let mut failures: Vec<String> = Vec::new();
        for (i, r) in summaries.iter().enumerate() {
            match r {
                Ok(s) => {
                    succeeded += 1;
                    event::publish(Event::lifecycle_with_result(
                        &scope_moved,
                        "task_completed",
                        format!("{label_moved} task {i} done"),
                        serde_json::json!({
                            "dest": s.dest.to_string_lossy().to_string(),
                            "size": s.size,
                            "resumed": s.resumed,
                        }),
                    ));
                }
                Err(e) => failures.push(e.to_string()),
            }
        }
        if !failures.is_empty() {
            event::publish(Event::error_with_code(
                &scope_moved,
                "download_failed",
                format!(
                    "{label_moved} failed: {} task(s) failed ({})",
                    failures.len(),
                    failures.join("; ")
                ),
            ));
        } else {
            event::publish(Event::lifecycle_with_result(
                &scope_moved,
                "completed",
                format!("{label_moved} completed"),
                serde_json::json!({ "succeeded": succeeded, "tasks": tasks.len() }),
            ));
        }
    });

    Ok(json!({ "ok": true, "scope": scope }))
}
/// Spawn a fire-and-forget **modpack import** job (task 17). The actual
/// work runs on the shared [`job_runtime`] and reports progress + lifecycle
/// + error events through the [event::EventBus]. Returns
/// `{"ok": true, "scope": "modpack-..."}` so the UI can subscribe to
/// progress events by scope and call [`cancel_job`] to abort.
pub fn spawn_modpack_import_job(
    importer: crate::mods::modpack::ModpackImporter,
    manifest: crate::mods::modpack::Manifest,
    instance_id: String,
    allow_overwrite: bool,
) -> Value {
    // Each job gets a unique scope so multiple imports can run in parallel.
    let scope = format!(
        "modpack-{}-{}",
        instance_id,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let label = format!("import {}", manifest.spec().name);
    let cancel = Arc::new(AtomicBool::new(false));
    cancels()
        .lock()
        .map(|mut g| g.insert(scope.clone(), cancel.clone()))
        .ok();

    let scope_for_task = scope.clone();
    job_runtime().spawn(async move {
        let _guard = ScopeGuard {
            scope: scope_for_task.clone(),
        };
        // Lifecycle: started.
        let _ = event::publish(Event::lifecycle(
            &scope_for_task,
            "started",
            format!("{} ({})", label, scope_for_task),
        ));

        let report = if allow_overwrite {
            importer
                .import_allowing_overwrite(&manifest, &instance_id)
                .await
        } else {
            importer.import(&manifest, &instance_id).await
        };
        match report {
            Ok(r) => {
                let phase = if r.is_clean() {
                    "succeeded"
                } else {
                    "completed_with_errors"
                };
                let _ = event::publish(Event::lifecycle_with_result(
                    &scope_for_task,
                    phase,
                    format!("imported {} files", r.downloaded_count),
                    serde_json::to_value(&r)
                        .unwrap_or_else(|e| json!({ "serialise_error": e.to_string() })),
                ));
            }
            Err(e) => {
                let _ = event::publish(Event::error(&scope_for_task, e.to_string()));
            }
        }

        // Lifecycle: finished.
        let _ = event::publish(Event::lifecycle(
            &scope_for_task,
            "finished",
            "modpack import finished",
        ));
    });

    json!({ "ok": true, "scope": scope })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::{FetchResult, HttpSource};
    use crate::event::{EventKind, EventSink};
    use async_trait::async_trait;
    use std::collections::HashMap;
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

    /// In-memory `HttpSource` so the download job can be exercised without any
    /// network (task 10 integration with task 2 end-to-end on the host).
    struct MemSource {
        files: HashMap<String, Vec<u8>>,
    }

    #[async_trait]
    impl HttpSource for MemSource {
        async fn fetch_range(
            &self,
            url: &str,
            start: u64,
            end: Option<u64>,
        ) -> RcResult<FetchResult> {
            let data = self
                .files
                .get(url)
                .ok_or_else(|| RcError::Download(format!("mem source: unknown url {url}")))?;
            let last = data.len().saturating_sub(1) as u64;
            let end = end.unwrap_or(last).min(last);
            let s = start as usize;
            let e = end as usize;
            if s > e {
                return Err(RcError::Download(format!(
                    "mem source: bad range {start}..={end}"
                )));
            }
            Ok(FetchResult {
                bytes: data[s..=e].to_vec(),
                total_size: data.len() as u64,
                supports_range: true,
            })
        }
    }

    #[test]
    fn download_job_streams_progress_and_completes() {
        let _lock = crate::event::GLOBAL_BUS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        event::unsubscribe();
        let c = Arc::new(Collector::default());
        event::subscribe(c.clone());

        let tmp = std::env::temp_dir().join("rc_dl_test_a.bin");
        let content: Vec<u8> = (0u8..=255).cycle().take(200_000).collect();
        let mut files = HashMap::new();
        files.insert("http://example.com/a.bin".to_string(), content);
        let source = std::sync::Arc::new(MemSource { files });

        let spec = json!({
            "type": "download",
            "scope": "dl1",
            "label": "Fetch",
            "tasks": [ { "url": "http://example.com/a.bin", "dest": tmp.to_string_lossy().to_string(), "size": 200000u64 } ]
        });
        let out = spawn_download_job(&spec, Some(source)).unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(out["scope"], "dl1");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut got_completed = false;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(20));
            let g = c.events.lock().unwrap();
            if g.iter().any(|e| {
                e.kind == EventKind::Lifecycle
                    && e.data
                        .as_ref()
                        .map(|d| d["phase"] == "completed")
                        .unwrap_or(false)
            }) {
                got_completed = true;
                break;
            }
            if g.iter().any(|e| e.kind == EventKind::Error) {
                break;
            }
            drop(g);
            if deadline.elapsed() > std::time::Duration::from_secs(10) {
                break;
            }
        }
        assert!(
            got_completed,
            "download job should emit lifecycle:completed"
        );
        let g = c.events.lock().unwrap();
        let progress_count = g.iter().filter(|e| e.kind == EventKind::Progress).count();
        assert!(
            progress_count > 0,
            "download job should emit progress events"
        );
        let completed = g
            .iter()
            .find(|e| {
                e.kind == EventKind::Lifecycle
                    && e.data
                        .as_ref()
                        .map(|d| d["phase"] == "completed")
                        .unwrap_or(false)
            })
            .unwrap();
        assert_eq!(completed.data.as_ref().unwrap()["result"]["succeeded"], 1);
        event::unsubscribe();
    }

    #[test]
    fn download_job_reports_error_with_code_on_failure() {
        let _lock = crate::event::GLOBAL_BUS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        event::unsubscribe();
        let c = Arc::new(Collector::default());
        event::subscribe(c.clone());

        let tmp = std::env::temp_dir().join("rc_dl_test_b.bin");
        // Empty source -> every fetch hits an unknown url -> the job must fail
        // with a structured `download_failed` code (not a silent drop).
        let source = std::sync::Arc::new(MemSource {
            files: HashMap::new(),
        });
        let spec = json!({
            "type": "download",
            "scope": "dl2",
            "label": "Fetch",
            "tasks": [ { "url": "http://example.com/missing.bin", "dest": tmp.to_string_lossy().to_string() } ]
        });
        let _ = spawn_download_job(&spec, Some(source)).unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut got_error = false;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(20));
            let g = c.events.lock().unwrap();
            if let Some(e) = g.iter().find(|e| e.kind == EventKind::Error) {
                assert_eq!(e.code.as_deref(), Some("download_failed"));
                got_error = true;
                break;
            }
            drop(g);
            if deadline.elapsed() > std::time::Duration::from_secs(10) {
                break;
            }
        }
        assert!(got_error, "download job should report an error with a code");
        event::unsubscribe();
    }
}
