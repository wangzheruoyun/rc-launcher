#!/usr/bin/env python3
"""Generate / verify the JRE prebuilt manifest for com.rc.launcher (task 6).

FCL ships each Android OpenJDK build under
``assets/app_runtime/java/jre<major>/`` as a shared ``universal.tar.xz`` plus one
``bin-<abi>.tar.xz`` slice per ABI and a ``version`` file (FCL build number).

This script scans that directory, computes the SHA-1 + size of every archive
(and reads the build number), and emits ``jre_manifest.json``. Run it after
updating the prebuilt binaries, or with ``--check`` in CI to fail when the
committed manifest no longer matches the actual binaries.

Usage:
    python3 generate_jre_manifest.py            # (re)write jre_manifest.json
    python3 generate_jre_manifest.py --check    # exit 1 if it would change
"""
from __future__ import annotations

import hashlib
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
JAVA_DIR = os.path.join(HERE, "src", "main", "assets", "app_runtime", "java")
MANIFEST = os.path.join(JAVA_DIR, "jre_manifest.json")

JRE_DIRS = {
    "jre8": 8,
    "jre17": 17,
    "jre21": 21,
    "jre25": 25,
}

# FCL bin-<suffix>.tar.xz -> Android ABI.
ABI_SUFFIX = {
    "arm64": "arm64-v8a",
    "arm": "armeabi-v7a",
    "x86": "x86",
    "x86_64": "x86_64",
}


def sha1_of(path: str) -> tuple[str, int]:
    h = hashlib.sha1()
    size = 0
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
            size += len(chunk)
    return h.hexdigest(), size


def scan() -> dict:
    versions = []
    for dname, major in JRE_DIRS.items():
        d = os.path.join(JAVA_DIR, dname)
        if not os.path.isdir(d):
            continue
        # build number from `version`
        build = 0
        vfile = os.path.join(d, "version")
        if os.path.isfile(vfile):
            with open(vfile) as f:
                build = int(f.read().strip() or 0)
        archives = []
        uni = os.path.join(d, "universal.tar.xz")
        if os.path.isfile(uni):
            sha, size = sha1_of(uni)
            archives.append({
                "kind": "universal",
                "abi": None,
                "file": "universal.tar.xz",
                "sha1": sha,
                "size": size,
            })
        for fn in sorted(os.listdir(d)):
            if not fn.startswith("bin-") or not fn.endswith(".tar.xz"):
                continue
            suffix = fn[len("bin-"):-len(".tar.xz")]
            abi = ABI_SUFFIX.get(suffix)
            if abi is None:
                continue
            sha, size = sha1_of(os.path.join(d, fn))
            archives.append({
                "kind": "bin",
                "abi": abi,
                "file": fn,
                "sha1": sha,
                "size": size,
            })
        if not archives:
            raise SystemExit(f"no tar.xz archives found under {d}")
        versions.append({
            "java_version": dname,
            "major": major,
            "build": build,
            "archives": archives,
        })
    versions.sort(key=lambda v: v["major"])
    return {
        "schema_version": 1,
        "source": "FCL-release-1.3.2.7-arm64-v8a.apk assets/app_runtime/java",
        "generated_at": "",
        "versions": versions,
    }


def main() -> int:
    manifest = scan()
    text = json.dumps(manifest, indent=2, ensure_ascii=False) + "\n"
    if "--check" in sys.argv:
        if not os.path.isfile(MANIFEST):
            print("jre_manifest.json is missing", file=sys.stderr)
            return 1
        with open(MANIFEST) as f:
            existing = f.read()
        if existing != text:
            print("jre_manifest.json is out of date; regenerate it", file=sys.stderr)
            return 1
        print("jre_manifest.json is up to date")
        return 0
    with open(MANIFEST, "w") as f:
        f.write(text)
    print(f"wrote {MANIFEST} ({len(manifest['versions'])} versions)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
