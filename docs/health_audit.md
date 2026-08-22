# Health audit (task 24)

`scripts/health_audit.py` is the supply-chain health gate for com.rc.launcher. It
is driven by `.github/workflows/health.yml`, which runs **daily** (off-peak) and
on manual dispatch.

The audit covers the three things a China-mainland Minecraft launcher must keep an
eye on, and emits a single **health report** (`health-report.json` + `health-report.md`,
echoed to the Actions job summary) plus a standalone license SBOM
(`third_party_licenses.md`).

## 1. Mirror speed / health — 测速各镜像源

For every built-in mirror (BMCLAPI / MCBBS / Aliyun, kept in sync with
`rust/crates/rc-launcher-core/src/net/mirror.rs :: default_mirrors()`) the script
issues a small ranged `GET` and records:

* **reachability** — any HTTP response (incl. 404) counts as up, exactly like the
  Rust `MirrorProvider::measure` on-device;
* **response time** — average time-to-first-byte over a few samples;
* **throughput** — bytes/sec of the ranged download.

Tunables: `--mirror-timeout` (per-request seconds) and `--mirror-samples`
(samples per mirror). A mirror that is down never aborts the run.

## 2. Manifest hash validation — 校验清单哈希

Re-checks `runtime/src/main/assets/app_runtime/java/jre_manifest.json` against the
actual on-disk JRE slices: for each `universal.tar.xz` / `bin-<abi>.tar.xz` it
recomputes the SHA-1 + size and compares them to the committed manifest, and it
cross-runs `runtime/generate_jre_manifest.py --check` as a second gate. Any
mismatch or missing file is reported and drives the overall status to
`unhealthy`.

## 3. Dependency license audit — 扫描依赖许可证

Enumerates every third-party dependency and buckets it by SPDX license
(inspired by cuberite's `Server/Install/ThirdPartyLicenses` directory):

* **Rust** — `cargo metadata --format-version 1` over the workspace (the committed
  `Cargo.lock` means this works without re-resolving the index). Each crate's
  `license` expression is normalised (`OR`/`AND`/`/` disjunctions, `WITH
  <exception>` clauses, parenthesised groups) and bucketed into
  `permissive` / `weak-copyleft` / `copyleft` / `incompatible` (AGPL) / `unknown`
  / `other`.
* **Gradle** — the `implementation(...)` / `api(...)` / … declarations in every
  `*.gradle.kts` are statically scraped into a declared-dependency list.

The project is `GPL-3.0-or-later`; crates whose license resolves to `incompatible`
(AGPL) or `unknown` are flagged, because they can taint a GPL-3.0 distribution.

## Health status

| status | meaning |
| --- | --- |
| `healthy` | all mirrors reachable, manifest valid, no license flags |
| `degraded` | some mirrors down, but manifest + licenses OK |
| `unhealthy` | manifest drift / AGPL dependency / all mirrors down |

The script exits `0` for `healthy`/`degraded` and `2` for `unhealthy`; the workflow
therefore fails loudly (and opens/refreshes a tracking issue) only when something
is genuinely wrong.

## Running it locally

```bash
# full audit (needs network for the mirror probe)
python3 scripts/health_audit.py --out build/reports

# offline audit (manifest + license only)
python3 scripts/health_audit.py --skip-mirrors --out build/reports

# a single check
python3 scripts/health_audit.py manifest
python3 scripts/health_audit.py licenses
python3 scripts/health_audit.py mirrors
```

The report is also written to `$GITHUB_STEP_SUMMARY` when the script runs inside an
Actions job.
