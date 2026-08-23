#!/usr/bin/env python3
"""Generate the Android string resources from the Rust i18n catalogues (task 20).

The `.properties` files under `rust/crates/rc-launcher-core/i18n/` are the
**single source of truth** for every user-facing string. This script projects
them onto the artefacts Android needs:

  * `app/src/main/res/values/strings.xml`          (zh-CN — Chinese-first default)
  * `app/src/main/res/values-zh-rTW/strings.xml`   (zh-Hant)
  * `app/src/main/res/values-en/strings.xml`       (en)
  * `app/src/main/res/xml/locales_config.xml`      (Android 13 per-app language)
  * `app/src/main/java/com/rc/launcher/ui/i18n/RcStringResources.kt`
        (key -> R.string.* map: no reflection, survives resource shrinking)

Both the generated files and the sources are committed; `check_i18n.py` re-runs
the generator in-memory and fails when they differ, so a translator can never
land a catalogue change that the app has not picked up.

Usage:  python3 scripts/gen_android_strings.py [--check]
"""

from __future__ import annotations

import os
import sys
import xml.sax.saxutils as sax

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import i18n_common as C  # noqa: E402

HEADER = """<?xml version="1.0" encoding="utf-8"?>
<!--
  GENERATED FILE — DO NOT EDIT.

  Source of truth: rust/crates/rc-launcher-core/i18n/{tag}.properties
  Regenerate with: python3 scripts/gen_android_strings.py
  Verified in CI : python3 scripts/check_i18n.py

  {note}
-->
"""

NOTE_BASE = (
    "This is the launcher's DEFAULT catalogue (中文优先 / Chinese-first): Android\n"
    "  falls back here for every locale we do not ship, and the Rust core falls back\n"
    "  to the same zh-CN catalogue key-by-key."
)
NOTE_OTHER = "Translation of the zh-CN base catalogue; missing keys fall back to values/."


def escape(value: str) -> str:
    """XML-escape a message, keeping `{name}` placeholders verbatim.

    `@`/`?` are only special at the *start* of a resource value, and a leading /
    trailing space would be trimmed by aapt, so both are protected.
    """
    out = sax.escape(value).replace('"', "&quot;").replace("'", "\\'")
    if out.startswith(("@", "?")):
        out = "\\" + out
    if out != out.strip():
        # Preserve significant whitespace exactly as aapt expects.
        return '"' + out + '"'
    return out


def render_strings_xml(tag: str, entries: dict[str, str], is_base: bool) -> str:
    note = NOTE_BASE if is_base else NOTE_OTHER
    parts = [HEADER.format(tag=tag, note=note), "<resources>\n"]
    section = None
    for key in sorted(entries):
        top = key.split(".")[0]
        if top != section:
            if section is not None:
                parts.append("\n")
            parts.append(f"    <!-- {top} -->\n")
            section = top
        parts.append(
            f'    <string name="{C.android_name(key)}">{escape(entries[key])}</string>\n'
        )
    parts.append("</resources>\n")
    return "".join(parts)


def render_locales_config() -> str:
    """`android:localeConfig` — lets Android 13+ list the app in per-app language settings."""
    lines = [
        '<?xml version="1.0" encoding="utf-8"?>\n',
        "<!--\n",
        "  GENERATED FILE — DO NOT EDIT (python3 scripts/gen_android_strings.py).\n",
        "\n",
        "  Declares the languages the launcher ships so Android 13+ (API 33) shows RC\n",
        "  Launcher in Settings > System > Languages > App languages, and so\n",
        "  LocaleManager.applicationLocales can be persisted by the platform.\n",
        "-->\n",
        '<locale-config xmlns:android="http://schemas.android.com/apk/res/android">\n',
    ]
    for tag, dirname, is_base in C.LANGUAGES:
        # The platform wants BCP-47 tags; zh-Hant is advertised as zh-Hant-TW to
        # match the values-zh-rTW resource qualifier.
        android_tag = {"zh-CN": "zh-CN", "zh-Hant": "zh-Hant-TW", "en": "en"}[tag]
        lines.append(f'    <locale android:name="{android_tag}"/>\n')
    lines.append("</locale-config>\n")
    return "".join(lines)


def render_kotlin_map(keys: list[str]) -> str:
    body = "\n".join(
        f'        "{k}" to R.string.{C.android_name(k)},' for k in sorted(keys)
    )
    return f'''// GENERATED FILE — DO NOT EDIT.
//
// Source of truth: rust/crates/rc-launcher-core/i18n/zh-CN.properties
// Regenerate with: python3 scripts/gen_android_strings.py
// Verified in CI : python3 scripts/check_i18n.py
package com.rc.launcher.ui.i18n

import com.rc.launcher.R

/**
 * Maps an i18n **key** (`nav.home`) to its Android resource id
 * (`R.string.nav_home`), for task 20.
 *
 * Generated rather than resolved with `Resources.getIdentifier`, because
 * reflection-style resource lookup breaks under resource shrinking / R8 and
 * silently returns 0 instead of failing the build.
 *
 * This is the *fallback* path: [RcStrings] prefers the live catalogue handed over
 * by the Rust core ([com.rc.launcher.core.RustBridge.i18nBundle]) and only reads
 * Android resources when the native core is unavailable — which is exactly the
 * offline / degraded case task 19 requires the UI to survive.
 */
object RcStringResources {{
    /** Every key the generator projected into the `values-...` resource files. */
    val ids: Map<String, Int> = mapOf(
{body}
    )

    /** The resource id of [key], or `null` when the key is not a resource. */
    fun idOf(key: String): Int? = ids[key]

    /** Number of generated string resources — asserted by the unit tests. */
    val size: Int get() = ids.size
}}
'''


def targets() -> dict[str, str]:
    """Path -> desired content for every generated artefact."""
    catalogues = C.load_all()
    out: dict[str, str] = {}
    for tag, dirname, is_base in C.LANGUAGES:
        entries, problems = catalogues[tag]
        if problems:
            raise SystemExit(f"refusing to generate: {tag} has problems {problems}")
        out[os.path.join(C.RES_DIR, dirname, "strings.xml")] = render_strings_xml(
            tag, entries, is_base
        )
    out[os.path.join(C.RES_DIR, "xml", "locales_config.xml")] = render_locales_config()
    base_keys = list(catalogues[C.BASE_TAG][0])
    out[
        os.path.join(
            C.REPO,
            "app/src/main/java/com/rc/launcher/ui/i18n/RcStringResources.kt",
        )
    ] = render_kotlin_map(base_keys)
    return out


def main(argv: list[str]) -> int:
    check = "--check" in argv
    stale = []
    for path, content in targets().items():
        rel = os.path.relpath(path, C.REPO)
        existing = None
        if os.path.exists(path):
            with open(path, encoding="utf-8") as fh:
                existing = fh.read()
        if existing == content:
            if not check:
                print(f"  unchanged  {rel}")
            continue
        if check:
            stale.append(rel)
            continue
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(content)
        print(f"  {'updated' if existing else 'created'}    {rel}")
    if check and stale:
        print("STALE generated i18n files (run scripts/gen_android_strings.py):")
        for s in stale:
            print(f"  - {s}")
        return 1
    if check:
        print("i18n generated files are up to date.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
