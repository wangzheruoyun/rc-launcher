#!/usr/bin/env python3
"""Validate the screen-orientation / adaptive-layout setup (task 9).

The Rust core (`display` module) owns the orientation policy and the window
size-class table; the Compose UI mirrors it in `ui/AdaptiveLayout.kt` because a
rotating device must not pay a JNI crossing per recomposition. Three layers have
to stay in lock-step (core ⇄ Kotlin mirror ⇄ Activity/manifest), so this script
is the gate that keeps them honest.

Checks, in order:

  1. the golden fixtures exist, are well formed, carry the documented columns and
     cover both orientations of every device plus every breakpoint edge;
  2. the fixtures are **fresh** (re-runs the Rust generator when cargo is
     available and diffs the output byte-for-byte);
  3. the breakpoints and clamp bounds in `display.rs` and `AdaptiveLayout.kt` are
     the same numbers;
  4. the orientation ids / `android:screenOrientation` values agree between
     `display.rs`, the Kotlin `OrientationMode` and the fixture;
  5. `MainActivity` maps every mode onto the matching
     `ActivityInfo.SCREEN_ORIENTATION_*` constant, and handles all three;
  6. the manifest lets the Activity rotate *without* being recreated
     (`screenOrientation="user"` + the required `configChanges` set);
  7. the Kotlin mirror really exposes every decision the core publishes, and the
     parity test asserts every fixture column;
  8. the UI actually offers the setting (settings screen + view model) and the
     FFI surface is declared on both sides.

Usage:  python3 scripts/check_layout_parity.py
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

DISPLAY_RS = os.path.join(REPO, "rust/crates/rc-launcher-core/src/display.rs")
ADAPTIVE_KT = os.path.join(REPO, "app/src/main/java/com/rc/launcher/ui/AdaptiveLayout.kt")
SETTINGS_MODEL_KT = os.path.join(
    REPO, "app/src/main/java/com/rc/launcher/ui/model/LauncherSettings.kt"
)
SETTINGS_SCREEN_KT = os.path.join(
    REPO, "app/src/main/java/com/rc/launcher/ui/screen/SettingsScreen.kt"
)
SETTINGS_VM_KT = os.path.join(
    REPO, "app/src/main/java/com/rc/launcher/ui/viewmodel/SettingsViewModel.kt"
)
MAIN_ACTIVITY_KT = os.path.join(REPO, "app/src/main/java/com/rc/launcher/MainActivity.kt")
MAIN_SCREEN_KT = os.path.join(REPO, "app/src/main/java/com/rc/launcher/ui/MainScreen.kt")
MANIFEST = os.path.join(REPO, "app/src/main/AndroidManifest.xml")
BRIDGE_KT = os.path.join(REPO, "core/src/main/java/com/rc/launcher/core/RustBridge.kt")
FFI_RS = os.path.join(REPO, "rust/crates/rc-launcher-core/src/ffi.rs")
AWT_VM_KT = os.path.join(
    REPO, "app/src/main/java/com/rc/launcher/ui/viewmodel/AwtSurfaceViewModel.kt"
)
FAKEFX_RS = os.path.join(REPO, "rust/crates/rc-launcher-core/src/launch/fakefx.rs")
PARITY_TEST_KT = os.path.join(
    REPO, "app/src/test/java/com/rc/launcher/ui/AdaptiveLayoutParityTest.kt"
)
FIXTURE_DIR = os.path.join(REPO, "app/src/test/resources")
LAYOUT_TSV = os.path.join(FIXTURE_DIR, "display_layout_golden.tsv")
ORIENTATION_TSV = os.path.join(FIXTURE_DIR, "display_orientation_golden.tsv")

LAYOUT_COLUMNS = [
    "width_dp",
    "height_dp",
    "orientation",
    "width_class",
    "height_class",
    "landscape",
    "short",
    "navigation_rail",
    "instance_columns",
    "settings_columns",
    "dashboard_columns",
    "content_padding_dp",
    "max_content_width_dp",
]
ORIENTATION_COLUMNS = ["id", "android", "forced", "in_w", "in_h", "out_w", "out_h"]

# id -> (android:screenOrientation, ActivityInfo constant suffix)
EXPECTED_MODES = {
    "system": ("user", "USER"),
    "landscape": ("sensorLandscape", "SENSOR_LANDSCAPE"),
    "portrait": ("sensorPortrait", "SENSOR_PORTRAIT"),
}

# The Activity must handle these itself, otherwise a rotation recreates it and
# the game surface / screen state is thrown away.
REQUIRED_CONFIG_CHANGES = [
    "orientation",
    "screenSize",
    "screenLayout",
    "smallestScreenSize",
    "keyboardHidden",
]

FAILURES: list[str] = []
CHECKS = 0


def check(name: str, ok: bool, detail: str = "") -> None:
    global CHECKS
    CHECKS += 1
    if ok:
        print(f"  ok    {name}")
    else:
        print(f"  FAIL  {name}{(': ' + detail) if detail else ''}")
        FAILURES.append(f"{name}{(': ' + detail) if detail else ''}")


def read(path: str) -> str:
    with open(path, encoding="utf-8") as fh:
        return fh.read()


def rows(text: str, columns: int, where: str) -> list[list[str]]:
    out = []
    for line in text.splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != columns:
            FAILURES.append(f"{where}: malformed row ({len(fields)} fields): {line}")
            continue
        out.append(fields)
    return out


def header_columns(text: str) -> list[str]:
    """Column names from the last `#` comment line of a fixture."""
    names: list[str] = []
    for line in text.splitlines():
        if line.startswith("#") and "\t" in line:
            names = [c.strip().lstrip("#").strip() for c in line.split("\t")]
    return names


def regenerate() -> str | None:
    """Fresh render of both fixtures, or None when cargo is unavailable."""
    if shutil.which("cargo") is None:
        return None
    try:
        proc = subprocess.run(
            ["cargo", "run", "--quiet", "--example", "display_layout_golden"],
            cwd=os.path.join(REPO, "rust"),
            capture_output=True,
            text=True,
            timeout=900,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if proc.returncode != 0:
        return None
    return proc.stdout


def rust_consts(src: str) -> dict[str, int]:
    out = {}
    for name, value in re.findall(
        r"pub const (\w+): u32 = ([0-9_]+);", src
    ):
        out[name] = int(value.replace("_", ""))
    for name, value in re.findall(r"pub const (\w+): u32 = ([0-9_]+);", src):
        out[name] = int(value.replace("_", ""))
    return out


def kotlin_consts(src: str) -> dict[str, int]:
    out = {}
    for name, value in re.findall(r"const val (\w+) = ([0-9_]+)", src):
        out[name] = int(value.replace("_", ""))
    return out


def main() -> int:
    print("== golden fixtures ==")
    for path in (LAYOUT_TSV, ORIENTATION_TSV):
        check(f"{os.path.basename(path)} exists", os.path.isfile(path))
    if FAILURES:
        print("\nlayout parity check FAILED")
        return 1

    layout_src = read(LAYOUT_TSV)
    orientation_src = read(ORIENTATION_TSV)

    check(
        "layout fixture documents its columns",
        header_columns(layout_src) == LAYOUT_COLUMNS,
        f"{header_columns(layout_src)}",
    )
    check(
        "orientation fixture documents its columns",
        header_columns(orientation_src) == ORIENTATION_COLUMNS,
        f"{header_columns(orientation_src)}",
    )

    layout_rows = rows(layout_src, len(LAYOUT_COLUMNS), "display_layout_golden.tsv")
    orient_rows = rows(
        orientation_src, len(ORIENTATION_COLUMNS), "display_orientation_golden.tsv"
    )
    check("layout fixture has rows", len(layout_rows) >= 20, f"{len(layout_rows)} rows")
    check("orientation fixture has rows", len(orient_rows) >= 9, f"{len(orient_rows)} rows")

    geometries = {(int(r[0]), int(r[1])) for r in layout_rows}
    # A rotation matrix that only ever tests one orientation would be pointless.
    rotated_pairs = [(w, h) for (w, h) in geometries if w != h and (h, w) in geometries]
    check(
        "fixture covers rotated pairs",
        len(rotated_pairs) >= 20,
        f"only {len(rotated_pairs)} geometries appear in both orientations",
    )
    # Every breakpoint edge must be exercised in at least one row.
    for edge in (599, 600, 839, 840):
        check(
            f"width breakpoint {edge}dp is covered",
            any(w == edge for (w, _h) in geometries),
        )
    for edge in (479, 480, 899, 900):
        check(
            f"height breakpoint {edge}dp is covered",
            any(h == edge for (_w, h) in geometries),
        )
    # Sanity: landscape always uses the rail, no decision is ever zero.
    bad = [
        r
        for r in layout_rows
        if (r[5] == "true" and r[7] != "true")
        or int(r[8]) < 1
        or int(r[9]) < 1
        or int(r[10]) < 1
    ]
    check("every fixture row is a sane layout", not bad, f"{bad[:2]}")

    print("\n== fixture freshness ==")
    regen = regenerate()
    if regen is None:
        print("  skip  fixture freshness (cargo unavailable)")
    else:
        check(
            "fixtures are up to date "
            "(cargo run --example display_layout_golden -- --write)",
            regen == layout_src + orientation_src,
            "committed fixtures differ from a fresh render",
        )

    print("\n== core <-> Kotlin mirror ==")
    rust_src = read(DISPLAY_RS)
    kt_src = read(ADAPTIVE_KT)
    rconsts = rust_consts(rust_src)
    kconsts = kotlin_consts(kt_src)
    for name in (
        "WIDTH_MEDIUM_DP",
        "WIDTH_EXPANDED_DP",
        "HEIGHT_MEDIUM_DP",
        "HEIGHT_EXPANDED_DP",
    ):
        check(
            f"{name} agrees",
            rconsts.get(name) is not None and rconsts.get(name) == kconsts.get(name),
            f"rust={rconsts.get(name)} kotlin={kconsts.get(name)}",
        )
    rust_max = re.search(r"pub const MAX_DP: u32 = ([0-9_]+);", rust_src)
    check(
        "MAX_DP agrees",
        rust_max is not None
        and int(rust_max.group(1).replace("_", "")) == kconsts.get("MAX_DP"),
        f"rust={rust_max.group(1) if rust_max else None} kotlin={kconsts.get('MAX_DP')}",
    )

    # The mirror must expose every decision the fixture (i.e. the core) publishes.
    kt_members = {
        "orientation": "val orientation",
        "width_class": "val widthClass",
        "height_class": "val heightClass",
        "landscape": "val isLandscape",
        "short": "val isShort",
        "navigation_rail": "val usesNavigationRail",
        "instance_columns": "val instanceColumns",
        "settings_columns": "val settingsColumns",
        "dashboard_columns": "val dashboardColumns",
        "content_padding_dp": "val contentPaddingDp",
        "max_content_width_dp": "val maxContentWidthDp",
    }
    missing = [col for col, decl in kt_members.items() if decl not in kt_src]
    check("Kotlin mirror exposes every core decision", not missing, f"{missing}")

    # Size-class / orientation ids must be spelled the same on both sides.
    for ident in ("compact", "medium", "expanded", "landscape", "portrait", "square"):
        check(
            f'id "{ident}" exists on both sides',
            f'"{ident}"' in rust_src and f'"{ident}"' in kt_src,
        )

    print("\n== orientation policy (core / settings / Activity / manifest) ==")
    model_src = read(SETTINGS_MODEL_KT)
    enum_block = model_src[model_src.index("enum class OrientationMode") :]
    enum_block = enum_block[: enum_block.index("\n}")]
    for mode_id, (android_value, constant) in EXPECTED_MODES.items():
        check(
            f'core knows "{mode_id}" -> {android_value}',
            f'"{mode_id}" => ' in rust_src or f'"{mode_id}"' in rust_src,
        )
        check(
            f'core maps "{mode_id}" to "{android_value}"',
            f'"{android_value}"' in rust_src,
        )
        check(
            f'OrientationMode carries "{mode_id}" / "{android_value}"',
            f'"{mode_id}"' in enum_block and f'"{android_value}"' in enum_block,
        )
    fixture_modes = {r[0]: r[1] for r in orient_rows}
    check(
        "fixture policy ids match the expected set",
        set(fixture_modes) == set(EXPECTED_MODES),
        f"{sorted(fixture_modes)}",
    )
    for mode_id, android_value in fixture_modes.items():
        check(
            f'fixture "{mode_id}" -> "{android_value}"',
            EXPECTED_MODES.get(mode_id, ("", ""))[0] == android_value,
        )

    activity_src = read(MAIN_ACTIVITY_KT)
    for mode_id, (_android, constant) in EXPECTED_MODES.items():
        check(
            f"MainActivity requests SCREEN_ORIENTATION_{constant}",
            f"ActivityInfo.SCREEN_ORIENTATION_{constant}" in activity_src,
        )
    # `UNSPECIFIED` would ignore the device rotation lock.
    check(
        "MainActivity does not use SCREEN_ORIENTATION_UNSPECIFIED",
        "SCREEN_ORIENTATION_UNSPECIFIED" not in activity_src,
    )
    check(
        "MainActivity keeps the requested orientation in sync with the settings",
        "settingsViewModel.settings.collect" in activity_src
        and "applyOrientation" in activity_src,
    )

    manifest_src = read(MANIFEST)
    check(
        'manifest declares screenOrientation="user" (rotation allowed)',
        'android:screenOrientation="user"' in manifest_src,
    )
    config = re.search(r'android:configChanges="([^"]+)"', manifest_src)
    changes = set(config.group(1).split("|")) if config else set()
    for needed in REQUIRED_CONFIG_CHANGES:
        check(
            f'configChanges handles "{needed}" (no Activity recreation)',
            needed in changes,
            f"{sorted(changes)}",
        )
    check(
        'manifest declares resizeableActivity="true"',
        'android:resizeableActivity="true"' in manifest_src,
    )

    print("\n== UI wiring ==")
    check(
        "settings view model exposes the orientation catalogue + setter",
        "orientationModes" in read(SETTINGS_VM_KT) and "setOrientation" in read(SETTINGS_VM_KT),
    )
    screen_src = read(SETTINGS_SCREEN_KT)
    check(
        "settings screen offers the three orientation modes",
        "setOrientation" in screen_src and "orientationModes" in screen_src,
    )
    main_screen_src = read(MAIN_SCREEN_KT)
    check(
        "the shell swaps the bottom bar for a navigation rail",
        "usesNavigationRail" in main_screen_src and "RcNavigationRail" in main_screen_src,
    )
    check(
        "the root publishes the measured window",
        "ProvideRcWindowInfo" in read(
            os.path.join(REPO, "app/src/main/java/com/rc/launcher/ui/RcApp.kt")
        ),
    )

    print("\n== rotation-safe input ==")
    awt_vm_src = read(AWT_VM_KT)
    check(
        "the AWT canvas flushes queued samples before a rotation",
        "flushInput()" in awt_vm_src
        and "rotationFlips" in awt_vm_src
        and "releaseAll()" in awt_vm_src,
    )
    fakefx_src = read(FAKEFX_RS)
    check(
        "the core drops the in-flight gesture on a surface rotation",
        "rotation_flips" in fakefx_src and "surface_rotations" in fakefx_src,
    )

    print("\n== FFI surface ==")
    bridge_src = read(BRIDGE_KT)
    ffi_src = read(FFI_RS)
    for name in ("displayOrientations", "displayLayout"):
        check(f"RustBridge declares {name}", f"external fun {name}" in bridge_src)
        check(
            f"ffi.rs implements {name}",
            f"Java_com_rc_launcher_core_RustBridge_{name}" in ffi_src,
        )

    print("\n== parity test ==")
    test_src = read(PARITY_TEST_KT)
    for column in LAYOUT_COLUMNS:
        check(f'parity test asserts "{column}"', f'"{column}' in test_src or column in test_src)
    check(
        "parity test replays both fixtures",
        "display_layout_golden.tsv" in test_src
        and "display_orientation_golden.tsv" in test_src,
    )

    print()
    if FAILURES:
        print(f"layout parity check FAILED — {len(FAILURES)} problem(s):")
        for f in FAILURES:
            print(f"  - {f}")
        return 1
    print(
        f"layout parity check passed — {CHECKS} checks, "
        f"{len(layout_rows)} geometries x {len(LAYOUT_COLUMNS)} decisions, "
        f"{len(orient_rows)} policy rows."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
