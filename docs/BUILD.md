# Build & release

How to build RC Launcher from source and cut a release. Companion to
[`ARCHITECTURE.md`](ARCHITECTURE.md), [`MODULES.md`](MODULES.md) and
[`CONTRIBUTING.md`](CONTRIBUTING.md). The CI (`.github/workflows/`) performs
exactly these steps; this page is the human-readable version.

## 0. Prerequisites

| Tool | Version | Used for |
|---|---|---|
| Rust (stable) + `rustfmt` + `clippy` | channel pinned in `rust-toolchain.toml` | Rust core |
| `cargo-ndk` | latest | cross-compile `libcrc_launcher.so` for Android ABIs |
| Android NDK | r25+ | targets consumed by `cargo-ndk` |
| Android SDK / Gradle | AGP 9.1.2, Gradle 8.12 | Android app + signing |
| JDK | 17 | Gradle + Android build |
| Python 3 | 3.10+ | `scripts/` (manifest / health tooling) |

Install the Android targets for the Rust core:

```bash
rustup target add aarch64-linux-android
cargo install cargo-ndk
```

## 1. Build the Rust core (arm64-v8a)

```bash
cd rust
# Produces libcrc_launcher.so in target/aarch64-linux-android/release for the arm64-v8a ABI.
cargo ndk -t arm64-v8a -o ../core/src/main/jniLibs build --release
```

The `.so` file lands under `core/src/main/jniLibs/arm64-v8a/` and is packaged
into the APK automatically by the Android build. (CI does this in the
`rust-core` matrix job — see `.github/workflows/build.yml`.)

## 2. Run the Rust tests (host, no NDK)

```bash
cd rust
cargo test --workspace          # unit + integration tests (538+ cases)
cargo clippy --all-targets --all-features -- -D warnings   # lint gate
cargo fmt --all -- --check      # formatting gate
```

## 3. Build / sign the Android app

```bash
./gradlew assembleDebug         # local debug APK
./gradlew assembleRelease       # release APK
```

**Signing.** `app/build.gradle.kts` reads four env vars and creates a
`ciRelease` signing config when they are present:

| Env var | Meaning |
|---|---|
| `KEYSTORE_BASE64` | base64 of the release `.jks` (decoded to `release-keystore.jks` by CI) |
| `KEYSTORE_PASSWORD` | keystore password |
| `KEY_ALIAS` | signing key alias |
| `KEY_PASSWORD` | key password |

When any is absent (local builds, fork PRs without secrets) the build **falls
back to the auto-generated debug key**, so an installable APK is always
produced. This mirrors the FCL `build.yml` / Zalith `push_ci` flow.

## 4. Kotlin style checks (optional, local)

```bash
./gradlew ktlintCheck           # formatting-first Kotlin gate (.editorconfig)
./gradlew detekt                # best-effort static analysis (config/detekt/detekt.yml)
./gradlew ktlintFormat          # auto-normalise Kotlin if ktlintCheck complains
```

## 5. Release pipeline

`release.yml` runs on a version tag / `workflow_dispatch`:

1. **version** — generates a `ci-<date>-<sha>` tag for push/main dispatch.
2. **build** — reuses `build.yml`'s cross-compile + signing to produce the APK.
3. **release** — `softprops/action-gh-release` with generated notes; marks
   `prerelease`/`make_latest` per branch.
4. **checksum** — writes `SHA256SUMS.txt`.
5. **cleanup-artifacts** — deletes CI artifacts older than the keep-last-N
   window via the GitHub API.

See also `health.yml` (daily mirror speed-test + licence audit) and
`stylecheck.yml` (the unified style gate this repo enforces).

## 6. Common tasks

| Goal | Command |
|---|---|
| Regenerate the JRE manifest (with SHA-1 + size) | `python3 scripts/generate_jre_manifest.py` |
| Verify a JRE manifest | `python3 scripts/generate_jre_manifest.py --check` |
| Run the mirror / licence health audit | `python3 scripts/health_audit.py` |
| Check i18n catalogue parity | `python3 scripts/check_i18n.py` |
| Generate Android `strings.xml` from `.properties` | `python3 scripts/gen_android_strings.py` |
