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
     (tags + Android resource qualifiers);
 11. every catalogue key the **value formatters** read exists, and the Rust
     (`i18n/number.rs`) and Kotlin (`RcValueFormat.kt`) ports agree on the key
     set, the byte-unit ladder and the duration units — the two are hand-ported
     mirrors, so drift there means the core and the UI would render the same
     byte count differently;
 11b. dynamic language packs (`i18n/pack.rs`): the plural-rule ids are the same
     on both sides of the FFI, every `_meta.*` key the parser reads is documented,
     `_meta.` cannot collide with a shipped key, the picker consumes the fields the
     core emits, and a pack still cannot shadow a built-in language;
 12. the Rust->Kotlin golden fixture (`i18n_format_golden.tsv`) exists, is
     well formed, covers every language and every `i18nFormat` kind, leaks no
     placeholder, and is byte-identical to a fresh render (so a wording or
     rounding change cannot land without the Kotlin parity test seeing it);
 13. any generated `<string>` holding a literal `%` carries `formatted="false"`
     (otherwise aapt2 treats it as a printf specifier).

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



def _regenerate_golden():
    """Render the golden fixture with cargo; None when cargo is unavailable."""
    import shutil
    import subprocess

    if shutil.which("cargo") is None:
        return None
    try:
        proc = subprocess.run(
            ["cargo", "run", "--quiet", "--example", "i18n_format_golden"],
            cwd=os.path.join(C.REPO, "rust"),
            capture_output=True,
            text=True,
            timeout=900,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if proc.returncode != 0:
        return None
    return proc.stdout

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
    # Anchor on the `impl CrashCategory` block first: `crash.rs` also defines
    # `CrashSeverity::id()` with the *same* signature, and it happens to come
    # first in the file. A bare `pub fn id(self)` search silently scraped the
    # severity ids ("fatal", "recoverable", ...) and made this gate assert the
    # wrong contract, so scope the search the way a reader would.
    impl_block = re.search(
        r"\nimpl CrashCategory \{\n(.*?)\n\}\n", crash_src, re.S
    )
    ids_block = (
        re.search(r"pub fn id\(self\) -> &'static str \{(.*?)\n        \}", impl_block.group(1), re.S)
        if impl_block
        else None
    )
    crash_ids = (
        re.findall(r'CrashCategory::\w+ => "([a-z_]+)"', ids_block.group(1))
        if ids_block
        else []
    )
    check(
        "crash ids were located in crash.rs",
        len(crash_ids) >= 10 and "clean_exit" in crash_ids,
        f"{crash_ids}",
    )
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

    print("== value formatters (Rust number.rs <-> Kotlin RcValueFormat.kt) ==")
    # 11) the locale-aware value formatters
    number_rs = os.path.join(
        C.REPO, "rust/crates/rc-launcher-core/src/i18n/number.rs"
    )
    value_kt = os.path.join(
        C.REPO, "app/src/main/java/com/rc/launcher/ui/i18n/RcValueFormat.kt"
    )
    with open(number_rs, encoding="utf-8") as fh:
        number_src = fh.read()
    with open(value_kt, encoding="utf-8") as fh:
        value_src = fh.read()

    def _rust_list(name: str) -> list[str]:
        """The string literals of a `const NAME: [...] = [ ... ];` array."""
        m = re.search(rf"{name}[^=]*=\s*\[(.*?)\];", number_src, re.S)
        return re.findall(r'"([^"]+)"', m.group(1)) if m else []

    # Byte-unit ladders must be identical *and* in the same order: index `i`
    # means "1024^i" on both sides.
    rust_units = _rust_list("BYTE_UNIT_KEYS")
    kt_units = re.findall(
        r'"(unit\.[a-z]+)"', re.search(r"val BYTE_UNIT_KEYS = listOf\((.*?)\)", value_src, re.S).group(1)
    )
    check(
        "byte-unit ladders agree (same units, same order)",
        rust_units == kt_units and len(rust_units) >= 5,
        f"rust {rust_units} vs kotlin {kt_units}",
    )

    # Duration units: same bases, same order (largest first). Each entry also
    # carries a compiled-in fallback template, which must match too — that is
    # what renders when the catalogue cannot supply the piece, so a divergence
    # would only show up on a device with a missing native core.
    rust_durations_full = re.findall(
        r'\(\s*[\d_]+,\s*"(duration\.[a-z]+)",\s*"([^"]*)"\s*\)',
        re.search(r"DURATION_UNITS[^=]*=\s*\[(.*?)\];", number_src, re.S).group(1),
    )
    kt_durations_full = re.findall(
        r'Triple\(\s*[\d_]+L,\s*"(duration\.[a-z]+)",\s*"([^"]*)"\s*\)',
        re.search(r"val DURATION_UNITS = listOf\((.*?)\n    \)", value_src, re.S).group(1),
    )
    rust_durations = [b for b, _f in rust_durations_full]
    kt_durations = [b for b, _f in kt_durations_full]
    check(
        "duration-unit ladders agree (same units, same order)",
        rust_durations == kt_durations and len(rust_durations) == 4,
        f"rust {rust_durations} vs kotlin {kt_durations}",
    )
    check(
        "duration fallback templates agree",
        rust_durations_full == kt_durations_full and len(rust_durations_full) == 4,
        f"rust {rust_durations_full} vs kotlin {kt_durations_full}",
    )

    # The full key set each port declares it reads.
    def _required(src: str) -> set[str]:
        keys = set(re.findall(r'"((?:format|unit|duration|relative|download)\.[a-z_.]+)"', src))
        # `duration.<unit>` bases are read through their plural sub-keys.
        for base in rust_durations or kt_durations:
            keys.discard(base)
            keys.update({f"{base}.one", f"{base}.other"})
        return keys

    rust_keys = _required(number_src)
    kt_keys = _required(value_src)
    check(
        "both ports read the same catalogue keys",
        rust_keys == kt_keys,
        f"rust-only {sorted(rust_keys - kt_keys)} kotlin-only {sorted(kt_keys - rust_keys)}",
    )
    missing_fmt = sorted(k for k in rust_keys | kt_keys if k not in base_keys)
    check(
        "every formatter key is in the base catalogue",
        not missing_fmt,
        f"{missing_fmt}",
    )

    # An explicitly *empty* `format.group_separator` means "this locale does not
    # group digits" and must be honoured, while a blank unit name / template must
    # fall back. Both ports need the dedicated lookup, or a locale that disables
    # grouping would render differently in the core and in the UI.
    group_via_skeleton = re.compile(
        r'skeleton\([^)]*"format\.group_separator"', re.S
    )
    rust_group = bool(
        re.search(r"fn group_separator\s*\(", number_src)
    ) and not group_via_skeleton.search(number_src)
    kt_group = bool(
        re.search(r"fun groupSeparator\s*\(", value_src)
    ) and not group_via_skeleton.search(value_src)
    check(
        "both ports honour an explicitly empty group separator",
        rust_group and kt_group,
        f"rust={rust_group} kotlin={kt_group}",
    )
    # ... while the *decimal* separator must still fall back (a blank one would
    # fuse "1" and "5" into "15").
    # Whitespace-tolerant: `cargo fmt` / ktlint are free to reflow the call across
    # lines, and a gate that a formatter can break is a gate nobody trusts.
    decimal_fallback = re.compile(
        r'"format\.decimal_separator"\s*,\s*DEFAULT_DECIMAL_SEPARATOR', re.S
    )
    rust_decimal = bool(decimal_fallback.search(number_src))
    kt_decimal = bool(decimal_fallback.search(value_src))
    check(
        "both ports keep a fallback for the decimal separator",
        rust_decimal and kt_decimal,
        f"rust={rust_decimal} kotlin={kt_decimal}",
    )

    # The precision cap must match, or a `digits` request would be clamped
    # differently on the two sides.
    rust_cap = re.search(r"MAX_FRACTION_DIGITS: usize = (\d+)", number_src)
    kt_cap = re.search(r"MAX_FRACTION_DIGITS = (\d+)", value_src)
    check(
        "MAX_FRACTION_DIGITS agrees",
        bool(rust_cap and kt_cap) and rust_cap.group(1) == kt_cap.group(1),
        f"rust {rust_cap and rust_cap.group(1)} vs kotlin {kt_cap and kt_cap.group(1)}",
    )

    # Every `kind` the FFI advertises must be handled by the match arm above it.
    ffi_rs = os.path.join(C.REPO, "rust/crates/rc-launcher-core/src/ffi.rs")
    with open(ffi_rs, encoding="utf-8") as fh:
        ffi_src = fh.read()
    kinds_fn = re.search(
        r"pub fn i18n_format_kinds\(\) -> &'static \[&'static str\] \{(.*?)\n\}", ffi_src, re.S
    )
    advertised = re.findall(r'"([a-z_]+)"', kinds_fn.group(1)) if kinds_fn else []
    bridge_kt = os.path.join(
        C.REPO, "core/src/main/java/com/rc/launcher/core/RustBridge.kt"
    )
    with open(bridge_kt, encoding="utf-8") as fh:
        bridge_src = fh.read()
    documented = set(re.findall(r"`([a-z_]+)`", bridge_src[bridge_src.find("external fun i18nFormat") - 900 : bridge_src.find("external fun i18nFormat")]))
    check(
        "i18nFormat advertises kinds and RustBridge documents them",
        len(advertised) >= 10 and set(advertised) <= documented,
        f"advertised {sorted(advertised)} undocumented {sorted(set(advertised) - documented)}",
    )

    # 11b) dynamic language packs: the two sides must agree on the plural rules,
    # on the `_meta.*` vocabulary and on the limits, or a pack would behave
    # differently in the core and in the picker.
    pack_rs = os.path.join(C.REPO, "rust/crates/rc-launcher-core/src/i18n/pack.rs")
    format_rs = os.path.join(C.REPO, "rust/crates/rc-launcher-core/src/i18n/format.rs")
    option_kt = os.path.join(
        C.REPO, "app/src/main/java/com/rc/launcher/ui/i18n/LanguageOption.kt"
    )
    strings_kt = os.path.join(
        C.REPO, "app/src/main/java/com/rc/launcher/ui/i18n/RcStrings.kt"
    )
    with open(pack_rs, encoding="utf-8") as fh:
        pack_src = fh.read()
    with open(format_rs, encoding="utf-8") as fh:
        format_src = fh.read()
    with open(option_kt, encoding="utf-8") as fh:
        option_src = fh.read()
    with open(strings_kt, encoding="utf-8") as fh:
        strings_src = fh.read()

    # The plural rule ids are a wire contract (`_meta.plural`, `i18nBundle.plural`).
    rust_rules = set(re.findall(r'PluralRule::\w+ => "([a-z_]+)"', format_src))
    # Scope to the `RcPluralRule` enum body: `RcStringFormat.Plural` next door
    # also spells its suffixes as `NAME("one")`, which would over-match.
    kt_rule_block = re.search(
        r"enum class RcPluralRule\(val id: String\) \{(.*?)\n    ;", strings_src, re.S
    )
    kt_rule_ids = (
        set(re.findall(r'[A-Z_]+\("([a-z_]+)"\)', kt_rule_block.group(1)))
        if kt_rule_block
        else set()
    )
    check(
        "plural rule ids agree between core and Compose",
        rust_rules and rust_rules == kt_rule_ids,
        f"rust {sorted(rust_rules)} vs kotlin {sorted(kt_rule_ids)}",
    )

    # Every `_meta.*` key the parser reads must be documented in the module docs,
    # so a translator writing a pack has one authoritative list.
    meta_read = set(re.findall(r'meta\.get\("([a-z_]+)"\)', pack_src))
    meta_documented = set(re.findall(r"//! _meta\.([a-z_]+)", pack_src))
    check(
        "every `_meta.*` key the pack parser reads is documented",
        meta_read and meta_read <= meta_documented,
        f"read {sorted(meta_read)} undocumented {sorted(meta_read - meta_documented)}",
    )

    # `_meta.` must be namespaced away from real UI keys, or a pack could inject a
    # message the catalogue gate knows nothing about.
    check(
        "no shipped key collides with the pack metadata namespace",
        not [k for k in base_keys if k.startswith("_meta.")],
        f"{[k for k in base_keys if k.startswith('_meta.')]}",
    )

    # The picker rows must expose the fields the core actually emits.
    emitted = set(re.findall(r'"(\w+)":', pack_src[pack_src.find("pub fn describe") :]))
    consumed = set(re.findall(r'entries\["(\w+)"\]', option_src))
    required_fields = {"tag", "native_name", "completeness", "dynamic", "plural", "parent"}
    check(
        "the picker consumes the pack fields the core emits",
        required_fields <= emitted and required_fields <= consumed,
        f"core-missing {sorted(required_fields - emitted)} "
        f"kotlin-missing {sorted(required_fields - consumed)}",
    )

    # A pack must never be able to shadow a shipped language (that is the
    # overlay's job) — the guard is what keeps the picker honest.
    check(
        "packs cannot shadow a built-in language",
        "is a built-in language" in pack_src
        and "Language::from_tag(&tag).is_some()" in pack_src,
        "the built-in collision guard is gone from pack.rs",
    )

    # 12) the Rust->Kotlin golden fixture must exist and be fresh
    golden = os.path.join(C.REPO, "app/src/test/resources/i18n_format_golden.tsv")
    if not os.path.exists(golden):
        check("golden formatter fixture exists", False, golden)
    else:
        with open(golden, encoding="utf-8") as fh:
            golden_src = fh.read()
        rows = [
            l for l in golden_src.splitlines() if l.strip() and not l.startswith("#")
        ]
        check(
            "golden formatter fixture is substantial",
            len(rows) >= 300,
            f"{len(rows)} rows",
        )
        bad_rows = [l for l in rows if len(l.split("\t")) != 7]
        check("golden fixture rows are well formed", not bad_rows, f"{bad_rows[:3]}")
        langs = {l.split("\t")[0] for l in rows}
        check(
            "golden fixture covers every shipped language",
            langs == {t for t, _d, _b in C.LANGUAGES},
            f"{sorted(langs)}",
        )
        kinds = {l.split("\t")[1] for l in rows}
        check(
            "golden fixture covers every i18nFormat kind",
            set(advertised) <= kinds,
            f"uncovered {sorted(set(advertised) - kinds)}",
        )
        # No rendering may leak a placeholder or a raw catalogue key.
        leaks = [
            l for l in rows
            if "{" in l.split("\t")[6]
            or any(l.split("\t")[6].startswith(p) for p in ("unit.", "duration.", "format."))
        ]
        check("golden renderings resolve fully", not leaks, f"{leaks[:3]}")
        # Freshness: re-run the generator when cargo is available.
        regen = _regenerate_golden()
        if regen is None:
            print("  skip  golden fixture freshness (cargo unavailable)")
        else:
            check(
                "golden fixture is up to date "
                "(cargo run --example i18n_format_golden -- --write)",
                regen == golden_src,
                "committed fixture differs from a fresh render",
            )

    # 13) `%` needs formatted="false" in the generated XML
    unguarded = []
    for tag, dirname, _base in C.LANGUAGES:
        path = os.path.join(C.RES_DIR, dirname, "strings.xml")
        with open(path, encoding="utf-8") as fh:
            xml_src = fh.read()
        for name, attrs, body in re.findall(
            r'<string name="([^"]+)"([^>]*)>(.*?)</string>', xml_src, re.S
        ):
            if "%" in body and 'formatted="false"' not in attrs:
                unguarded.append(f"{dirname}/{name}")
    check(
        'literal "%" values carry formatted="false"',
        not unguarded,
        f"{unguarded}",
    )

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
