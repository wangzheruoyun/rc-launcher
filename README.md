# RC Launcher

A high-performance, robust **Minecraft Java Edition launcher for Android**, written
with a **Rust core** (download / auth / launch / version logic, cross-compiled to
native `.so` via `cargo-ndk`) and a **Jetpack Compose** UI, optimised for the
China-mainland network (mirror sources, DNS optimisation, resume-friendly downloads).

This repository currently implements:

* **Task 1 — project scaffold & multi-module engineering** (Gradle + Cargo
  workspace, `:app` / `:core` / `:runtime` module boundaries).
* **Task 2 — async download manager (resume + verification)**: a resumable,
  parallel, chunked manager with SHA-1/MD5 verification, exponential-backoff
  retries and a cumulative progress callback
  (`rust/crates/rc-launcher-core/src/download/`).
* **Task 3 — China-mainland network optimisation** : built-in BMCLAPI / MCBBS /
  Aliyun mirrors with automatic speed-testing & selection, DNS optimisation
  (DoH resolvers, static/custom resolvers, Happy Eyeballs, connection reuse,
  timeouts and exponential backoff) and configurable HTTP/HTTPS/SOCKS5 proxy
  (`rust/crates/rc-launcher-core/src/net/`). The network client implements the
  download crate's `HttpSource`, so the task-2 downloader automatically inherits
  every network optimisation.

* **Task 6 — JRE / JDK supply (Android OpenJDK packaging / extraction)**: a
  `runtime` subsystem that consumes FCL's prebuilt, cross-compiled OpenJDK
  packages (one `universal.tar.xz` + a `bin-<abi>.tar.xz` slice per ABI under
  `assets/app_runtime/java/jre<major>/`, pre-set from the FCL APK). It verifies
  each archive by SHA-1, extracts them with a pure-Rust `.tar.xz` decoder, and
  tracks on-disk JRE homes with multi-version coexistence (Java 8 / 17 / 21 / 25)
  and on-demand release (`rust/crates/rc-launcher-core/src/runtime/`).

* **Task 7 — launch engine**: the [`launch`](rust/crates/rc-launcher-core/src/launch/)
  subsystem turns a resolved version + a provisioned JRE + the user's options into
  a running, supervised and *diagnosable* game process: preflight checks, classpath
  assembly with LWJGL substitution, the full JVM command line (including the
  caciocavallo AWT bridge and renderer properties), process supervision with a
  bounded log buffer, and crash classification with bilingual advice.
  See [docs/launch.md](docs/launch.md).

* **Task 17 — render integration (LWJGL + GL4ES/ANGLE)**: the
  [`launch::render`](rust/crates/rc-launcher-core/src/launch/render.rs) module wires
  FCL/Zalith's prebuilt LWJGL 3.3.x native libraries, the GL4ES/ANGLE OpenGL→OpenGL
  ES translation layers and a tunable performance profile into the game's native
  search path and environment: it validates the on-disk LWJGL natives
  (`liblwjgl.so` / `liblwjgl_opengl.so` …) before spawn, selects the ANGLE Vulkan
  backend, and emits `LIBGL_*` / Mesa throughput knobs for weak devices.
  See [docs/rendering.md](docs/rendering.md).

* **Task 18 — AWT/Swing compatibility layer (fakefx)**: caciocavallo renders
  Minecraft's *embedded* UI (Forge / OptiFine installers, the Mojang splash,
  `JOptionPane` crash dialogs, font metrics) into an off-screen ARGB desktop
  inside the game JVM; the core hosts that session
  ([`launch::awt`](rust/crates/rc-launcher-core/src/launch/awt.rs),
  [`fakefx`](rust/crates/rc-launcher-core/src/launch/fakefx.rs),
  [`awt_host`](rust/crates/rc-launcher-core/src/launch/awt_host.rs)) and Compose
  draws it: named-pipe frame/event channels pumped off the UI thread, a
  double-buffered damage-tracking canvas, damaged rows converted straight into the
  direct `ByteBuffer` behind the Compose `Bitmap` (zero copy), letterboxing with
  identical integer math on both sides, and touches / keys / IME text translated
  back into `java.awt.event.*` records. See [docs/awt.md](docs/awt.md).

* **Task 11 — app framework & navigation**: a Material 3 + Navigation Compose
  shell (`MainScreen` / `RcNavHost` / `RcBottomNavigationBar`) with a
  ViewModel-bound theme engine and a night-mode quick toggle.

* **Task 12 — home & instance dashboard**: game-instance cards, one-tap quick
  launch with a launch-lifecycle banner, a "最近游玩" rail, a live
  "资源占用" panel (memory / storage / CPU read from the device) and a floating
  frame-rate HUD (Choreographer-based FPS, see `ui/resource/`). The dashboard is
  driven by `DashboardViewModel` over a `StateFlow`; the actual JVM spawn is
  delegated to a `LaunchExecutor` so the Rust-core integration (task 7 preflight
  + task 10 event bus) can replace the simulator without UI changes.

* **Task 20 — internationalisation (zh-CN / zh-Hant / en)**: a resource-file
  i18n framework that is **Chinese-first**. Every user-facing string lives in
  `rust/crates/rc-launcher-core/i18n/<tag>.properties`; `zh-CN` is the base
  locale, complete by contract, and every other catalogue falls back to it
  key-by-key, so an untranslated string shows Chinese copy instead of a raw key.
  The core owns the copy — crash verdicts and every `RcError` variant read the
  same catalogues — and hands Kotlin the whole table in one FFI crossing
  (`i18nBundle`). The Android resources (`values/`, `values-en/`,
  `values-zh-rTW/`, `locales_config.xml`, the `key → R.string` map) are
  *generated* from those files, so the two can never drift. Switching is
  instant: `LocaleEngine.setLanguage(...)` re-provides a Compose
  `CompositionLocal` — no Activity recreation — and is mirrored into Android 13+
  per-app language settings. A runtime *overlay* ships community translations or
  wording hot-fixes without a new APK. See [docs/i18n.md](docs/i18n.md).

Later tasks (mods, FFI event bus, UI detail screens, CI release, tests,
performance) build on top of this skeleton.

## Render integration (task 17)

The [`launch::render`](rust/crates/rc-launcher-core/src/launch/render.rs) module is
what actually *integrates* the prebuilt Android graphics stack (task 7 lays the
classpath / env wiring; this module owns the native-library inventory and tuning):

* **LWJGL native bundle** — [`LwjglNativeBundle`] discovers and validates
  `app_runtime/lwjgl/<ver>/natives/<abi>/*.so` against a version-aware manifest
  (`LWJGL_3_3_3_NATIVES` / `LWJGL_3_4_1_NATIVES`). A missing *required* native
  (`liblwjgl.so`, `liblwjgl_opengl.so`) fails preflight with an actionable error
  instead of letting the JVM die with `UnsatisfiedLinkError`.
* **GL4ES / ANGLE translation** — [`gl_translation_env`] emits the OpenGL→OpenGL ES
  translation layer: GL4ES is `dlopen`ed by LWJGL through
  `-Dorg.lwjgl.opengl.libname` (set by the command builder) and reads `LIBGL_*`;
  ANGLE is pointed at its Vulkan backend via `ANGLE_DEFAULT_PLATFORM=vulkan`.
* **Performance profile** — [`PerfProfile`] (Diagnostic / Balanced / LowPower /
  HighPerformance / Maximum) trades GL error-checking for throughput on weak GPUs
  through `LIBGL_NOERROR` / `MESA_NO_ERROR` / `LIBGL_NOINDIRECT` / `LIBGL_FPS`,
  applied last by `build_env` so the user can still override them.

The launch engine runs [`LwjglNativeBundle::discover`] in `preflight_app_runtime`
and `build_env` layers the translation + perf environment on top of the renderer's
base variables. All of it is unit-tested (`cargo test --workspace`).

## AWT / Swing compatibility (task 18, "fakefx")

Task 17 draws the *game*; task 18 draws everything the game builds on the desktop
toolkit. Android has no X11, so the moment an installer or a crash dialog touches
`java.awt.Toolkit` the JVM dies with `AWTError: Can't connect to X11 window
server`. The fix is FCL's: **caciocavallo** re-implements the AWT peers into an
off-screen ARGB desktop — and this repository adds the half that makes it
*visible and touchable* on a phone:

* **Backend & preflight** — [`AwtBackend`](rust/crates/rc-launcher-core/src/launch/awt.rs)
  picks `caciocavallo` (Java 8) or `caciocavallo17` (Java 9+), validates the jars
  and the AWT natives on disk, and emits the JVM arguments that activate the
  bridge (toolkit, graphics env, `-Dcacio.managed.screensize`, the Java-17
  `--add-opens` set, `-javaagent:cacio-agent.jar`). A missing **required** jar
  fails preflight instead of dying at the first `Toolkit.getDefaultToolkit()`.
* **Transport** — [`AwtHost`](rust/crates/rc-launcher-core/src/launch/awt_host.rs)
  creates two named pipes (`awt-frames.rcaf` / `awt-events.rcae`, advertised to
  the JVM through `LaunchOptions.awt_transport_dir`) and pumps them on two
  threads, so blocking I/O and frame validation never touch the UI thread. Both
  pumps `poll(2)` with a timeout, so stopping the session never hangs.
* **Canvas** — frames carry **only the damaged rectangle**; `AwtCanvas` is double
  buffered, coalesces damage and converts just the changed rows into RGBA8888 —
  directly into the `ByteBuffer` backing the Compose `Bitmap`. An idle desktop
  answers "nothing changed", so the UI skips the upload *and* the recomposition.
* **Input** — Compose gestures are batched once per frame, mapped through the same
  integer viewport math on both sides, and translated into `MOUSE_*` / `KEY_*`
  records with AWT's `getModifiersEx()` contract (drag vs move, a synthetic
  `MOUSE_CLICKED` after a steady tap, letterbox taps ignored, focus loss releasing
  everything held).
* **UI** — `AwtCanvasSurface` (the canvas) and `AwtScreen` (settings → 渲染器与画面 →
  「AWT / Swing 兼容层」) with a live diagnostics card and a self-test that pushes a
  locally generated pattern through the entire pipeline without a running game.

```bash
cd rust && cargo test --workspace   # 524 tests, incl. 81 for i18n (catalogues / negotiation / FFI / overlay)
cargo run --example awt_demo        # end-to-end over real named pipes, no JVM needed
```

See [docs/awt.md](docs/awt.md) for the wire formats, the FFI table and the
robustness matrix.

## Module layout

```
RCLauncher/
├── app/                 # :app     — Compose UI (Android application)
├── core/                # :core    — Rust/JNI bridge + runtime resources (Android library)
│   └── src/main/jniLibs/<abi>/librc_launcher.so   # produced by cargo-ndk
├── runtime/             # :runtime — JRE / native library management (Android library)
├── rust/                # Cargo workspace
│   └── crates/rc-launcher-core/   # cdylib -> libcrc_launcher.so + unit-tested units
└── .github/workflows/   # build.yml / release.yml / health.yml  (all builds run on GitHub Actions)
```

### Dependency direction (clear & acyclic)

```
:app  ──▶  :core  ──▶  :runtime
```

* **`:runtime`** — pure Kotlin filesystem / JRE layout logic. No native, no UI.
  Fully unit-testable on the JVM.
* **`:core`** — owns the JNI bridge (`RustBridge`) and ships the native
  `librc_launcher.so`. Depends on `:runtime`.
* **`:app`** — Compose UI. Talks to the core **only** through `RustBridge`.

### Rust core subsystems (mirror FCL `FCLCore` boundaries)

| crate module | purpose | roadmap |
|---|---|---|
| `net`     | China-mainland network optimisation (mirrors / DoH / proxy) | 3 (implemented) |
| `download`| async download w/ Range resume + SHA-1/MD5 verify (implemented) | 2 |
| `auth`    | account & authentication (Microsoft OAuth / offline)       | 5 |
| `game`    | version manifest & dependency resolution                   | 4 |
| `runtime` | JRE / JDK supply (OpenJDK packaging / extraction)          | 6 (implemented) |
| `launch`  | launch engine (JVM args, process spawn, crash capture) + render integration (task 17) | 7 (implemented) |
| `mods`    | mod / resource-pack management                             | 8 |
| `error`   | unified `RcError` model + recoverability metadata (severity / retryable / backoff) | 19 (implemented) |
| `robust`  | robustness layer (task 19): network-jitter retry/backoff, offline cache degradation, crash logging + reporting | 19 (implemented) |
| `i18n`    | internationalisation (task 20): resource-file catalogues (zh-CN base / zh-Hant / en), locale negotiation, `{name}` + plurals, runtime overlay | 20 (implemented) |

Each subsystem is a self-contained, unit-tested unit (`cargo test`).

## Network optimisation (task 3)

The [`net`](rust/crates/rc-launcher-core/src/net/) module is what makes downloads
reliable from the China mainland:

* **Mirrors** — [`MirrorSource`](rust/crates/rc-launcher-core/src/net/mirror.rs)
  rewrites a canonical Mojang CDN URL onto BMCLAPI / MCBBS / Aliyun
  (path-preserving). [`MirrorProvider`] measures each mirror's latency and pins
  the fastest reachable one; the client then transparently retries failed
  downloads against the mirrors in priority order.
* **DNS** — [`DnsConfig`](rust/crates/rc-launcher-core/src/net/dns.rs) supports
  `System`, `Static` (explicit host→IP overrides to defeat DNS poisoning) and
  `Doh` (DNS-over-HTTPS via Aliyun / DNSPod / 360 / Cloudflare / Google). The
  resolved addresses are turned into `reqwest` `resolve_to_addrs` overrides, and
  Happy Eyeballs is enabled via the `hickory-dns` resolver.
* **Proxy** — [`ProxyConfig`](rust/crates/rc-launcher-core/src/net/proxy.rs)
  supports HTTP / HTTPS / SOCKS5.
* **Client** — [`NetworkClient`](rust/crates/rc-launcher-core/src/net/client.rs)
  ties it together with connection reuse (keep-alive pooling), connect/read
  timeouts and exponential-backoff retries, and implements
  [`download::HttpSource`] so the resumable download manager (task 2) inherits
  mirror fallback, DoH and proxy support with zero extra wiring.

The Rust core unit tests (`cargo test --workspace`) cover mirror rewriting,
best-mirror selection, DoH request building / JSON parsing, proxy building, the
mirror-fallback + retry algorithm (offline mock) and the client builder — all
without requiring network access.

## Launch engine (task 7)

The [`launch`](rust/crates/rc-launcher-core/src/launch/) module is what actually
starts Minecraft on a phone:

* **Preflight** — [`LaunchEngine::prepare`](rust/crates/rc-launcher-core/src/launch/engine.rs)
  validates the options, checks the JRE exists *and* satisfies the version's
  `javaVersion.majorVersion`, verifies the `app_runtime/` bundle (LWJGL /
  caciocavallo), checks every classpath entry is on disk and creates the
  directories the JVM writes into. Anything that can fail is caught **before** a
  JVM is spawned, with an actionable error.
* **Classpath** — [`ClasspathBuilder`](rust/crates/rc-launcher-core/src/launch/classpath.rs)
  filters libraries by rule, **substitutes the manifest's desktop `org.lwjgl:*`
  jars with the prebuilt Android bundle** (exactly like FCL), and collapses
  duplicate Maven coordinates to the highest version.
* **Command line** — [`CommandBuilder`](rust/crates/rc-launcher-core/src/launch/command.rs)
  emits heap/GC flags, UTF-8 encoding, `java.library.path` (natives + LWJGL +
  `nativeLibraryDir` + JRE + system GLES dirs), JNA, the renderer's
  `-Dorg.lwjgl.opengl.libname`, mod-loader quirks, the log4j config, the
  caciocavallo AWT bridge (Java 8 and Java 17+ variants, incl. `--add-opens` and
  `-javaagent:`), then the rule-filtered, `${...}`-templated manifest arguments.
  Arguments whose placeholders cannot be resolved are dropped **with their flag**
  (an offline account has no `${clientid}`), and every drop is reported.
* **Process** — [`GameProcess`](rust/crates/rc-launcher-core/src/launch/process.rs)
  streams stdout/stderr to a callback, keeps a bounded ring buffer for diagnosis,
  reports the exit code / terminating signal, and stops the game with
  SIGTERM → SIGKILL so Minecraft can save the world first.
* **Crash diagnosis** — [`crash`](rust/crates/rc-launcher-core/src/launch/crash.rs)
  classifies a failed session into 16 categories (out of memory, wrong Java
  version, missing native library, renderer failure, native JVM crash, corrupt
  file, mod-loader failure, Android LMK kill, ...) with the log lines that prove
  it, any `hs_err_pid*.log`, and advice in English **and Chinese**.
* **Secrets** — the Minecraft access token appears in `argv`, so it is redacted
  from `Debug`, from the printable command line and from every captured log line.

```bash
cd rust
cargo test --workspace            # 118 launch-subsystem unit tests included
cargo run --example launch_demo   # end-to-end: clean exit / OOM crash / forced stop
```

## Building (GitHub Actions)

All builds run on GitHub Actions — there is no local Android SDK / NDK requirement:

* **`build.yml`** (on push/PR): cross-compiles the Rust core to `librc_launcher.so`
  for `arm64-v8a`, `armeabi-v7a`, `x86_64`, `x86` via `cargo-ndk`, runs the Rust
  unit tests, then builds the signed APK + AAB with Gradle. Caches Cargo & Gradle.
* **`release.yml`** (on `v*` tags): reuses `build.yml` and publishes a GitHub
  Release with the APK/AAB and SHA-256 checksums.
* **`health.yml`** (daily + manual): the task-24 supply-chain gate — probes the
  BMCLAPI / MCBBS / Aliyun mirrors for speed & reachability, re-validates the
  committed `jre_manifest.json` SHA-1/size against the on-disk JRE slices, and
  audits every third-party crate (via `cargo metadata`) + Gradle dependency for
  its license. Emits `health-report.json`/`health-report.md`/`third_party_licenses.md`
  (echoed to the job summary) and opens a tracking issue when the report is
  unhealthy (manifest drift / AGPL / all mirrors down).

### Signing the release artifacts

The release build is signed when the following repository **secrets** are set
(consumed by `app/build.gradle.kts` through the `KEYSTORE_BASE64` gate, exactly
like the FCL `build.yml` / Zalith `push_ci` flow where the key lives base64-encoded
in secrets and is materialised on the runner):

| Secret | Meaning |
| --- | --- |
| `KEYSTORE_BASE64` | base64 of the release `.jks` keystore (`base64 -w0 key.jks`) |
| `KEYSTORE_PASSWORD` | keystore password |
| `KEY_ALIAS` | signing key alias |
| `KEY_PASSWORD` | signing key password |

`build.yml` decodes `KEYSTORE_BASE64` into `release-keystore.jks` and passes the
passwords/alias to Gradle via env vars. When these secrets are **absent** (e.g. a
fork pull request) the release build automatically falls back to the auto-generated
**debug** key, so an installable APK/AAB is always produced. The `Verify APK
signature` step then confirms the artifact is signed (valid for both the release
key and the debug-key fallback).

## Local development

```bash
# Rust core (host checks / tests)
cd rust && cargo check --workspace && cargo test --workspace

# Android native libs (optional, for local APK builds)
cargo install cargo-ndk
cargo ndk -t aarch64-linux-android -o ../core/src/main/jniLibs build --release
# ... then point local.properties at your SDK and run ./gradlew assembleDebug
```


## Documentation & contributing

* **Architecture** — [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
* **Module interfaces (contracts)** — [docs/MODULES.md](docs/MODULES.md)
* **Build & release** — [docs/BUILD.md](docs/BUILD.md)
* **Contributing** — [CONTRIBUTING.md](CONTRIBUTING.md) (also [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md))
* **Subsystem docs** — `docs/{auth,launch,rendering,awt,i18n,ffi_event_bus,health_audit}.md`

Code style is enforced automatically by
[`.github/workflows/stylecheck.yml`](.github/workflows/stylecheck.yml):
Rust uses `rustfmt.toml` + `clippy.toml` (`cargo fmt --check` and
`cargo clippy -D warnings` are a **hard** gate); Kotlin/Compose uses
`.editorconfig` + `config/detekt/detekt.yml` (`ktlintCheck`, `detekt`). Run the
same checks locally before opening a PR — see [CONTRIBUTING.md](CONTRIBUTING.md).

## License

GPL-3.0-or-later.
