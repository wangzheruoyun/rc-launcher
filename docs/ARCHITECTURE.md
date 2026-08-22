# Architecture

This document explains how **RC Launcher** (`com.rc.launcher`) is put together:
the module boundaries, the dependency direction, and how the Rust core talks to
the Jetpack Compose UI. It is the canonical reference for tasks 1–25 and the
companion to [`MODULES.md`](MODULES.md) (interface contracts),
[`BUILD.md`](BUILD.md) (how to build & release) and
[`CONTRIBUTING.md`](CONTRIBUTING.md) (how to contribute).

The design deliberately borrows from the reference snapshots in `snapshots/`
(see [`ANALYSIS.md`](../ANALYSIS.md)):

* **FCL (`FCL-Team/FoldCraftLauncher`)** — clean UI / core / bootstrap split.
  We mirror its `FCL` / `FCLCore` / `FCLauncher` / `Terracotta` / `ZipFileSystem`
  boundaries with `:app` / `:core` / `:runtime` and Rust modules named after the
  same subsystems (`auth`, `download`, `launch`, `game`, `mod`, `fakefx`, …).
* **cuberite** — strict module boundaries and a cross-platform CI with a hard
  formatting gate (`StyleCheck.yml`). We adopt its "each unit is independently
  buildable & testable" ethos: every Rust subsystem carries its own
  `#[cfg(test)]` modules; `:runtime` carries JVM unit tests.
* **MCTier (`pmh1314520/MCTier`)** — the canonical "Rust core → FFI → Kotlin"
  shape. Our `:core` `RustBridge` object + `rc-launcher-core` `ffi` module
  follow exactly this pattern (plus `catch_unwind` so a Rust panic never aborts
  the VM).
* **Zalith (`ZalithLauncher`)** — mature push/release CI and a plugin-style
  native-library / renderer loading scheme (`NativeLibPlugin`,
  `RendererPlugin`). We adopt its plugin loader interfaces.

---

## 1. The three Gradle modules

```
:app      Jetpack Compose UI (screens, ViewModels, theme, i18n, controllers)
  │  depends only on the RustBridge surface (never native details)
  ▼
:core     Kotlin/JNI bridge — RustBridge.kt + RcEventBus.kt
  │  loads libcrc_launcher.so and forwards calls/events
  ▼
:runtime  JVM/JRE + native library packaging (assets/app_runtime, lib/*.so)
          (no dependencies → trivially unit-testable)
```

* **`:app`** — everything the user sees and touches. Organised by
  `ui/screen`, `ui/viewmodel`, `ui/model`, `ui/theme`, `ui/i18n`, `ui/awt`,
  `ui/component`, `ui/navigation`. It must **never** import anything from the
  Rust crate directly; the only native contact point is `RustBridge` in `:core`.
* **`:core`** — the JNI boundary. `RustBridge.kt` is the single Kotlin facade
  over `libcrc_launcher.so`; `RcEventBus.kt` relays progress/event callbacks
  back to the UI. Keeping the bridge in its own module means the UI stays
  testable and the native surface stays small.
* **`:runtime`** — the on-device runtime: the prebuilt JRE, LWJGL natives,
  GL4ES/ANGLE, caciocavallo and the `lib/*.so` set catalogued in
  `FCL_NATIVE_LIBRARIES.md` / `FCL_APK_RUNTIME_ASSETS_CATALOG.md`. No module
  depends on `:runtime`, which keeps it isolated and easy to audit.

The dependency direction is forced to be acyclic: `:app → :core → :runtime`.

---

## 2. The Rust core (Cargo workspace)

`rust/` is a Cargo workspace cross-compiled to a native library
(`libcrc_launcher.so`) by `cargo-ndk`. The crate `rc-launcher-core` is built as
both `cdylib` (for JNI) and `rlib` (so `cargo test` can exercise the internal
subsystems on the host without an Android NDK).

```
rust/crates/rc-launcher-core/src
├── lib.rs            public crate surface (module declarations, re-exports)
├── error.rs          LauncherError + thiserror-based error tree
├── event.rs          event bus types shared with FFI
├── ffi.rs            #[no_mangle] C ABI exposed to JNI (cbindgen-friendly)
├── capi.rs           C-ABI helper layer (rc_* C symbols) for JNI ergonomics
├── jobs.rs           async job/task scheduler used across subsystems
├── auth/             Microsoft OAuth + offline accounts, secure token vault
├── download/         resumable, chunked, parallel download manager + hashing
├── net/              China-mainland network optimisation (mirror/DNS/proxy)
├── game/             version_manifest + game.json + libraries + assets resolve
├── mods/             Forge/Fabric/Quilt/OptiFine metadata, conflict detection
├── launch/           JVM command assembly, process supervision, AWT, render
├── runtime/          JRE/JDK supply: manifest, extraction, multi-version homes
├── plugins/          renderer / native-lib / FCL-APK plugin loaders + validation
├── i18n/             core string catalogue + locale formatting
├── robust/           retry/backoff, bounded cache, crash reporter
└── util/             bufpool (zero-alloc hot paths), helpers
```

### Design invariants

* **Async-first, never block the VM.** The core is built on `tokio`; heavy work
  runs behind `spawn_blocking`. The Kotlin side wraps blocking JNI calls in
  `Dispatchers.IO` (task 25) so the two schedulers form a two-layer shield.
* **Panics never escape.** Every `#[no_mangle]` entry point is wrapped in
  `catch_unwind` (see `ffi.rs`) and returns an error code instead of aborting
  the JVM — the MCTier lesson, applied everywhere.
* **Streaming, not buffering.** Large files are downloaded and `.tar.xz`
  archives are extracted straight from disk streams (`util::bufpool`,
  `runtime::extract`, `download::client::fetch_range_into`) so memory stays
  bounded on low-end devices.
* **China-mainland first.** The `net` module is the download manager's
  `HttpSource`, so mirror selection, DoH, Happy Eyeballs and proxy support are
  inherited automatically by every download in the launcher.

---

## 3. The FFI / JNI seam

```
Kotlin (RustBridge.kt)  ──JNI──▶  ffi.rs (#[no_mangle] C ABI)
                                       │
                          capi.rs (rc_* C helpers, zero-copy ByteBuffers)
                                       │
                          rc-launcher-core subsystems (async, tokio)
```

* `ffi.rs` declares the `#[no_mangle]` functions JNI calls. Each one converts
  its raw arguments, calls into the crate, and converts the result/error back
  across the boundary — always inside `catch_unwind`.
* `capi.rs` adds a thin C-ABI layer (`rc_*`) so the Kotlin side can pass and
  receive buffers with **zero copies** (direct `ByteBuffer` for AWT frames,
  event draining, etc.).
* Progress and lifecycle events travel the other way through `RcEventBus.kt`
  (task 10), giving the UI an event-bus rather than a poll loop.

Thread-safety: FFI calls may arrive on any thread; the core uses `Arc`/interior
mutability and the shared tokio runtime, so no call blocks the calling thread
for longer than the FFI marshalling.

---

## 4. Build & release pipeline

```
                rust-core (matrix: 4 ABIs, cargo-ndk)
                          │  libcrc_launcher.so
                rust-test  (host cargo test) ── gate ─┐
                          │                           │
                      android (assemble + sign APK/AAB)│
                          │                           │
                       release (tag → GitHub Release + SHA-256)
                          │
                      health  (mirror speed-test + license audit, daily)
```

The Rust `.so` artifacts are assembled into `core/src/main/jniLibs/<abi>/` so
the Android library packages them into the final APK/AAB automatically. Full
commands and the signing/keystore flow are in [`BUILD.md`](BUILD.md); the
code-style gate that must pass before merge is in
[`stylecheck.yml`](../.github/workflows/stylecheck.yml) and
[`CONTRIBUTING.md`](CONTRIBUTING.md).

---

## 5. Where each task lives

| Concern | Module(s) | Docs |
|---|---|---|
| Scaffold / modules | Gradle + Cargo | this file |
| Download (resume+verify) | `download/` | `download/mod.rs` |
| CN network (mirror/DNS/proxy) | `net/` | `net/mirror.rs`, `net/dns.rs` |
| Version & dependency resolve | `game/` | `game/resolve.rs` |
| Accounts & auth | `auth/` | `docs/auth.md` |
| JRE/JDK supply | `runtime/` | `runtime/manager.rs` |
| Launch engine | `launch/` | `docs/launch.md` |
| Mods / packs / shaders | `mods/` | `mods/mod.rs` |
| Renderer / native-lib plugins | `plugins/`, `launch/render.rs` | `docs/rendering.md` |
| FFI / JNI | `ffi.rs`, `capi.rs` | `docs/ffi_event_bus.md` |
| AWT/Swing compat (fakefx) | `launch/awt*.rs`, `launch/fakefx.rs` | `docs/awt.md` |
| i18n | `i18n/`, `ui/i18n` | `docs/i18n.md` |
| Health audit (CI) | `scripts/health_audit.py` | `docs/health_audit.md` |
