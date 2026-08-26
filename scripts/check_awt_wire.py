#!/usr/bin/env python3
"""Validate the AWT bridge wire contract across Rust and Kotlin (task 18).

The AWT/Swing compatibility layer ("fakefx") is the one subsystem whose
correctness lives in *two* languages at once: the Rust core
(`launch::awt` / `launch::fakefx` / `launch::awt_host`) owns the transport, while
the Compose layer (`ui/awt/*.kt`) re-implements just enough of the same wire to
run a self-test, decode a Kotlin-owned transport and place the pointer overlay.

A constant that drifts between the two does not fail to compile and does not
fail a unit test on either side — it produces a silently black canvas, a cursor
that never changes, or a clipboard answer the JVM cannot parse. This script is
the guard rail, and it needs neither a Rust toolchain nor a JVM:

  1. frame / event wire constants agree (magic, version, header, record length,
     canvas bound, opaque black);
  2. control-plane constants agree (`RCAC` magic + version + header, text bound,
     the reserved control-record id, the chunk size);
  3. `AwtControlKind` — same set, same codes, same ids;
  4. `AwtReplyKind` — same set, same codes, same ids;
  5. `java.awt.Cursor` types — same set, same numbers, same ids;
  6. `java.awt.event` ids the two sides name agree;
  7. `ScaleMode` ids agree;
  8. every control kind the core can emit is handled by the Kotlin projection
     (an unhandled kind would be dropped without a trace);
  9. every JSON key the core emits for the control plane is read by Kotlin.

Usage:  python3 scripts/check_awt_wire.py
"""

from __future__ import annotations

import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

RUST_AWT = os.path.join(ROOT, "rust/crates/rc-launcher-core/src/launch/awt.rs")
RUST_FAKEFX = os.path.join(ROOT, "rust/crates/rc-launcher-core/src/launch/fakefx.rs")
KT_WIRE = os.path.join(ROOT, "app/src/main/java/com/rc/launcher/ui/awt/AwtWire.kt")
KT_CONTROL = os.path.join(ROOT, "app/src/main/java/com/rc/launcher/ui/awt/AwtControl.kt")
KT_GEOMETRY = os.path.join(ROOT, "app/src/main/java/com/rc/launcher/ui/awt/AwtGeometry.kt")
KT_BRIDGE = os.path.join(ROOT, "app/src/main/java/com/rc/launcher/ui/awt/AwtCanvasBridge.kt")

problems: list[str] = []


def fail(msg: str) -> None:
    problems.append(msg)


def read(path: str) -> str:
    if not os.path.exists(path):
        fail(f"missing source file: {os.path.relpath(path, ROOT)}")
        return ""
    with open(path, encoding="utf-8") as handle:
        return handle.read()


def num(text: str) -> int:
    """Parse a Rust/Kotlin integer literal (`0x5243_4146`, `8 * 1024`, `1 << 6`)."""
    text = text.strip().rstrip(";").strip()
    text = re.sub(r"\.toInt\(\)$", "", text)
    text = text.replace("_", "")
    text = re.sub(r"\b(\d+)(u8|u16|u32|u64|i8|i16|i32|i64|usize|isize|L)\b", r"\1", text)
    text = text.replace("ushr", ">>").replace("shl", "<<")
    value = eval(text, {"__builtins__": {}}, {})  # noqa: S307 - literals only
    if not isinstance(value, int):
        raise ValueError(f"not an integer literal: {text!r}")
    # Kotlin `Int` is signed 32-bit; normalise so 0xFF000000 == -16777216.
    if value >= 1 << 31:
        value -= 1 << 32
    return value


def rust_const(src: str, name: str) -> int | None:
    m = re.search(rf"pub const {re.escape(name)}\s*:\s*[A-Za-z0-9_]+\s*=\s*([^;]+);", src)
    if not m:
        fail(f"Rust constant {name} not found")
        return None
    return num(m.group(1))


def kt_const(src: str, name: str) -> int | None:
    m = re.search(rf"const val {re.escape(name)}\s*:\s*Int\s*=\s*(.+)", src)
    if not m:
        fail(f"Kotlin constant {name} not found")
        return None
    return num(m.group(1).split("//")[0])


def compare(label: str, rust: int | None, kotlin: int | None) -> None:
    if rust is None or kotlin is None:
        return
    if rust != kotlin:
        fail(f"{label}: Rust {rust} ({rust:#x}) != Kotlin {kotlin} ({kotlin:#x})")


def main() -> int:
    rust_awt = read(RUST_AWT)
    rust_fakefx = read(RUST_FAKEFX)
    kt_wire = read(KT_WIRE)
    kt_control = read(KT_CONTROL)
    kt_geometry = read(KT_GEOMETRY)
    if problems:
        return report()

    # ---- 1. frame / event wire ------------------------------------------
    for rust_name, kt_name in [
        ("FRAME_MAGIC", "FRAME_MAGIC"),
        ("FRAME_VERSION", "FRAME_VERSION"),
        ("FRAME_HEADER_LEN", "FRAME_HEADER_LEN"),
        ("EVENT_RECORD_LEN", "EVENT_RECORD_LEN"),
        ("MAX_CANVAS_DIM", "MAX_CANVAS_DIM"),
        ("OPAQUE_BLACK", "OPAQUE_BLACK"),
    ]:
        compare(rust_name, rust_const(rust_awt, rust_name), kt_const(kt_wire, kt_name))

    # ---- 2. control plane ------------------------------------------------
    for name in [
        "CONTROL_MAGIC",
        "CONTROL_VERSION",
        "CONTROL_HEADER_LEN",
        "MAX_CONTROL_TEXT",
        "CONTROL_EVENT_ID",
        "CONTROL_CHUNK_BYTES",
    ]:
        compare(name, rust_const(rust_awt, name), kt_const(kt_control, name))

    # The two headers must be the same length, or the demultiplexing reader in
    # `AwtFrameStream::read_next` cannot read one header for both record types.
    frame_header = rust_const(rust_awt, "FRAME_HEADER_LEN")
    control_header = rust_const(rust_awt, "CONTROL_HEADER_LEN")
    if frame_header != control_header:
        fail(
            "FRAME_HEADER_LEN != CONTROL_HEADER_LEN: one stream reader cannot "
            "demultiplex frames and control messages"
        )
    if rust_const(rust_awt, "FRAME_MAGIC") == rust_const(rust_awt, "CONTROL_MAGIC"):
        fail("FRAME_MAGIC == CONTROL_MAGIC: the two record types are indistinguishable")

    # ---- 3. control kinds -----------------------------------------------
    rust_kinds = dict(
        (name, int(code))
        for name, code in re.findall(
            r"AwtControlKind::(\w+) => (\d+),", rust_awt
        )
    )
    rust_kind_ids = dict(
        (name, ident)
        for name, ident in re.findall(
            r'AwtControlKind::(\w+) => "([a-z_]+)",', rust_awt
        )
    )
    kt_kinds = {}
    for block in re.finditer(
        r"enum class AwtControlKind\(val id: String, val code: Int\) \{(.*?)\n    ;",
        kt_control,
        re.S,
    ):
        for name, ident, code in re.findall(r'(\w+)\("([a-z_]+)",\s*(\d+)\)', block.group(1)):
            kt_kinds[ident] = int(code)
    rust_kind_by_id = {
        rust_kind_ids[name]: code for name, code in rust_kinds.items() if name in rust_kind_ids
    }
    if not rust_kind_by_id:
        fail("could not extract AwtControlKind codes from the Rust source")
    if rust_kind_by_id != kt_kinds:
        fail(
            "AwtControlKind mismatch:\n"
            f"  Rust   : {sorted(rust_kind_by_id.items())}\n"
            f"  Kotlin : {sorted(kt_kinds.items())}"
        )

    # ---- 4. reply kinds --------------------------------------------------
    rust_replies = dict(
        (name, int(code))
        for name, code in re.findall(r"AwtReplyKind::(\w+) => (\d+),", rust_awt)
    )
    rust_reply_ids = dict(
        re.findall(r'AwtReplyKind::(\w+) => "([a-z_]+)",', rust_awt)
    )
    rust_reply_by_id = {
        rust_reply_ids[name]: code
        for name, code in rust_replies.items()
        if name in rust_reply_ids
    }
    kt_replies = {}
    for block in re.finditer(
        r"enum class AwtReplyKind\(val id: String, val code: Int\) \{(.*?)\n    ;",
        kt_control,
        re.S,
    ):
        for name, ident, code in re.findall(r'(\w+)\("([a-z_]+)",\s*(\d+)\)', block.group(1)):
            kt_replies[ident] = int(code)
    if not rust_reply_by_id:
        fail("could not extract AwtReplyKind codes from the Rust source")
    if rust_reply_by_id != kt_replies:
        fail(
            "AwtReplyKind mismatch:\n"
            f"  Rust   : {sorted(rust_reply_by_id.items())}\n"
            f"  Kotlin : {sorted(kt_replies.items())}"
        )

    # ---- 5. cursor types -------------------------------------------------
    rust_cursor_nums = dict(
        (name, num(value))
        for name, value in re.findall(
            r"pub const (\w+): i32 = (-?\d+);", _section(rust_awt, "pub mod cursor_type {")
        )
    )
    # `CUSTOM` is deliberately not a Kotlin enum entry (it degrades to DEFAULT).
    rust_cursor_nums.pop("CUSTOM", None)
    kt_cursors = {}
    for block in re.finditer(
        r"enum class AwtCursorKind\(val id: String, val awtType: Int\) \{(.*?)\n    ;",
        kt_control,
        re.S,
    ):
        for name, ident, awt_type in re.findall(r'(\w+)\("([a-z_]+)",\s*(\d+)\)', block.group(1)):
            kt_cursors[name] = (ident, int(awt_type))
    if not rust_cursor_nums or not kt_cursors:
        fail("could not extract the cursor tables")
    else:
        if set(rust_cursor_nums) != set(kt_cursors):
            fail(
                "cursor set mismatch:\n"
                f"  Rust   : {sorted(rust_cursor_nums)}\n"
                f"  Kotlin : {sorted(kt_cursors)}"
            )
        for name, value in rust_cursor_nums.items():
            if name in kt_cursors and kt_cursors[name][1] != value:
                fail(f"cursor {name}: Rust {value} != Kotlin {kt_cursors[name][1]}")
        # `CursorKind::id()` in Rust must produce the same lowercase ids.
        rust_cursor_ids = set(
            re.findall(r'=> "([a-z_]+)"', _section(rust_awt, "impl CursorKind {"))
        )
        for _name, (ident, _value) in kt_cursors.items():
            if ident not in rust_cursor_ids:
                fail(f"cursor id {ident!r} is not produced by Rust CursorKind::id()")

    # ---- 6. java.awt.event ids ------------------------------------------
    rust_events = dict(
        (name, num(value))
        for name, value in re.findall(
            r"pub const (\w+): i32 = (\d+);", _section(rust_awt, "pub mod event_id {")
        )
    )
    kt_events = dict(
        (name, num(value))
        for name, value in re.findall(r"const val (\w+) = (\d+)\n", kt_wire)
    )
    for name, value in kt_events.items():
        if name in rust_events and rust_events[name] != value:
            fail(f"event id {name}: Rust {rust_events[name]} != Kotlin {value}")
        if name not in rust_events:
            fail(f"Kotlin names event id {name} which the core does not define")

    # ---- 7. scale modes --------------------------------------------------
    rust_modes = set(re.findall(r'ScaleMode::\w+ => "([a-z_]+)"', rust_awt))
    kt_modes = set()
    for block in re.finditer(
        r"enum class AwtScaleMode\(val id: String, val label: String\) \{(.*?)\n    ;",
        kt_geometry,
        re.S,
    ):
        kt_modes = set(re.findall(r'\w+\("([a-z_]+)"', block.group(1)))
    if rust_modes and kt_modes and rust_modes != kt_modes:
        fail(f"ScaleMode ids differ: Rust {sorted(rust_modes)} != Kotlin {sorted(kt_modes)}")

    # ---- 8. every kind is handled by the Kotlin fake projection ---------
    #
    # `FakeAwtCanvasBridge` re-implements the core's projection so the whole UI
    # (and its unit tests) runs without the native library. A kind it does not
    # handle would silently do nothing there while working in production - the
    # worst kind of divergence, because the tests would still be green.
    kt_bridge = read(KT_BRIDGE)
    for ident in kt_kinds:
        const = ident.upper()
        if f"AwtControlKind.{const}" not in kt_control + kt_bridge:
            fail(f"Kotlin never handles control kind {ident}")

    # ---- 9. control JSON keys -------------------------------------------
    state_section = _section(rust_fakefx, "impl AwtControlState {")
    emitted = set(re.findall(r'"([a-z_]+)":', state_section))
    # Everything the core promises to emit (mirrored by the Rust contract test
    # `the_control_snapshot_carries_every_field_the_compose_layer_parses`).
    core_keys = {
        "cursor",
        "cursor_awt_type",
        "title",
        "ime",
        "wants_keyboard",
        "clipboard_out",
        "clipboard_requests",
        "windows",
        "window_count",
        "beeps",
        "bye",
    }
    # The subset the Compose layer actually parses. `cursor_awt_type` and
    # `window_count` are derivable there (the enum carries the AWT number, the
    # list carries its own length), so they are emitted for other consumers -
    # logs, bug reports, a non-Compose front end - but not required here.
    kotlin_keys = core_keys - {"cursor_awt_type", "window_count"}
    missing = core_keys - emitted
    if missing:
        fail(f"AwtControlState::to_json no longer emits {sorted(missing)}")
    for key in sorted(kotlin_keys):
        if f'"{key}"' not in kt_control:
            fail(f"Kotlin does not read the control key {key!r}")

    return report()


def _section(src: str, marker: str) -> str:
    """The source text from `marker` to the matching closing brace."""
    start = src.find(marker)
    if start < 0:
        fail(f"could not find {marker!r}")
        return ""
    depth = 0
    for index in range(start + len(marker) - 1, len(src)):
        if src[index] == "{":
            depth += 1
        elif src[index] == "}":
            depth -= 1
            if depth == 0:
                return src[start : index + 1]
    return src[start:]


def report() -> int:
    if problems:
        print("AWT wire contract check FAILED:\n")
        for problem in problems:
            print(f"  * {problem}")
        print(f"\n{len(problems)} problem(s).")
        return 1
    print("AWT wire contract OK: Rust and Kotlin agree on the frame, event and")
    print("control formats (magics, versions, kinds, cursors, ids, JSON keys).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
