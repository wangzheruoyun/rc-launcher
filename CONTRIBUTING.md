# Contributing to RC Launcher

Thanks for wanting to improve RC Launcher! This guide explains how to set up,
style, test and submit changes. It is the human companion to the automated
gate in `.github/workflows/stylecheck.yml`. The philosophy is borrowed from
cuberite's `StyleCheck` and FCL/Zalith's docs: **one style, one gate, low
friction** — so reviews focus on behaviour, not whitespace.

## 1. Getting started

```bash
git clone https://github.com/com.rc.launcher/launcher.git
cd launcher
# Rust toolchain + components are pinned in rust-toolchain.toml (stable,
# with rustfmt + clippy). The Android build uses AGP 8.5.2 / Gradle 8.9.
./gradlew assembleDebug      # build the app
cd rust && cargo test        # run the Rust core tests
```

## 2. Before you open a PR

Run the same local checks the CI runs (`scripts/` and the style gate):

```bash
# Rust
cd rust
cargo fmt --all -- --check     # must be clean (or run `cargo fmt --all`)
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace

# Kotlin / Compose
./gradlew ktlintCheck          # formatting-first gate (driven by .editorconfig)
./gradlew detekt               # best-effort static analysis
./gradlew testDebugUnitTest    # JVM unit tests (Robolectric)

# Cross-cutting
python3 scripts/check_i18n.py   # i18n catalogue parity
python3 scripts/health_audit.py # (CI-only) mirror + licence audit
```

A PR is mergeable once **Rust `fmt` + `clippy -D warnings`** pass (the strict
gate) and the Kotlin `ktlintCheck` step reports no new formatting violations.
`detekt` is advisory and does not block.

## 3. Code style — the unified rules

All rules are enforced mechanically, not by reviewer preference:

* **Rust** — `rustfmt.toml` (max width 100, edition 2021). Run
  `cargo fmt --all`. `clippy.toml` raises a couple of thresholds for the
  intentionally generic/argument-heavy core.
* **Kotlin / Gradle** — `.editorconfig` (4-space indent, max line 120) + ktlint,
  which reads `.editorconfig` directly (`ktlint_disabled_rules` lives there).
* **Markdown / YAML / TOML / JSON** — `.editorconfig` (2-space for data files,
  `trim_trailing_whitespace`, `insert_final_newline`, `utf-8`, LF endings).
* **Detekt** — `config/detekt/detekt.yml` (merged on Detekt's defaults via
  `buildUponDefaultConfig`).

> Never reformat unrelated files in a behavioural PR — keep diffs focused so the
> review and `git blame` stay readable.

## 4. Architecture & module boundaries

Respect the seams in [`docs/MODULES.md`](docs/MODULES.md):

* `:app` → `:core` → `:runtime` (acyclic; `:app` never touches native details).
* All JNI access is funnelled through `RustBridge.kt` in `:core`.
* Every Rust subsystem has its own `#[cfg(test)]` module; the Rust core builds
  as both `cdylib` and `rlib` so it is unit-testable on the host.

## 5. Commit & PR conventions

* Write clear, imperative commit subjects ("add mirror speed-test selection").
* One logical change per PR; keep it small and focused.
* Reference the roadmap task when relevant (tasks 1–26, see `task_list.txt`).
* Describe **how to test** the change in the PR body.
* For user-facing strings, add them to the i18n catalogue
  (`docs/i18n.md`) rather than hard-coding text.

## 6. Reporting issues / security

* Bugs: open an issue with steps to reproduce, device/ABI, JRE version and a
  logcat snippet.
* Crash reports: include the Rust `CrashReporter` output and the JVM stack
  (see `launch/crash.rs` and `docs/launch.md`).
* Security-sensitive findings: please use private disclosure rather than a
  public issue.

## 7. Licence

Contributions are accepted under the project licence (`GPL-3.0-or-later`, see
`Cargo.toml` / `LICENSE`). By submitting a PR you agree your contribution is
licensed likewise.

---

See also: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md),
[`docs/BUILD.md`](docs/BUILD.md), [`docs/MODULES.md`](docs/MODULES.md).
