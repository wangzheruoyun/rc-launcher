package com.rc.launcher.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Unit tests for the adaptive layout / orientation rules (task 9).
 *
 * These are the Kotlin half of the parity contract with the Rust core's
 * `display` module: every expectation below has a twin in
 * `rust/crates/rc-launcher-core/src/display.rs`, and
 * `scripts/check_layout_parity.py` asserts the two tables still agree.
 */
class AdaptiveLayoutTest {

    // ---- Orientation -------------------------------------------------------

    @Test
    fun orientationOfARectangle() {
        assertEquals(ScreenOrientation.LANDSCAPE, ScreenOrientation.of(1920, 1080))
        assertEquals(ScreenOrientation.PORTRAIT, ScreenOrientation.of(1080, 1920))
        assertEquals(ScreenOrientation.SQUARE, ScreenOrientation.of(1000, 1000))
        // Not measured yet ⇒ square, never a spurious rotation.
        assertEquals(ScreenOrientation.SQUARE, ScreenOrientation.of(0, 0))
        assertEquals(ScreenOrientation.SQUARE, ScreenOrientation.of(1080, 0))
        assertEquals(ScreenOrientation.SQUARE, ScreenOrientation.of(-5, 10))
    }

    @Test
    fun orientationIdsRoundTrip() {
        for (o in ScreenOrientation.entries) {
            assertEquals(o, ScreenOrientation.fromId(o.id))
        }
        assertNull(ScreenOrientation.fromId("sideways"))
        assertNull(ScreenOrientation.fromId(null))
        assertTrue(ScreenOrientation.LANDSCAPE.isLandscape)
        assertTrue(ScreenOrientation.PORTRAIT.isPortrait)
        assertFalse(ScreenOrientation.SQUARE.isLandscape)
        assertFalse(ScreenOrientation.SQUARE.isPortrait)
    }

    @Test
    fun onlyARealQuarterTurnCountsAsARotation() {
        // Rotation: the in-flight gesture must be dropped.
        assertTrue(rotationFlips(1080, 2400, 2400, 1080))
        assertTrue(rotationFlips(2400, 1080, 1080, 2400))
        // The soft keyboard shrinking the surface is not a rotation.
        assertFalse(rotationFlips(1080, 2400, 1080, 1400))
        // Split screen inside one orientation is not a rotation.
        assertFalse(rotationFlips(2400, 1080, 1200, 1080))
        // The very first measurement is not a rotation.
        assertFalse(rotationFlips(0, 0, 1080, 2400))
        assertFalse(rotationFlips(1000, 1000, 2400, 1080))
        // Identity.
        assertFalse(rotationFlips(1080, 2400, 1080, 2400))
    }

    // ---- Size classes ------------------------------------------------------

    @Test
    fun sizeClassesSitExactlyOnTheBreakpoints() {
        assertEquals(RcSizeClass.COMPACT, RcSizeClass.ofWidthDp(599))
        assertEquals(RcSizeClass.MEDIUM, RcSizeClass.ofWidthDp(600))
        assertEquals(RcSizeClass.MEDIUM, RcSizeClass.ofWidthDp(839))
        assertEquals(RcSizeClass.EXPANDED, RcSizeClass.ofWidthDp(840))
        assertEquals(RcSizeClass.COMPACT, RcSizeClass.ofHeightDp(479))
        assertEquals(RcSizeClass.MEDIUM, RcSizeClass.ofHeightDp(480))
        assertEquals(RcSizeClass.MEDIUM, RcSizeClass.ofHeightDp(899))
        assertEquals(RcSizeClass.EXPANDED, RcSizeClass.ofHeightDp(900))
        for (c in RcSizeClass.entries) {
            assertEquals(c, RcSizeClass.fromId(c.id))
        }
        assertNull(RcSizeClass.fromId("huge"))
    }

    @Test
    fun breakpointsMatchTheCore() {
        // Same numbers as `display::WIDTH_MEDIUM_DP` & friends.
        assertEquals(600, RcSizeClass.WIDTH_MEDIUM_DP)
        assertEquals(840, RcSizeClass.WIDTH_EXPANDED_DP)
        assertEquals(480, RcSizeClass.HEIGHT_MEDIUM_DP)
        assertEquals(900, RcSizeClass.HEIGHT_EXPANDED_DP)
        assertEquals(10_000, RcWindowInfo.MAX_DP)
    }

    // ---- Window info -------------------------------------------------------

    @Test
    fun metricsAreSanitisedAndRotatable() {
        val degenerate = RcWindowInfo.of(0, 0)
        assertEquals(RcWindowInfo(1, 1), degenerate)
        assertEquals(RcWindowInfo.MAX_DP, RcWindowInfo.of(Int.MAX_VALUE, 10).widthDp)
        // Even an un-sanitised instance must not produce nonsense.
        val raw = RcWindowInfo(0, -10)
        assertEquals(ScreenOrientation.SQUARE, raw.orientation)
        assertTrue(raw.instanceColumns in 1..3)

        val phone = RcWindowInfo(392, 872)
        assertEquals(RcWindowInfo(872, 392), phone.rotated())
        assertEquals(phone, phone.rotated().rotated())
        assertEquals(phone, RcWindowInfo.DEFAULT)
    }

    @Test
    fun phonePortraitUsesASingleColumnAndABottomBar() {
        val m = RcWindowInfo(392, 872)
        assertEquals(ScreenOrientation.PORTRAIT, m.orientation)
        assertEquals(RcSizeClass.COMPACT, m.widthClass)
        assertEquals(RcSizeClass.MEDIUM, m.heightClass)
        assertFalse(m.isLandscape)
        assertFalse(m.isShort)
        assertFalse(m.usesNavigationRail)
        assertEquals(1, m.instanceColumns)
        assertEquals(1, m.settingsColumns)
        assertEquals(1, m.dashboardColumns)
        assertEquals(16, m.contentPaddingDp)
    }

    @Test
    fun theSamePhoneInLandscapeSwitchesToARailAndMoreColumns() {
        val m = RcWindowInfo(392, 872).rotated()
        assertEquals(ScreenOrientation.LANDSCAPE, m.orientation)
        assertEquals(RcSizeClass.EXPANDED, m.widthClass)
        assertEquals(RcSizeClass.COMPACT, m.heightClass)
        assertTrue(m.isLandscape)
        assertTrue(m.isShort)
        assertTrue(m.usesNavigationRail)
        assertEquals(3, m.instanceColumns)
        assertEquals(2, m.settingsColumns)
        assertEquals(2, m.dashboardColumns)

        val small = RcWindowInfo(640, 360)
        assertEquals(RcSizeClass.MEDIUM, small.widthClass)
        assertEquals(RcSizeClass.COMPACT, small.heightClass)
        assertTrue(small.usesNavigationRail)
        assertEquals(2, small.instanceColumns)
        assertEquals(2, small.settingsColumns)
        assertEquals(20, small.contentPaddingDp)
    }

    @Test
    fun aNarrowLandscapePhoneStillGetsTheRail() {
        val m = RcWindowInfo(480, 320)
        assertEquals(RcSizeClass.COMPACT, m.widthClass)
        assertTrue(m.usesNavigationRail)
        assertEquals(2, m.instanceColumns)
        // Settings rows are wide: keep one column while the width is compact.
        assertEquals(1, m.settingsColumns)
    }

    @Test
    fun tabletsExpandToThreeColumns() {
        val m = RcWindowInfo(1280, 800)
        assertEquals(RcSizeClass.EXPANDED, m.widthClass)
        assertTrue(m.usesNavigationRail)
        assertEquals(3, m.instanceColumns)
        assertEquals(2, m.settingsColumns)
        assertEquals(24, m.contentPaddingDp)
        assertEquals(1080, m.maxContentWidthDp)

        val portrait = m.rotated()
        assertEquals(RcSizeClass.MEDIUM, portrait.widthClass)
        assertEquals(RcSizeClass.EXPANDED, portrait.heightClass)
        assertTrue(portrait.usesNavigationRail)
        assertEquals(2, portrait.instanceColumns)
        assertEquals(1, portrait.settingsColumns)
        assertEquals(720, portrait.maxContentWidthDp)
    }

    @Test
    fun everyMetricIsSaneAcrossTheWholeDpRange() {
        val widths = listOf(1, 100, 320, 599, 600, 720, 839, 840, 1024, 1600, 4000)
        val heights = listOf(1, 100, 320, 479, 480, 640, 899, 900, 1200, 4000)
        for (w in widths) {
            for (h in heights) {
                val m = RcWindowInfo.of(w, h)
                assertTrue("$m columns", m.instanceColumns in 1..3)
                assertTrue("$m settings", m.settingsColumns in 1..2)
                assertTrue("$m dashboard", m.dashboardColumns in 1..2)
                assertTrue("$m padding", m.contentPaddingDp in 16..24)
                assertTrue("$m maxWidth", m.maxContentWidthDp >= 720)
                // Landscape ⇒ rail, always.
                if (m.isLandscape) assertTrue("$m rail", m.usesNavigationRail)
                // A short window must never also carry a bottom bar.
                if (m.isShort && m.isLandscape) assertTrue(m.usesNavigationRail)
            }
        }
    }

    @Test
    fun rotatingNeverLosesTheRail() {
        // Every device that shows the rail in landscape must keep a usable layout
        // in portrait too (the shell swaps the rail for the bottom bar).
        for (w in listOf(392, 480, 600, 720, 840, 1280)) {
            for (h in listOf(320, 640, 872, 1280)) {
                val landscape = RcWindowInfo.of(maxOf(w, h), minOf(w, h))
                val portrait = landscape.rotated()
                assertTrue(landscape.usesNavigationRail)
                assertEquals(landscape.widthDp, portrait.heightDp)
                assertEquals(landscape.heightDp, portrait.widthDp)
            }
        }
    }
}
