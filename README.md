# RC Launcher

> 🇨🇳 [简体中文](README_zh-CN.md)

A high-performance, robust **Minecraft Java Edition launcher for Android**, built with a
**Rust core** (download / auth / launch / version resolution, cross-compiled to a native
`.so` via `cargo-ndk`) and a **Jetpack Compose** UI, optimised for the China-mainland
network (mirror sources, DNS optimisation, resumable downloads).

## Features

- **Resumable, verified downloads** — parallel chunked downloads with Range resume,
  SHA-1/MD5 verification and exponential-backoff retries.
- **China-mainland network optimisation** — built-in BMCLAPI / MCBBS / Aliyun mirrors
  with automatic speed-test selection, DoH resolvers, Happy Eyeballs and a configurable
  HTTP/HTTPS/SOCKS5 proxy.
- **JRE supply** — consumes FCL's prebuilt OpenJDK packages (Java 8 / 17 / 21 / 25),
  verified by SHA-1 and extracted with a pure-Rust `.tar.xz` decoder.
- **Launch engine** — preflight checks, classpath assembly with LWJGL substitution, the
  full JVM command line, a supervised process and bilingual crash diagnosis.
- **Render integration** — prebuilt LWJGL natives + GL4ES/ANGLE translation and a tunable
  performance profile for weak GPUs.
- **AWT/Swing compatibility (fakefx)** — renders Forge/OptiFine installers and crash
  dialogs into a touchable, zero-copy Compose canvas via caciocavallo.
- **Internationalisation** — Chinese-first `*.properties` catalogues; untranslated strings
  fall back to zh-CN. See [docs/i18n.md](docs/i18n.md).

## Module layout

```
app/      # :app     — Compose UI
core/     # :core    — Rust/JNI bridge + native .so
runtime/  # :runtime — JRE / native library management
rust/     # Cargo workspace → libcrc_launcher.so
```

The dependency direction is strictly acyclic: `:app → :core → :runtime`.
All dependency versions live in `gradle/libs.versions.toml` (single source of truth).

## Building

All builds run on GitHub Actions; the release artifact is a **single `arm64-v8a` `.apk`**
(no AAB). See [docs/BUILD.md](docs/BUILD.md) for the full pipeline and
[.github/workflows/](.github/workflows/) for the CI definition.

```bash
cd rust && cargo test --workspace   # Rust core tests
./gradlew assembleDebug             # local debug APK
```

## Documentation & contributing

- Architecture — [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Module interfaces — [docs/MODULES.md](docs/MODULES.md)
- Build & release — [docs/BUILD.md](docs/BUILD.md)
- Contributing — [CONTRIBUTING.md](CONTRIBUTING.md)
- Subsystem docs — `docs/{auth,launch,rendering,awt,i18n,ffi_event_bus,health_audit}.md`

Code style is enforced by
[`.github/workflows/stylecheck.yml`](.github/workflows/stylecheck.yml)
(Rust `fmt` + `clippy` are a hard gate; Kotlin `ktlint`/`detekt` best-effort).

## License

GPL-3.0-or-later.
