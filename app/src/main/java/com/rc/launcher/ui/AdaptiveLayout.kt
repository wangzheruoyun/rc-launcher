package com.rc.launcher.ui

import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier

/**
 * Adaptive layout / screen-orientation support for the launcher UI (task 9).
 *
 * This file is the **Kotlin mirror** of the Rust core's `display` module
 * (`rust/crates/rc-launcher-core/src/display.rs`): same breakpoints, same
 * orientation classification, same column counts. The layout has to be resolved
 * on *every* recomposition of a rotating device, so paying a JNI round trip for
 * it would be absurd — instead the numbers live in the core (single source of
 * truth, exhaustively unit-tested there), are re-implemented here in pure
 * Kotlin, and `scripts/check_layout_parity.py` fails the build if the two ever
 * drift apart. `RustBridge.displayLayout(...)` exposes the core's answer for the
 * parity test and for the diagnostics screen.
 *
 * Nothing here depends on Android beyond the two Compose helpers at the bottom,
 * so every rule is covered by plain JVM unit tests
 * (`app/src/test/.../ui/AdaptiveLayoutTest.kt`).
 */

// ===========================================================================
// Orientation
// ===========================================================================

/** Orientation of a concrete rectangle (window, surface, virtual desktop). */
enum class ScreenOrientation(val id: String) {
    LANDSCAPE("landscape"),
    PORTRAIT("portrait"),

    /**
     * Exactly square *or not measured yet*. Deliberately its own value: the very
     * first `onSizeChanged` must not look like a rotation, otherwise the launcher
     * would drop a gesture the user never made.
     */
    SQUARE("square"),
    ;

    val isLandscape: Boolean get() = this == LANDSCAPE
    val isPortrait: Boolean get() = this == PORTRAIT

    /**
     * `true` when going from `this` to [other] is a real quarter turn
     * (landscape ⇄ portrait). Growing / shrinking inside one orientation (soft
     * keyboard, split screen, foldable hinge) is **not** a rotation.
     */
    fun flipsTo(other: ScreenOrientation): Boolean =
        (this == LANDSCAPE && other == PORTRAIT) || (this == PORTRAIT && other == LANDSCAPE)

    companion object {
        /** Classify a `width x height` rectangle (0 on either axis ⇒ [SQUARE]). */
        fun of(width: Int, height: Int): ScreenOrientation = when {
            width <= 0 || height <= 0 -> SQUARE
            width > height -> LANDSCAPE
            width < height -> PORTRAIT
            else -> SQUARE
        }

        /** Parse [id]; `null` for anything unknown. */
        fun fromId(id: String?): ScreenOrientation? = entries.firstOrNull { it.id == id }
    }
}

/**
 * `true` when resizing a surface from `from` to `to` flips its orientation.
 *
 * The predicate behind the rotation handling of the game / AWT surface: a
 * quarter turn moves the letterbox bars *and* the finger, so every in-flight
 * gesture must be released instead of silently re-mapped into the new viewport
 * (that re-mapping is precisely the "my tap landed in the wrong place after
 * rotating" bug). Mirrors `display::rotation_flips`.
 */
fun rotationFlips(fromWidth: Int, fromHeight: Int, toWidth: Int, toHeight: Int): Boolean =
    ScreenOrientation.of(fromWidth, fromHeight).flipsTo(ScreenOrientation.of(toWidth, toHeight))

// ===========================================================================
// Window size classes
// ===========================================================================

/**
 * Material-style window size class for one axis. Mirrors `display::SizeClass`.
 */
enum class RcSizeClass(val id: String) {
    COMPACT("compact"),
    MEDIUM("medium"),
    EXPANDED("expanded"),
    ;

    companion object {
        /** Width breakpoint: `< 600dp` is a phone in portrait. */
        const val WIDTH_MEDIUM_DP = 600

        /** Width breakpoint: `>= 840dp` is a tablet / large foldable. */
        const val WIDTH_EXPANDED_DP = 840

        /** Height breakpoint: `< 480dp` is a phone in landscape. */
        const val HEIGHT_MEDIUM_DP = 480

        /** Height breakpoint: `>= 900dp` is a tall tablet. */
        const val HEIGHT_EXPANDED_DP = 900

        /** Classify a width in dp. */
        fun ofWidthDp(widthDp: Int): RcSizeClass = when {
            widthDp < WIDTH_MEDIUM_DP -> COMPACT
            widthDp < WIDTH_EXPANDED_DP -> MEDIUM
            else -> EXPANDED
        }

        /** Classify a height in dp. */
        fun ofHeightDp(heightDp: Int): RcSizeClass = when {
            heightDp < HEIGHT_MEDIUM_DP -> COMPACT
            heightDp < HEIGHT_EXPANDED_DP -> MEDIUM
            else -> EXPANDED
        }

        /** Parse [id]; `null` for anything unknown. */
        fun fromId(id: String?): RcSizeClass? = entries.firstOrNull { it.id == id }
    }
}

/**
 * The live launcher window in **dp** plus every layout decision derived from it
 * (mirror of `display::WindowMetrics`).
 *
 * Built from two integers, so it is recomputed on every configuration change
 * instead of caching geometry that a rotation just invalidated.
 */
data class RcWindowInfo(val widthDp: Int, val heightDp: Int) {

    /** Sanitised axes: a bogus configuration can never produce absurd layouts. */
    private val w: Int get() = widthDp.coerceIn(1, MAX_DP)
    private val h: Int get() = heightDp.coerceIn(1, MAX_DP)

    /** Orientation of the window. */
    val orientation: ScreenOrientation get() = ScreenOrientation.of(w, h)

    /**
     * `true` when the window is at least as wide as it is tall. A square window
     * counts as landscape on purpose: the extra width is what the rail needs.
     */
    val isLandscape: Boolean get() = w >= h

    /** Width size class. */
    val widthClass: RcSizeClass get() = RcSizeClass.ofWidthDp(w)

    /** Height size class. */
    val heightClass: RcSizeClass get() = RcSizeClass.ofHeightDp(h)

    /**
     * `true` when vertical space is scarce (a phone in landscape): hero sections
     * collapse and the UI must not also spend ~80dp on a bottom navigation bar.
     */
    val isShort: Boolean get() = heightClass == RcSizeClass.COMPACT

    /**
     * Side navigation rail instead of a bottom navigation bar.
     *
     * Every landscape window and every window at least
     * [RcSizeClass.WIDTH_MEDIUM_DP] wide gets the rail: in landscape a bottom bar
     * eats the scarce height *and* sits right under the thumbs holding the phone.
     */
    val usesNavigationRail: Boolean
        get() = isLandscape || widthClass != RcSizeClass.COMPACT

    /** Column count for the instance grid (home + instances screens). */
    val instanceColumns: Int
        get() = when (widthClass) {
            // A landscape phone is short but wide enough for two cards.
            RcSizeClass.COMPACT -> if (isLandscape) 2 else 1
            RcSizeClass.MEDIUM -> 2
            RcSizeClass.EXPANDED -> 3
        }

    /** Column count for the settings screen's section cards. */
    val settingsColumns: Int
        get() = when (widthClass) {
            RcSizeClass.COMPACT -> 1
            // Settings rows are wide; only spend two columns when really wide.
            RcSizeClass.MEDIUM -> if (isLandscape) 2 else 1
            RcSizeClass.EXPANDED -> 2
        }

    /** Column count for the home dashboard's summary cards. */
    val dashboardColumns: Int
        get() = if (widthClass == RcSizeClass.COMPACT && !isLandscape) 1 else 2

    /** Screen edge padding in dp. */
    val contentPaddingDp: Int
        get() = when (widthClass) {
            RcSizeClass.COMPACT -> 16
            RcSizeClass.MEDIUM -> 20
            RcSizeClass.EXPANDED -> 24
        }

    /**
     * Upper bound for a text column so a 1200dp tablet does not render
     * 200-character-wide settings descriptions.
     */
    val maxContentWidthDp: Int
        get() = when (widthClass) {
            RcSizeClass.COMPACT -> MAX_DP
            RcSizeClass.MEDIUM -> 720
            RcSizeClass.EXPANDED -> 1080
        }

    /** The same window rotated by 90° (previews + tests). */
    fun rotated(): RcWindowInfo = RcWindowInfo(h, w)

    companion object {
        /** Clamp bound shared with the core (`WindowMetrics::MAX_DP`). */
        const val MAX_DP = 10_000

        /** A reference phone in portrait (Pixel-class, 392x872dp). */
        val DEFAULT = RcWindowInfo(392, 872)

        /** Sanitising factory. */
        fun of(widthDp: Int, heightDp: Int): RcWindowInfo =
            RcWindowInfo(widthDp.coerceIn(1, MAX_DP), heightDp.coerceIn(1, MAX_DP))
    }
}

// ===========================================================================
// Compose plumbing
// ===========================================================================

/**
 * The window info every screen reads. Defaults to [RcWindowInfo.DEFAULT] so
 * previews and unit-tested composables work without the provider.
 */
val LocalRcWindowInfo: ProvidableCompositionLocal<RcWindowInfo> =
    staticCompositionLocalOf { RcWindowInfo.DEFAULT }

/** Shorthand for `LocalRcWindowInfo.current`. */
@Composable
fun rcWindowInfo(): RcWindowInfo = LocalRcWindowInfo.current

/**
 * Measure the available space and publish it as [LocalRcWindowInfo].
 *
 * Deliberately implemented with [BoxWithConstraints] rather than
 * `LocalConfiguration`: the constraints are what the UI is actually laid out
 * with (so split screen, freeform windows and foldables are handled for free),
 * they are re-measured without an Activity recreation on rotation, and the
 * composable does not depend on any Compose API that moved between versions.
 */
@Composable
fun ProvideRcWindowInfo(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    BoxWithConstraints(modifier = modifier.fillMaxSize()) {
        val info = RcWindowInfo.of(maxWidth.value.toInt(), maxHeight.value.toInt())
        CompositionLocalProvider(LocalRcWindowInfo provides info, content = content)
    }
}
