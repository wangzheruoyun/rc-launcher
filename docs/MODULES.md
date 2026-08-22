# Module interfaces

This document is the **contract surface** of RC Launcher: the public types and
functions each module exposes, and the boundaries a contributor must respect.
It pairs with [`ARCHITECTURE.md`](ARCHITECTURE.md) (how things fit together) and
[`BUILD.md`](BUILD.md) (how to build). Every entry below is a stable seam — code
*behind* it may change, but the signatures documented here are the integration
points for the Compose UI, the CI and external tooling.

## 1. Rust core (`rc-launcher-core`)

Built as `cdylib` (`libcrc_launcher.so`, loaded by `:core` via JNI) **and**
`rlib` (so `cargo test` exercises the subsystems on the host). The crate root
is `rust/crates/rc-launcher-core/src/lib.rs`.

| Module | Public surface (selected) | Responsibility |
|---|---|---|
| `error` | `RcError`, `RcResult`, severity / `retryable` / `backoff` metadata | Unified error model + recoverability hints. |
| `event` | `EventBus`, `Event`, `EventKind`, `EventSink`, `publish*` helpers | Process-wide event bus streamed to Kotlin. |
| `jobs` | async job/task scheduler | Drives long-running work off the UI thread. |
| `auth` | `MicrosoftService`, `OfflineAccount`, `AuthStore`, `AuthResult` | MS OAuth device-code + XBL/XSTS chain, offline accounts, secure token vault. |
| `net` | `MirrorProvider`, `DnsResolver`, `ProxyConfig`, `HttpClient` | CN mirrors (BMCLAPI/MCBBS/Aliyun) + DoH/Happy-Eyeballs + HTTP/SOCKS proxy. |
| `download` | `DownloadManager`, `DownloadOptions`, `ProgressCallback`, `HttpSource` | Resumable, chunked, parallel download + SHA-1/MD5 verify. |
| `game` | `VersionManifest`, `ResolvedVersion`, `DependencyResolver`, `AssetIndexRef` | version_manifest / game.json / libraries / assets resolution. |
| `mods` | `ModLoader`, `ModMetadata`, `ConflictReport`, `ResourcePack`, `Shader` | Forge/Fabric/Quilt/OptiFine metadata, conflict detection. |
| `runtime` | `JreManager`, `JreHome`, `JavaVersion`, `RuntimeManifest` | JRE/JDK supply: manifest, pure-Rust `.tar.xz` extraction, multi-version homes. |
| `launch` | `LaunchEngine`, `LaunchOptions`, `LaunchCommand`, `AwtSession`, `RendererProfile` | JVM arg/classpath assembly, process supervision, AWT bridge, render config. |
| `plugins` | `RendererPlugin`, `NativeLib`, `TrustStore`, `PluginValidator` | Pluggable renderer + native-lib injection, tamper/trust validation. |
| `i18n` | catalogue types, `Language`, `translate`/`format` helpers | zh-CN / zh-Hant / en catalogues, negotiation, `{name}` + plurals, overlay. |
| `robust` | `RetryPolicy`, `bounded_cache`, `CrashReporter` | Retry/backoff, offline cache degradation, crash reporting. |
| `util` | `BufPool`, `ObjectPool`, helpers | Zero-alloc hot-path buffers (task 25). |
| `ffi` | `#[no_mangle] rc_*` C ABI | The JNI boundary — every Kotlin call enters here, always inside `catch_unwind`. |
| `capi` | `rc_*` C-ABI helpers (zero-copy `ByteBuffer`) | Ergonomic C layer for JNI; direct buffers avoid copies for AWT frames/events. |

**FFI contract (non-negotiable):**
* Every `#[no_mangle]` function in `ffi.rs` must (a) convert raw args, (b) call
  the crate, (c) convert the result/error back, and (d) be wrapped in
  `catch_unwind` so a Rust panic returns an error code instead of aborting the
  JVM. A panic must **never** cross into Kotlin.
* No `#[no_mangle]` function may be `unsafe` to call from JNI without an
  explicit, documented contract in `RustBridge.kt`.
* `capi.rs` helpers that receive/return `ByteBuffer` must use the direct-buffer
  API so frames/events are moved with **zero copies**.

## 2. Kotlin/JNI bridge (`:core`)

`core/src/main/java/com/rc/launcher/core/`:

* **`RustBridge.kt`** — the single Kotlin facade over `libcrc_launcher.so`.
  It is the *only* module that touches native details. It:
  * loads the `.so` via `System.loadLibrary("crc_launcher")`,
  * declares the `external` JNI functions generated from `ffi.rs`/`capi.rs`,
  * converts Kotlin types ↔ Rust types, and
  * exposes a suspending, coroutine-friendly Kotlin API to `:app`.
* **`RcEventBus.kt`** — relays the Rust `event::EventBus` stream (progress /
  log / lifecycle / error) to Kotlin `Flow`/`StateFlow` consumers.

Contract: `:app` **must not** import anything from `rc-launcher-core` directly;
all native access is funnelled through `RustBridge`. Blocking JNI calls in
`RustBridge` are wrapped in `Dispatchers.IO` (task 25) so the UI never stalls.

## 3. Compose UI (`:app`)

`app/src/main/java/com/rc/launcher/ui/` is organised by concern:

* `model/` — immutable state models (`GameInstance`, `Account`, `LauncherSettings`,
  `ControlLayout`, `InstallProfile`) + `*Repository` persistence.
* `viewmodel/` — `ViewModel`s backed by `StateFlow` (e.g. `MainViewModel`,
  `InstallViewModel`, `AccountViewModel`, `SettingsViewModel`).
* `screen/` — one composable per destination (`HomeScreen`, `InstallWizardScreen`,
  `InstanceDetailScreen`, `SettingsScreen`, `AccountsScreen`, `AwtScreen`, …).
* `component/` — reusable UI (`InstanceCard`, `FloatingHud`, `AwtCanvasSurface`,
  `ResourceSummary`).
* `theme/` — `ThemeEngine`/`ThemeData` (FCL-style theming singleton).
* `i18n/` — `LocaleEngine` (mirrors `ThemeEngine`); `rcString()` resolves
  core catalogue strings with an Android `strings.xml` fallback.
* `awt/` — the AWT/fakefx surface that draws the Rust-hosted desktop into a
  Compose `Bitmap` (`AwtCanvasBridge`, `AwtWire`, `AwtInput`, …).
* `navigation/` — `RcNavigation` routes.

Contract: screens observe `ViewModel`s; `ViewModel`s call `RustBridge`; nothing
in `:app` reaches the native layer except through `:core`.

## 4. Runtime assets (`:runtime` + APK)

`:runtime` packages the on-device JRE, LWJGL natives, GL4ES/ANGLE, caciocavallo
and the `lib/*.so` set. The authoritative catalogues live in:
* `FCL_APK_RUNTIME_ASSETS_CATALOG.md` — `assets/app_runtime/...` (caciocavallo,
  JRE slices, LWJGL 3.3.3 / 3.4.1 natives, JNA, …).
* `FCL_NATIVE_LIBRARIES.md` — `lib/arm64-v8a/*.so` (ANGLE, Mesa, GL4ES, terracotta,
  pojavexec, …).

Plugin loaders in `plugins/` (`fcl_apk`, `native_lib`, `renderer`) read these
catalogues to (a) locate verified natives and (b) validate injected plugin
`.so` files against expected size/SHA-1 (see `PluginValidator`).

## 5. Adding a module

* **Rust subsystem:** add `pub mod x;` in `lib.rs`, give it its own
  `#[cfg(test)]` module, and expose it through `ffi.rs` only if the UI needs it.
  Keep `:runtime` dependency-free.
* **Kotlin screen:** add a `StateFlow` `ViewModel`, a composable `screen`, and a
  route in `RcNavigation`; never call JNI outside `RustBridge`.
* **Style:** every new file must pass `cargo fmt --check` (Rust) and
  `ktlintCheck` (Kotlin) — see [`CONTRIBUTING.md`](CONTRIBUTING.md) and
  `.github/workflows/stylecheck.yml`.
