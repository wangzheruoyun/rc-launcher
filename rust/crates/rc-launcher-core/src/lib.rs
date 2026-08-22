//! RC Launcher — Rust core.
//!
//! This crate is cross-compiled to `librc_launcher.so` (via `cargo ndk`) and
//! exposed to Kotlin through JNI (see [`ffi`]). The internal subsystems mirror
//! the module boundaries of FCL's `FCLCore` and will be filled in across the
//! roadmap:
//!
//! | module   | purpose                                   | roadmap task |
//! |----------|-------------------------------------------|--------------|
//! | `net`    | China-mainland network optimisation (mirrors / DoH / proxy) | 3 (implemented) |
//! | `download` | async download w/ resume + verify       | 2            |
//! | `auth`   | account & authentication                 | 5 (implemented) |
//! | `game`   | version manifest & dependency resolution | 4 (implemented) |
//! | `runtime`| JRE / JDK supply (OpenJDK packaging / extraction) | 6 (implemented) |
//! | `launch` | launch engine                            | 7 (implemented) |
//! | `mods`   | mod / resource-pack / shader management      | 8 (implemented) |
//! | `plugins`| pluggable renderer & native-lib extension (registry / injection / validation) | 9 (implemented) |
//! | `ffi`/`event`/`capi`/`jobs` | FFI/JNI bridge: event bus + async callbacks + C-ABI (cbindgen) | 10 (implemented) |
//! | `i18n`   | internationalisation: resource-file catalogues (zh-CN base / zh-Hant / en), negotiation, `{name}` + plurals, runtime overlay | 20 (implemented) |
//! | `error`  | unified `RcError` model + recoverability metadata (severity / retryable / backoff) | 19 (implemented) |
//! | `robust` | robustness layer (task 19): network-jitter retry/backoff, offline cache degradation, crash logging + reporting | 19 (implemented) |
//!
//! Each subsystem is a self-contained, unit-tested unit so the core stays
//! robust and testable independently of the Android UI.
//!
//! Implemented subsystems:
//! * **`ffi` / `event` / `capi` / `jobs`** (task 10) — the FFI/JNI bridge layer.
//!   A process-wide [`event::EventBus`] streams structured, JSON events
//!   (progress / log / lifecycle / error) from Rust worker threads into
//!   Kotlin. Kotlin subscribes once via `RustBridge.eventBusSubscribe(RcEventSink)`;
//!   the Rust side keeps a JNI `GlobalRef` + `JavaVM` and attaches *any* worker
//!   thread to the JVM to invoke the callback (the EasyTier / MCTier
//!   attach-per-thread pattern). Long-running work is started fire-and-forget
//!   through [`jobs::spawn_job`] and [`ffi`] `runAsync`, reporting exclusively
//!   via the bus. A flat C-ABI ([`capi`], consumed by `cbindgen` into
//!   `rc_launcher.h`) mirrors MCTier's `libeasytier_ffi.so` so non-JNI native
//!   consumers (Unity/Unreal plugins, CLI) can drive the core too. Every FFI
//!   entry point is wrapped in `catch_unwind` so a panic never aborts the VM.
//!
//! * **`i18n`** (task 20) — the internationalisation framework. Every
//!   user-facing string lives in a `.properties` **resource file** under
//!   `crates/rc-launcher-core/i18n/` (`zh-CN` / `zh-Hant` / `en`), embedded with
//!   `include_str!` so lookups are allocation-free. The launcher is
//!   **Chinese-first**: `zh-CN` is the base locale, it is complete by contract
//!   (a unit test enforces it), an unknown device locale resolves to it and every
//!   other catalogue falls back to it key-by-key — so an untranslated string
//!   shows Chinese copy, never a raw key. [`i18n::set_language`] is a single
//!   atomic store, which is what makes the Compose picker switch instantly.
//!   Crash verdicts ([`launch::crash::CrashCategory`]) and every [`RcError`]
//!   variant read their copy from the same catalogues, so the core and the UI can
//!   never disagree; [`i18n::bundle`] hands Kotlin the whole resolved table in
//!   one FFI crossing, and an on-disk *overlay* can hot-fix wording or add a
//!   community translation without a new APK.
//! * **`download`** (task 2) — a resumable, parallel, chunked
//!   [`download::DownloadManager`] built on `tokio` + `reqwest` with SHA-1/MD5
//!   verification, exponential-backoff retries and a cumulative progress
//!   callback.
//! * **`net`** (task 3) — China-mainland network optimisation: built-in
//!   BMCLAPI/MCBBS/Aliyun mirrors with automatic speed-testing and selection,
//!   DNS optimisation (DoH resolvers, static/custom resolvers, Happy Eyeballs,
//!   connection reuse, timeouts and exponential backoff) and configurable
//!   HTTP/HTTPS/SOCKS5 proxy. [`net::NetworkClient`] also implements
//!   [`download::HttpSource`] so the download manager inherits every network
//!   optimisation automatically.
//! * **`game`** (task 4) — version manifest & dependency resolution: parses
//!   Mojang's `version_manifest` and each `version.json`, resolves the
//!   `inheritsFrom` chain, filters libraries by platform rules, and produces a
//!   deduplicated [`game::DownloadPlan`] whose URLs are rewritten through the
//!   mirror provider (so the download manager inherits mirror fallback).
//! * **`runtime`** (task 6) — JRE / JDK supply: typed [`runtime::Abi`] /
//!   [`runtime::JavaVersion`] models, a [`runtime::JreManifest`] (with a
//!   `from_prebuilt_dir` scanner CI uses to verify the committed manifest),
//!   pluggable [`runtime::JreSource`]s (local directory or HTTP(S) over the
//!   task-3 client), pure-Rust `.tar.xz` extraction ([`runtime::extract`]) and a
//!   [`runtime::RuntimeManager`] that installs + verifies + tracks JRE homes
//!   with multi-version coexistence and on-demand release. The end-to-end tests
//!   run against the real prebuilt FCL JRE packages extracted from the FCL APK.
//! * **`auth`** (task 5) — account & authentication: Microsoft OAuth 2.0
//!   *device-code* login with the full `XBL → XSTS → Minecraft` token chain
//!   ([`auth::microsoft`]), offline accounts with deterministic offline UUIDs
//!   ([`auth::offline`]), and secure token storage ([`auth::store`] +
//!   [`auth::vault`]) with proactive auto-refresh ([`auth::AccountManager`]).
//!   All flows are written against the pluggable [`auth::transport::AuthTransport`]
//!   trait so they are unit-tested with a scripted mock (no network).
//! * **`launch`** (task 7) — the launch engine: it turns a
//!   [`game::ResolvedVersion`] + [`launch::LaunchOptions`] + a provisioned JRE
//!   into a running, supervised, *diagnosable* game process.
//!   [`launch::LaunchEngine`] runs the preflight checks (JRE present and new
//!   enough, `app_runtime/` complete, every classpath entry on disk, runtime
//!   directories created), assembles the classpath
//!   ([`launch::ClasspathBuilder`], including the LWJGL substitution Android
//!   needs), the process environment ([`launch::build_env`]) and the JVM command
//!   line ([`launch::CommandBuilder`]: heap/GC, encoding, `java.library.path`,
//!   renderer properties, the caciocavallo AWT bridge, rule-filtered and
//!   `${...}`-templated manifest arguments), then spawns and supervises the JVM
//!   ([`launch::GameProcess`]: streamed stdout/stderr, a bounded log ring
//!   buffer, exit code / signal, SIGTERM→SIGKILL stop) and classifies the
//!   outcome ([`launch::crash`]: 16 categories with evidence, `hs_err` files and
//!   bilingual advice). Secrets are redacted everywhere; `cargo run --example
//!   launch_demo` exercises the whole pipeline against a fake JRE.
//!
//! * **`mods`** (task 8) — mod / resource-pack / shader management: a
//!   per-instance [`mods::ModManager`] that scans the `mods/` folder, detects
//!   each loader by its manifest (`fabric.mod.json` / `quilt.mod.json` /
//!   `META-INF/mods.toml` / `mcmod.info` / `litemod.json`; OptiFine by file
//!   name) and parses it into a uniform [`mods::ModMetadata`] (ids, versions,
//!   dependencies, conflicts). [`mods::resolve_issues`] walks every enabled
//!   mod's `depends` / `breaks` edges against the instance's Minecraft version
//!   and reports missing-dependency / incompatible / conflict / duplicate /
//!   wrong-MC-version [`mods::ModIssue`]s. [`mods::resource_pack`] and
//!   [`mods::shader`] manage `pack.mcmeta` resource packs (with
//!   `pack_format` vs version compatibility) and OptiFine/Iris shader packs
//!   (validated by their `shaders/` tree), all with durable `.disabled`-suffix
//!   enable/disable and per-instance version isolation. Every parser is
//!   lenient (a bad dependency string degrades to "no constraint" instead of
//!   failing the scan).

#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::items_after_test_module)]

pub mod auth;
pub mod capi;
pub mod download;
pub mod error;
pub mod event;
pub mod ffi;
pub mod game;
pub mod i18n;
pub mod jobs;
pub mod launch;
pub mod mods;
pub mod net;
pub mod plugins;
pub mod robust;
pub mod runtime;
pub mod util;

/// Crate version, surfaced to the UI through [`ffi::get_version`].
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Returns a greeting used to validate the JNI boundary end-to-end.
pub fn greet(name: &str) -> String {
    format!("Hello, {}! (RC Launcher core {})", name, VERSION)
}

/// Convenience re-export of the i18n language type (task 20).
pub use i18n::Language;

/// Convenience re-export of the unified error type ([`error::RcError`]).
pub use error::RcError;

/// Convenience re-exports for the robustness layer (task 19).
pub use robust::*;

#[cfg(test)]
mod integration_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        assert!(VERSION.starts_with('0'));
    }

    #[test]
    fn greet_includes_name_and_version() {
        let g = greet("Player");
        assert!(g.contains("Player"));
        assert!(g.contains(VERSION));
    }
}
