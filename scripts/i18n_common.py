"""Shared helpers for the i18n tooling (task 20).

Parses the *same* `.properties` subset the Rust core does
(`rust/crates/rc-launcher-core/src/i18n/catalog.rs`): the first unescaped `=`
separates, `#`/`!` are comments, a trailing `\\` continues the line and
`\\n \\r \\t \\f \\\\ \\= \\: \\uXXXX` are escapes.

Keeping one parser here (rather than a second dialect) is what lets
`check_i18n.py` prove the Android resources and the Rust catalogues agree.
"""

from __future__ import annotations

import os
import re

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROPS_DIR = os.path.join(REPO, "rust", "crates", "rc-launcher-core", "i18n")
RES_DIR = os.path.join(REPO, "app", "src", "main", "res")

# Language tag -> (Android values-* directory, is base locale)
# Chinese-first: zh-CN owns the default `values/` directory.
LANGUAGES = [
    ("zh-CN", "values", True),
    ("zh-Hant", "values-zh-rTW", False),
    ("en", "values-en", False),
]

BASE_TAG = "zh-CN"


def _unescape(s: str) -> str:
    out = []
    i = 0
    while i < len(s):
        c = s[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        i += 1
        if i >= len(s):
            out.append("\\")
            break
        n = s[i]
        if n == "n":
            out.append("\n")
        elif n == "r":
            out.append("\r")
        elif n == "t":
            out.append("\t")
        elif n == "f":
            out.append("\f")
        elif n == "u":
            hex4 = s[i + 1 : i + 5]
            if len(hex4) == 4 and all(ch in "0123456789abcdefABCDEF" for ch in hex4):
                out.append(chr(int(hex4, 16)))
                i += 4
            else:
                out.append("\\u")
        else:
            out.append(n)
        i += 1
    return "".join(out)


def _split_entry(line: str):
    """Split at the first *unescaped* `=`."""
    escaped = False
    for i, c in enumerate(line):
        if escaped:
            escaped = False
            continue
        if c == "\\":
            escaped = True
        elif c == "=":
            return line[:i], line[i + 1 :]
    return None


def parse_properties(path: str):
    """Return (entries: dict, problems: list[str]) for a `.properties` file."""
    entries: dict[str, str] = {}
    problems: list[str] = []
    with open(path, encoding="utf-8") as fh:
        text = fh.read()
    if text.startswith("\ufeff"):
        text = text[1:]

    logical = ""
    start = 0
    pending = False
    for n, raw in enumerate(text.splitlines(), start=1):
        line = raw[:-1] if raw.endswith("\r") else raw
        if not pending:
            t = line.lstrip()
            if not t or t.startswith("#") or t.startswith("!"):
                continue
            logical = t
            start = n
        else:
            logical += line.lstrip()
        trailing = len(logical) - len(logical.rstrip("\\"))
        if trailing % 2 == 1:
            logical = logical[:-1]
            pending = True
            continue
        pending = False
        parts = _split_entry(logical)
        if parts is None:
            problems.append(f"{os.path.basename(path)}:{start}: no `=` separator")
            continue
        key = _unescape(parts[0]).strip()
        if not key:
            problems.append(f"{os.path.basename(path)}:{start}: empty key")
            continue
        if key in entries:
            problems.append(f"{os.path.basename(path)}:{start}: duplicate key `{key}`")
        entries[key] = _unescape(parts[1].strip())
    if pending:
        problems.append(f"{os.path.basename(path)}: dangling `\\` continuation at EOF")
    return entries, problems


def load_all():
    """tag -> (entries, problems) for every shipped catalogue."""
    out = {}
    for tag, _dirname, _base in LANGUAGES:
        path = os.path.join(PROPS_DIR, f"{tag}.properties")
        out[tag] = parse_properties(path)
    return out


PLACEHOLDER_RE = re.compile(r"(?<!\{)\{([^{}]+)\}")


def placeholders(value: str) -> set[str]:
    """The `{name}` placeholder set, ignoring `{{escaped}}` braces."""
    # Mirror the Rust implementation: drop `{{`/`}}` first, then scan.
    stripped = value.replace("{{", "\x00").replace("}}", "\x01")
    return {m.group(1) for m in PLACEHOLDER_RE.finditer(stripped) if m.group(1).strip()}


def android_name(key: str) -> str:
    """`nav.home` -> `nav_home` (Android resource names allow no dots)."""
    return key.replace(".", "_").replace("-", "_")
