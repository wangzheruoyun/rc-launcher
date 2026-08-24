#!/usr/bin/env python3
"""Validate the whole i18n setup (task 20). Run it in CI and before committing.

Checks, in order:

  1. every `.properties` catalogue parses with **zero** problems;
  2. the base catalogue (zh-CN) is the superset — no other catalogue may have a
     key it lacks (that is always a typo), and none may be missing one
     (an untranslated string would fall back silently);
  3. no empty values, and no value left equal to its own key;
  4. the `{name}` placeholder set of every translation matches the base;
  5. the generated Android resources + `RcStringResources.kt` + `locales_config.xml`
     are byte-identical to what the generator produces now (no stale files);
  6. the generated `values*/strings.xml` round-trip back to the catalogue values
     (XML escaping is lossless);
  7. every key the Compose UI references (`RcStringKeys.required`) exists;
  8. every Rust `CrashCategory::id()` has `crash.<id>.summary` + `.advice`;
  9. every Rust `RcError::i18n_key()` target exists;
 10. the Kotlin `AppLanguage` enum agrees with the shipped catalogues
     (tags + Android resource qualifiers).

Usage:  python3 scripts/check_i18n.py
"""

from __future__ import annotations

import os
import re
import sys
import xml.etree.ElementTree as ET

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import i18n_common as C  # noqa: E402
import gen_android_strings as G  # noqa: E402

FAILURES: list[str] = []
CHECKS = 0


def fail(msg: str) -> None:
    FAILURES.append(msg)


def check(name: str, ok: bool, detail: str = "") -> None:
    global CHECKS
    CHECKS += 1
    if ok:
        print(f"  ok    {name}")
    else:
        print(f"  FAIL  {name}{(': ' + detail) if detail else ''}")
        fail(f"{name}{(': ' + detail) if detail else ''}")


def read_xml_strings(path: str) -> dict[str, str]:
    """name -> text for every <string> in an Android resource file."""
    tree = ET.parse(path)
    out: dict[str, str] = {}
    for node in tree.getroot().findall("string"):
        name = node.get("name")
        text = node.text or ""
        # Reverse generator Android escaping so the value matches the raw catalogue.
        # aapt decodes backslash-apostrophe to a plain apostrophe at build time; ElementTree
        # keeps the backslash (it is not a standard XML entity), so un-escape it here.
        text = text.replace("\\'", "'").replace('\\"', '"')
        # The generator quotes values with significant whitespace.
        if len(text) >= 2 and text.startswith('"') and text.endswith('"'):
            text = text[1:-1]
        # And escapes a leading @/? with a backslash.
        if text.startswith("\\@") or text.startswith("\\?"):
            text = text[1:]
        out[name] = text
    return out


def main() -> int:
    print("== i18n catalogues ==")
    catalogues = C.load_all()
    base_entries, base_problems = catalogues[C.BASE_TAG]
    base_keys = set(base_entries)

    # 1) parse cleanliness
    for tag, (entries, problems) in catalogues.items():
        check(f"{tag}.properties parses cleanly", not problems, "; ".join(problems))
        check(f"{tag}.properties is non-empty", len(entries) > 0)

    # 2) key parity against the Chinese-first base
    for tag, (entries, _p) in catalogues.items():
        keys = set(entries)
        missing = sorted(base_keys - keys)
        orphan = sorted(keys - base_keys)
        check(f"{tag} has every base key", not missing, f"missing {missing}")
        check(f"{tag} has no orphan keys", not orphan, f"orphan {orphan}")

    # 3) value sanity
    for tag, (entries, _p) in catalogues.items():
        empty = sorted(k for k, v in entries.items() if not v.strip())
        selfish = sorted(k for k, v in entries.items() if v.strip() == k)
        check(f"{tag} has no empty values", not empty, f"{empty}")
        check(f"{tag} has no value left as its key", not selfish, f"{selfish}")

    # 4) placeholder parity
    for tag, (entries, _p) in catalogues.items():
        if tag == C.BASE_TAG:
            continue
        drift = []
        for k, v in entries.items():
            if k not in base_entries:
                continue
            want = C.placeholders(base_entries[k])
            got = C.placeholders(v)
            if want != got:
                drift.append(f"{k}: expected {sorted(want)}, got {sorted(got)}")
        check(f"{tag} placeholders match the base", not drift, "; ".join(drift))

    print("== generated Android artefacts ==")
    # 5) freshness
    stale = []
    for path, content in G.targets().items():
        rel = os.path.relpath(path, C.REPO)
        if not os.path.exists(path):
            stale.append(f"{rel} (missing)")
            continue
        with open(path, encoding="utf-8") as fh:
            if fh.read() != content:
                stale.append(rel)
    check(
        "generated files are up to date",
        not stale,
        f"stale: {stale} — run python3 scripts/gen_android_strings.py",
    )

    # 6) XML round-trip
    for tag, dirname, _base in C.LANGUAGES:
        path = os.path.join(C.RES_DIR, dirname, "strings.xml")
        if not os.path.exists(path):
            check(f"{dirname}/strings.xml exists", False)
            continue
        xml = read_xml_strings(path)
        entries = catalogues[tag][0]
        expected = {C.android_name(k): v for k, v in entries.items()}
        check(
            f"{dirname}/strings.xml has every key",
            set(xml) == set(expected),
            f"missing {sorted(set(expected) - set(xml))}, extra {sorted(set(xml) - set(expected))}",
        )
        mismatch = [n for n in sorted(set(xml) & set(expected)) if xml[n] != expected[n]]
        check(
            f"{dirname}/strings.xml values round-trip",
            not mismatch,
            f"{[(n, xml[n], expected[n]) for n in mismatch[:3]]}",
        )

    print("== cross-language / cross-module consistency ==")
    # 7) the keys the Compose UI references
    keys_kt = os.path.join(
        C.REPO, "app/src/main/java/com/rc/launcher/ui/i18n/RcStringKeys.kt"
    )
    with open(keys_kt, encoding="utf-8") as fh:
        kt = fh.read()
    ui_keys = set(re.findall(r'const val [A-Z0-9_]+ = "([^"]+)"', kt))
    # DOWNLOAD_FILES is a plural *base* key: the catalogue holds `.one` / `.other`.
    plural_bases = {k for k in ui_keys if f"{k}.one" in base_keys}
    concrete = (ui_keys - plural_bases) | {
        f"{b}.{s}" for b in plural_bases for s in ("one", "other")
    }
    unknown = sorted(concrete - base_keys)
    check("every RcStringKeys constant exists", not unknown, f"{unknown}")
    check("plural base keys have one/other forms", bool(plural_bases))

    # 8) crash categories
    crash_rs = os.path.join(
        C.REPO, "rust/crates/rc-launcher-core/src/launch/crash.rs"
    )
    with open(crash_rs, encoding="utf-8") as fh:
        crash_src = fh.read()
    ids_block = re.search(r"pub fn id\(self\) -> &'static str \{(.*?)\n    \}", crash_src, re.S)
    crash_ids = re.findall(r'=> "([a-z_]+)"', ids_block.group(1)) if ids_block else []
    check("crash ids were located in crash.rs", len(crash_ids) >= 10, f"{crash_ids}")
    missing_crash = [
        f"crash.{i}.{kind}"
        for i in crash_ids
        for kind in ("summary", "advice")
        if f"crash.{i}.{kind}" not in base_keys
    ]
    check("every crash category has summary+advice", not missing_crash, f"{missing_crash}")

    # 9) error keys
    error_rs = os.path.join(C.REPO, "rust/crates/rc-launcher-core/src/error.rs")
    with open(error_rs, encoding="utf-8") as fh:
        error_src = fh.read()
    error_keys = set(re.findall(r'"(error\.[a-z_.]+)"', error_src))
    missing_err = sorted(k for k in error_keys if k not in base_keys)
    check("every RcError i18n key exists", not missing_err, f"{missing_err}")

    # 10) Kotlin AppLanguage <-> shipped catalogues
    lang_kt = os.path.join(
        C.REPO, "app/src/main/java/com/rc/launcher/ui/i18n/AppLanguage.kt"
    )
    with open(lang_kt, encoding="utf-8") as fh:
        lang_src = fh.read()
    # `re.findall` yields "" for a non-participating group, i.e. for `null`.
    declared = dict(
        (tag, qual if qual else None)
        for tag, qual in re.findall(
            r'\("(?!system)([^"]+)",\s*"[^"]*",\s*"[^"]*",\s*(?:"([^"]*)"|null)\)', lang_src
        )
    )
    expected_langs = {
        tag: (None if dirname == "values" else dirname[len("values-"):])
        for tag, dirname, _b in C.LANGUAGES
    }
    check(
        "AppLanguage tags match the shipped catalogues",
        set(declared) == set(expected_langs),
        f"kotlin {sorted(declared)} vs catalogues {sorted(expected_langs)}",
    )
    qual_drift = [
        f"{t}: kotlin {declared.get(t)} vs resources {expected_langs[t]}"
        for t in expected_langs
        if t in declared and declared[t] != expected_langs[t]
    ]
    check("AppLanguage Android qualifiers match", not qual_drift, "; ".join(qual_drift))

    print()
    if FAILURES:
        print(f"i18n check FAILED — {len(FAILURES)} of {CHECKS} checks failed:")
        for f in FAILURES:
            print(f"  - {f}")
        return 1
    print(f"i18n check passed — {CHECKS} checks, "
          f"{len(base_keys)} keys x {len(C.LANGUAGES)} languages.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
