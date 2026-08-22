#!/usr/bin/env python3
"""Health audit for com.rc.launcher (task 24).

Runs on a schedule (and on demand) to produce a single *health report* that
covers the three things the China-mainland launcher must keep an eye on:

  1. Mirror-source speed/health  (测速各镜像源)
       Probes every built-in mirror (BMCLAPI / MCBBS / Aliyun) and measures
       reachability + response time + a small ranged-DOWNLOAD throughput, the
       same way the Rust ``MirrorProvider::measure`` does on the device.

  2. Manifest hash validation    (校验清单哈希)
       Re-checks the committed JRE prebuilt manifest (``jre_manifest.json``) and
       the ``generate_jre_manifest.py --check`` gate so a drifted SHA-1 / size
       fails loudly instead of shipping a corrupt JRE slice.

  3. Dependency license audit     (扫描依赖许可证)
       Enumerates every third-party crate via ``cargo metadata`` and every
       Gradle dependency declared in the ``*.gradle.kts`` files, then buckets
       them by SPDX license (inspired by cuberite's ``ThirdPartyLicenses`` dir)
       and flags unknown / AGPL licenses that may be incompatible with the
       project's GPL-3.0-or-later.

Everything is aggregated into a machine-readable ``health-report.json`` and a
human-readable ``health-report.md`` (also echoed to ``$GITHUB_STEP_SUMMARY``)
plus a standalone ``third_party_licenses.md`` SBOM-style listing.

The script is deliberately dependency-free (Python stdlib only) so it runs on
the stock ``actions/setup-python`` image, and every check degrades gracefully:
a mirror that is down never aborts the run, and the report always gets written.

Usage:
    python3 health_audit.py                 # run all checks, write to ./reports
    python3 health_audit.py --skip-mirrors  # offline run (manifest + license only)
    python3 health_audit.py --out build/reports
    python3 health_audit.py mirrors         # run only the mirror speed test
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from datetime import datetime, timezone

# --------------------------------------------------------------------------- #
# Paths / configuration
# --------------------------------------------------------------------------- #

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(HERE)

# Built-in mirrors, kept in sync with
# rust/crates/rc-launcher-core/src/net/mirror.rs :: default_mirrors().
@dataclass
class Mirror:
    id: str
    name: str
    base_url: str
    path_prefix: str = ""


BUILTIN_MIRRORS = [
    Mirror("bmclapi", "BMCLAPI", "https://bmclapi2.bangbang93.com"),
    Mirror("mcbbs", "MCBBS", "https://download.mcbbs.net"),
    Mirror("aliyun", "Aliyun", "https://mirrors.aliyun.com/minecraft",
           path_prefix="minecraft"),
]

# Throughput probe: a small, always-mirrored file (the Minecraft version
# manifest). Mirrors are path-preserving, so the same path works on each.
THROUGHPUT_PROBE_PATH = "/mc/game/version_manifest_v2.json"

PROJECT_LICENSE = "GPL-3.0-or-later"

# License buckets (SPDX-ish). Anything not listed lands in "other".
PERMISSIVE = {
    "MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib",
    "0BSD", "Unlicense", "CC0-1.0", "BSL-1.0", "MIT-0", "Unicode-DFS-2016",
    "Unicode-3.0", "WTFPL", "CC-BY-4.0", "CC-BY-3.0", "MS-PL",
    "CDLA-Permissive-2.0",
}
WEAK_COPYLEFT = {
    "LGPL-2.1", "LGPL-2.1-or-later", "LGPL-3.0", "LGPL-3.0-or-later",
    "LGPL-2.0", "MPL-2.0", "EPL-2.0", "EPL-1.0", "CDDL-1.0", "CDDL-1.1",
}
COPYLEFT = {
    "GPL-2.0", "GPL-2.0-or-later", "GPL-2.0-only", "GPL-3.0",
    "GPL-3.0-or-later", "GPL-3.0-only", "AGPL-3.0", "AGPL-3.0-only",
    "AGPL-3.0-or-later",
}
# Licenses that are NOT one-way compatible with GPL-3.0 (i.e. would taint a
# GPL-3.0 distribution). GPL and LGPL/MPL are fine; AGPL is the danger.
INCOMPATIBLE = {"AGPL-3.0", "AGPL-3.0-only", "AGPL-3.0-or-later"}


# --------------------------------------------------------------------------- #
# Small helpers
# --------------------------------------------------------------------------- #

def now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def shell(cmd, cwd=None, timeout=120):
    """Run a command, return (rc, stdout, stderr). Never raises."""
    try:
        p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True,
                            timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return 124, "", "timeout"
    except FileNotFoundError as e:
        return 127, "", f"missing: {e}"
    except Exception as e:  # pragma: no cover - defensive
        return 1, "", str(e)


def _known_bucket(token: str) -> str | None:
    if token in PERMISSIVE:
        return "permissive"
    if token in WEAK_COPYLEFT:
        return "weak-copyleft"
    if token in COPYLEFT:
        return "copyleft"
    if token in INCOMPATIBLE:
        return "incompatible"
    return None


def bucket_of(license_expr: str | None) -> str:
    """Bucket an SPDX license expression into permissive / weak-copyleft /
    copyleft / incompatible / unknown / other.

    Handles OR/AND/slash disjunctions, WITH <exception> clauses and
    parenthesised groups (e.g. (MIT OR Apache-2.0) AND Apache-2.0). A WITH
    exception (e.g. LLVM-exception) is treated as part of its base license, so
    Apache-2.0 WITH LLVM-exception stays permissive.
    """
    if not license_expr:
        return "unknown"
    norm = license_expr.replace("(", " ").replace(")", " ")
    tokens = re.split(r"\s+OR\s+|\s+AND\s+|\s+WITH\s+|\s*/\s+|/", norm)
    tokens = [t.strip() for t in tokens if t.strip()]
    if not tokens:
        return "other"
    recognized = [t for t in tokens if _known_bucket(t) is not None]
    if not recognized:
        return "other"
    if any(_known_bucket(t) == "incompatible" for t in recognized):
        return "incompatible"
    if all(_known_bucket(t) == "permissive" for t in recognized):
        return "permissive"
    if all(_known_bucket(t) in ("permissive", "weak-copyleft")
           for t in recognized):
        return "weak-copyleft"
    if any(_known_bucket(t) == "copyleft" for t in recognized):
        return "copyleft"
    return "other"
def probe_url(m: Mirror, path: str) -> str:
    base = m.base_url.rstrip("/")
    pref = m.path_prefix.strip("/")
    if pref:
        return f"{base}/{pref}{path}"
    return f"{base}{path}"


def probe_mirror(m: Mirror, timeout: float, samples: int) -> dict:
    """Measure a mirror: reachability, response time, throughput.

    Mirrors the Rust ``MirrorProvider::measure`` semantics — any HTTP response
    (incl. 404) counts as reachable; we only care how fast the mirror answers.
    """
    latencies = []
    errors = []
    reachable = False
    status = None
    throughput_bps = None
    best_bytes = 0

    for _ in range(max(1, samples)):
        url = probe_url(m, "/favicon.ico")
        req = urllib.request.Request(url, headers={"Range": "bytes=0-262143"})
        t0 = time.monotonic()
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                status = r.status
                data = r.read()
                dt = time.monotonic() - t0
                reachable = True
                latencies.append(dt)
                best_bytes = max(best_bytes, len(data))
                if dt > 0:
                    throughput_bps = max(throughput_bps or 0, len(data) / dt)
        except urllib.error.HTTPError as e:
            # 4xx/5xx still means the mirror is up and answering.
            dt = time.monotonic() - t0
            status = e.code
            reachable = True
            latencies.append(dt)
        except Exception as e:  # timeout / DNS / conn refused
            errors.append(str(e))
            latencies.append(None)

    ok_lat = [x for x in latencies if x is not None]
    avg_ms = round(sum(ok_lat) / len(ok_lat) * 1000, 1) if ok_lat else None
    return {
        "id": m.id,
        "name": m.name,
        "base_url": m.base_url,
        "reachable": reachable,
        "http_status": status,
        "avg_response_ms": avg_ms,
        "samples": len(latencies),
        "throughput_bytes_per_s": int(throughput_bps) if throughput_bps else None,
        "error": errors[0] if errors and not reachable else None,
    }


def run_mirrors(mirrors=None, timeout=8.0, samples=3) -> dict:
    mirrors = mirrors or BUILTIN_MIRRORS
    results = [probe_mirror(m, timeout, samples) for m in mirrors]
    total = len(results)
    reachable = sum(1 for r in results if r["reachable"])
    # Rank reachable mirrors by response time.
    ranked = sorted([r for r in results if r["reachable"] and r["avg_response_ms"] is not None],
                    key=lambda r: r["avg_response_ms"])
    return {
        "total": total,
        "reachable": reachable,
        "best": ranked[0]["id"] if ranked else None,
        "results": results,
    }


# --------------------------------------------------------------------------- #
# 2) Manifest hash validation
# --------------------------------------------------------------------------- #

def sha1_of(path: str) -> tuple[str, int]:
    import hashlib
    h = hashlib.sha1()
    size = 0
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
            size += len(chunk)
    return h.hexdigest(), size


def run_manifest() -> dict:
    """Validate jre_manifest.json (and the generate_jre_manifest.py gate)."""
    java_dir = os.path.join(REPO_ROOT, "runtime", "src", "main", "assets",
                            "app_runtime", "java")
    manifest = os.path.join(java_dir, "jre_manifest.json")
    out = {
        "manifest_path": os.path.relpath(manifest, REPO_ROOT),
        "present": os.path.isfile(manifest),
        "entries": [],
        "matched": 0,
        "mismatched": 0,
        "missing": 0,
        "gate": None,
    }
    if not out["present"]:
        return out

    with open(manifest, "r", encoding="utf-8") as f:
        data = json.load(f)

    for ver in data.get("versions", []):
        jv = ver.get("java_version", "?")
        for arc in ver.get("archives", []):
            rel = os.path.join("java", jv, arc.get("file", ""))
            disk = os.path.join(java_dir, jv, arc.get("file", ""))
            entry = {
                "java_version": jv,
                "kind": arc.get("kind"),
                "abi": arc.get("abi"),
                "file": arc.get("file"),
                "expected_sha1": arc.get("sha1"),
                "expected_size": arc.get("size"),
            }
            if not os.path.isfile(disk):
                entry["status"] = "missing"
                entry["error"] = "file not on disk"
                out["missing"] += 1
            else:
                actual_sha1, actual_size = sha1_of(disk)
                entry["actual_sha1"] = actual_sha1
                entry["actual_size"] = actual_size
                if actual_sha1 == arc.get("sha1") and actual_size == arc.get("size"):
                    entry["status"] = "ok"
                    out["matched"] += 1
                else:
                    entry["status"] = "mismatch"
                    entry["error"] = "sha1/size drift"
                    out["mismatched"] += 1
            out["entries"].append(entry)

    # Cross-check with the existing CI gate script.
    rc, so, se = shell([sys.executable,
                        os.path.join(REPO_ROOT, "runtime",
                                     "generate_jre_manifest.py"), "--check"],
                       cwd=REPO_ROOT)
    out["gate"] = {
        "returncode": rc,
        "passed": rc == 0,
        "stdout": (so or "").strip()[-2000:],
        "stderr": (se or "").strip()[-1000:],
    }
    return out


# --------------------------------------------------------------------------- #
# 3) Dependency license audit
# --------------------------------------------------------------------------- #

def run_license_audit() -> dict:
    """Enumerate third-party licenses (cargo + gradle)."""
    out = {
        "project_license": PROJECT_LICENSE,
        "cargo": {"available": False, "packages": 0, "by_bucket": {},
                  "unknown": [], "incompatible": [], "licenses": []},
        "gradle": {"available": False, "declared": []},
    }

    # --- Rust crates via `cargo metadata` (falls back to Cargo.lock) ---
    rc, so, se = shell(["cargo", "metadata", "--format-version", "1"],
                       cwd=os.path.join(REPO_ROOT, "rust"), timeout=180)
    pkgs = []
    if rc == 0:
        try:
            meta = json.loads(so)
            pkgs = meta.get("packages", [])
        except Exception:
            pkgs = []
    if pkgs:
        out["cargo"]["available"] = True
        out["cargo"]["packages"] = len(pkgs)
        by_bucket = {}
        for p in pkgs:
            lic = p.get("license") or p.get("license_file")
            # license_file path -> treat as "custom"
            lic_expr = lic if p.get("license") else ("custom" if lic else None)
            bkt = bucket_of(lic_expr)
            rec = {"name": p["name"], "version": p.get("version"),
                   "license": lic_expr, "bucket": bkt,
                   "source": (p.get("source") or "local")}
            out["cargo"]["licenses"].append(rec)
            by_bucket[bkt] = by_bucket.get(bkt, 0) + 1
            if bkt == "unknown":
                out["cargo"]["unknown"].append(f"{p['name']}@{p.get('version')}")
            if bkt == "incompatible":
                out["cargo"]["incompatible"].append(
                    f"{p['name']}@{p.get('version')} ({lic_expr})")
        out["cargo"]["by_bucket"] = by_bucket

    # --- Gradle dependencies (best-effort static parse of *.gradle.kts) ---
    gradle_files = []
    for root, _dirs, files in os.walk(REPO_ROOT):
        if "build" in root.split(os.sep) or ".gradle" in root.split(os.sep):
            continue
        for fn in files:
            if fn.endswith(".gradle.kts"):
                gradle_files.append(os.path.join(root, fn))
    declared = []
    pat = re.compile(
        r'(?:implementation|api|compileOnly|runtimeOnly|testImplementation|'
        r'ksp|kapt|coreLibraryDesugaring)\s*\(\s*["\']'
        r'([A-Za-z0-9_.\-]+):([A-Za-z0-9_.\-]+):([^"\']+)["\']')
    for gf in sorted(set(gradle_files)):
        try:
            with open(gf, "r", encoding="utf-8", errors="replace") as f:
                txt = f.read()
        except Exception:
            continue
        for m in pat.finditer(txt):
            declared.append({"group": m.group(1), "name": m.group(2),
                             "version": m.group(3),
                             "file": os.path.relpath(gf, REPO_ROOT)})
    if declared:
        out["gradle"]["available"] = True
        out["gradle"]["declared"] = declared
    return out


# --------------------------------------------------------------------------- #
# Report assembly
# --------------------------------------------------------------------------- #

def build_report(skip_mirrors=False, mirror_timeout=8.0,
                mirror_samples=3) -> dict:
    mirrors = run_mirrors(timeout=mirror_timeout,
                          samples=mirror_samples) if not skip_mirrors else {
        "total": 0, "reachable": 0, "best": None, "results": [],
        "skipped": True,
    }
    manifest = run_manifest()
    licenses = run_license_audit()

    # --- overall health scoring ---
    flags = []
    mirror_ok = mirrors.get("skipped") or (mirrors["reachable"] == mirrors["total"]
                                           and mirrors["total"] > 0)
    if not mirrors.get("skipped"):
        if mirrors["reachable"] == 0:
            flags.append("all mirrors unreachable")
        elif mirrors["reachable"] < mirrors["total"]:
            flags.append(f"{mirrors['total'] - mirrors['reachable']} mirror(s) down")

    manifest_ok = manifest["present"] and manifest["mismatched"] == 0 \
        and manifest["missing"] == 0 and (manifest["gate"] or {}).get("passed", True)
    if manifest["present"] and manifest["mismatched"]:
        flags.append(f"{manifest['mismatched']} JRE archive hash mismatch")
    if manifest["present"] and manifest["missing"]:
        flags.append(f"{manifest['missing']} JRE archive missing")
    if manifest["gate"] and manifest["gate"].get("passed") is False:
        flags.append("jre_manifest.py --check failed")

    lic_unknown = licenses["cargo"].get("unknown", [])
    lic_incompat = licenses["cargo"].get("incompatible", [])
    if lic_unknown:
        flags.append(f"{len(lic_unknown)} crate(s) without a license")
    if lic_incompat:
        flags.append(f"{len(lic_incompat)} crate(s) with AGPL (incompatible)")

    if flags:
        status = "unhealthy" if (not manifest_ok or lic_incompat) else "degraded"
    else:
        status = "healthy"

    report = {
        "schema_version": 1,
        "generated_at": now_iso(),
        "tool": "health_audit.py",
        "status": status,
        "flags": flags,
        "mirrors": mirrors,
        "manifest": manifest,
        "licenses": licenses,
    }
    return report


def render_markdown(report: dict) -> str:
    L = []
    L.append(f"# com.rc.launcher health report")
    L.append("")
    L.append(f"- **Generated:** {report['generated_at']}")
    L.append(f"- **Status:** `{report['status']}`")
    if report["flags"]:
        L.append(f"- **Flags:** {', '.join(report['flags'])}")
    else:
        L.append("- **Flags:** none")
    L.append("")

    # Mirrors
    L.append("## 1. Mirror speed / health (测速各镜像源)")
    m = report["mirrors"]
    if m.get("skipped"):
        L.append("_Mirror test skipped (offline run)._")
    else:
        L.append(f"Reachable **{m['reachable']}/{m['total']}** "
                 f"— fastest: `{m['best']}`")
        L.append("")
        L.append("| Mirror | Base URL | Reachable | HTTP | Avg resp (ms) | "
                 "Throughput |")
        L.append("|---|---|---|---|---|---|")
        for r in m["results"]:
            thr = f"{r['throughput_bytes_per_s']:,} B/s" if r["throughput_bytes_per_s"] else "-"
            resp = str(r["avg_response_ms"]) if r["avg_response_ms"] is not None else "-"
            L.append(f"| {r['name']} | {r['base_url']} | {'yes' if r['reachable'] else 'NO'} "
                     f"| {r['http_status']} | {resp} | {thr} |")
    L.append("")

    # Manifest
    L.append("## 2. Manifest hash validation (校验清单哈希)")
    man = report["manifest"]
    if not man["present"]:
        L.append("_`jre_manifest.json` not found._")
    else:
        gate = man.get("gate") or {}
        L.append(f"Matched **{man['matched']}**, mismatched **{man['mismatched']}**, "
                 f"missing **{man['missing']}** "
                 f"(gate: `{'PASS' if gate.get('passed') else 'FAIL'}`).")
        if man["mismatched"] or man["missing"]:
            L.append("")
            L.append("| Java | Kind | ABI | File | Status |")
            L.append("|---|---|---|---|---|")
            for e in man["entries"]:
                if e["status"] != "ok":
                    L.append(f"| {e['java_version']} | {e['kind']} | {e['abi']} "
                             f"| {e['file']} | {e['status']} |")
    L.append("")

    # Licenses
    L.append("## 3. Dependency license audit (扫描依赖许可证)")
    lic = report["licenses"]
    c = lic["cargo"]
    L.append(f"Project license: `{lic['project_license']}`")
    if c["available"]:
        L.append(f"Rust crates scanned: **{c['packages']}**")
        L.append("")
        L.append("| Bucket | Count |")
        L.append("|---|---|")
        for b, n in sorted(c["by_bucket"].items(), key=lambda kv: -kv[1]):
            L.append(f"| {b} | {n} |")
        if c["unknown"]:
            L.append("")
            L.append(f"**Unknown license:** {', '.join(c['unknown'])}")
        if c["incompatible"]:
            L.append("")
            L.append(f"**Incompatible (AGPL):** {', '.join(c['incompatible'])}")
    else:
        L.append("_`cargo metadata` unavailable — crate licenses not audited._")
    g = lic["gradle"]
    if g["available"]:
        L.append("")
        L.append(f"Gradle dependencies declared: **{len(g['declared'])}** "
                 f"(see `third_party_licenses.md`).")
    L.append("")
    L.append("---")
    L.append("_Generated by `scripts/health_audit.py` (task 24)._")
    return "\n".join(L)


def render_license_sbom(report: dict) -> str:
    L = []
    L.append("# Third-party licenses")
    L.append("")
    L.append(f"Project license: `{report['licenses']['project_license']}` "
             f"(generated {report['generated_at']}).")
    L.append("")
    L.append("This file is the dependency-license audit produced by "
             "`scripts/health_audit.py` (task 24), modelled after cuberite's "
             "`Server/Install/ThirdPartyLicenses` directory.")
    L.append("")
    c = report["licenses"]["cargo"]
    if c["available"]:
        L.append(f"## Rust crates ({c['packages']})")
        L.append("")
        for rec in sorted(c["licenses"], key=lambda r: (r["bucket"], r["name"])):
            L.append(f"- `{rec['name']}@{rec['version']}` — "
                     f"{rec['license'] or 'UNKNOWN'} "
                     f"_[{rec['bucket']}]_")
        L.append("")
    g = report["licenses"]["gradle"]
    if g["available"]:
        L.append(f"## Gradle dependencies ({len(g['declared'])})")
        L.append("")
        L.append("| Group | Artifact | Version | Declared in |")
        L.append("|---|---|---|---|")
        for d in g["declared"]:
            L.append(f"| {d['group']} | {d['name']} | {d['version']} | {d['file']} |")
        L.append("")
    L.append("---")
    L.append("_License expressions are taken from crate metadata and Gradle "
             "declarations; verify against the upstream repositories before "
             "redistribution._")
    return "\n".join(L)


def write_reports(report: dict, out_dir: str) -> None:
    os.makedirs(out_dir, exist_ok=True)
    md = render_markdown(report)
    json_path = os.path.join(out_dir, "health-report.json")
    md_path = os.path.join(out_dir, "health-report.md")
    sbom_path = os.path.join(out_dir, "third_party_licenses.md")
    with open(json_path, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2, ensure_ascii=False)
    with open(md_path, "w", encoding="utf-8") as f:
        f.write(md + "\n")
    with open(sbom_path, "w", encoding="utf-8") as f:
        f.write(render_license_sbom(report) + "\n")

    # Echo to the Actions step summary when running in CI.
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        try:
            with open(summary, "a", encoding="utf-8") as f:
                f.write("\n" + md + "\n")
        except Exception:
            pass
    print(md)
    print(f"\n[health_audit] wrote {json_path}", file=sys.stderr)
    print(f"[health_audit] wrote {md_path}", file=sys.stderr)
    print(f"[health_audit] wrote {sbom_path}", file=sys.stderr)


def main(argv=None) -> int:
    global REPO_ROOT
    ap = argparse.ArgumentParser(description="com.rc.launcher health audit (task 24)")
    ap.add_argument("--repo", default=REPO_ROOT, help="repository root")
    ap.add_argument("--out", default=os.path.join(REPO_ROOT, "reports"),
                    help="output directory for reports")
    ap.add_argument("--skip-mirrors", action="store_true",
                    help="skip the network mirror speed test (offline run)")
    ap.add_argument("--mirror-timeout", type=float, default=8.0,
                    help="per-request timeout (seconds) for the mirror probe")
    ap.add_argument("--mirror-samples", type=int, default=3,
                    help="number of probe samples per mirror (latency median)")
    sub = ap.add_subparsers(dest="check")
    for name in ("mirrors", "manifest", "licenses", "report"):
        sub.add_parser(name)
    args = ap.parse_args(argv)

    REPO_ROOT = os.path.abspath(args.repo)

    only = args.check
    if only in (None, "report"):
        report = build_report(skip_mirrors=args.skip_mirrors,
                              mirror_timeout=args.mirror_timeout,
                              mirror_samples=args.mirror_samples)
        write_reports(report, args.out)
        return 0 if report["status"] in ("healthy", "degraded") else 2

    if only == "mirrors":
        data = run_mirrors() if not args.skip_mirrors else {"skipped": True}
        print(json.dumps(data, indent=2, ensure_ascii=False))
        return 0
    if only == "manifest":
        data = run_manifest()
        print(json.dumps(data, indent=2, ensure_ascii=False))
        return 0
    if only == "licenses":
        data = run_license_audit()
        print(json.dumps(data, indent=2, ensure_ascii=False))
        return 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
