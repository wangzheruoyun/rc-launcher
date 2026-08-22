# FFI / JNI bridge & event bus (task 10)

This document describes the Rust-core ⇄ Kotlin bridge for RC Launcher. It is
the layer that exposes the safe Rust core (`librc_launcher.so`) to the Compose
UI without ever blocking the UI thread.

## Layered design (mirrors MCTier)

```
 Rust core (tokio workers)                 Kotlin / Compose
 ┌───────────────────────────┐             ┌────────────────────────────┐
 │  subsystems: download/    │             │  RustBridge (object)       │
 │  launch/auth/runtime/... │             │   external fun ...         │
 └────────────┬──────────────┘             └────────────┬───────────────┘
              │  event::EventBus.publish(Event)          │ JNI call
              ▼                                           ▼
 ┌───────────────────────────┐   C-ABI    ┌────────────────────────────┐
 │  capi  (extern "C")        │◀─────────▶│  (future native consumers: │
 │  rc_event_bus_* /         │  libeasytier│   Unity/Unreal/CLI)        │
 │  rc_run_async             │  _ffi.so    └────────────────────────────┘
 └────────────┬──────────────┘
              │  event::EventSink
              ▼
 ┌───────────────────────────┐   JNI      ┌────────────────────────────┐
 │  ffi  (extern "system")    │◀─────────▶│  RcEventBus : RcEventSink   │
 │  eventBusSubscribe(...)    │  JNIEnv    │   onEvent(json: String)     │
 │  runAsync / cancelAsync    │            │     │                      │
 └───────────────────────────┘            │     ▼                      │
                                           │  RcEventListener (Flow-ish)│
                                           └────────────────────────────┘
```

* **`capi`** — a flat, `#[repr(C)]`-friendly C API (`extern "C"` functions with
  function-pointer callbacks). `cbindgen` turns it into `rc_launcher.h`, exactly
  like MCTier's `libeasytier_ffi.so`. This is the *portable* contract.
* **`ffi`** — the *thin* JNI wrapper (`extern "system"` `Java_com_rc_…`
  functions). It registers a Kotlin `RcEventSink` as the bus sink and forwards
  async jobs to `jobs`. All JNI entry points are wrapped in `catch_unwind` so a
  panic becomes a `null`/`false` return instead of an aborted VM.

## The event bus

`event::EventBus` is a process-wide singleton (one sink for the whole process,
like the real JVM sink). Every subsystem funnels its:

* **progress** — downloads, extraction, install (with `downloaded`/`total`/`fraction`)
* **log** — game-process stdout/stderr lines (with `level`)
* **lifecycle** — `started` / `completed` / `cancelled` of a job
* **error** — failures (delivered as events, not only thrown exceptions)
* **status** — free-form health pings

into one `Event { seq, kind, message, scope, data }`, serialised to a **single
JSON `String`**. On the Kotlin side `RcEventBus.onEvent(json)` decodes it once
into a typed `RcEvent`. Marshalling one `String` (not field-by-field) is the
"near zero-copy" path on the JNI boundary.

### Thread model & safety

* The sink is held behind `Arc<Mutex<Option<Arc<dyn EventSink>>>>`.
* Events are emitted **after** the lock is released, so a slow/blocking sink
  (or one that re-enters the bus) can never deadlock a publisher.
* A panic inside a sink is caught (`catch_unwind`) so a misbehaving subscriber
  cannot take down the core.
* The JNI sink (`JniEventSink`) stores a JNI `GlobalRef` to the Kotlin callback
  plus the `JavaVM`. Worker threads spawned by tokio have no attached
  `JNIEnv`, so `emit` calls `JavaVM::attach_current_thread()` (the EasyTier /
  MCTier attach-per-thread pattern) and the guard detaches on drop — no leaked
  thread attachments.

## Async callbacks (fire-and-forget)

`RustBridge.runAsync(specJson)` / `rc_run_async(spec)` start a background job on
the shared tokio runtime and return **immediately** with
`{"ok":true,"scope":<scope>}`. The job streams `lifecycle:started`, one
`progress` event per step, then either `lifecycle:completed` or `error`, or
`lifecycle:cancelled` when `RustBridge.cancelAsync(scope)` /
`rc_cancel_async(scope)` sets the per-scope cancellation flag. The UI learns the
outcome **exclusively through the event bus** — it is never blocked.

`specJson`:
```json
{ "scope": "dl-1.20.4", "label": "Download", "steps": 12,
  "fail_at": null, "delay_ms": 0 }
```

## Kotlin usage

```kotlin
class App : Application() {
    override fun onCreate() {
        super.onCreate()
        RcEventBus.connect()            // subscribe once
        RcEventBus.addListener { event ->
            when (event.kind) {
                RcEventKind.PROGRESS -> updateProgressBar(event.progressFraction ?: 0.0)
                RcEventKind.ERROR    -> showError(event.message)
                else -> Unit
            }
        }
        // fire-and-forget; progress arrives via the bus
        RustBridge.runAsync(RcJobSpec(scope = "dl", label = "Download", steps = 12).toJson())
    }
}
```

## Regenerating `rc_launcher.h`

The header is generated from the C-ABI module in single-file mode (so it
contains exactly the C contract and no JNI/serde internals):

```bash
cd rust/crates/rc-launcher-core
cbindgen --config cbindgen.toml src/capi.rs -o rc_launcher.h -l C
```
