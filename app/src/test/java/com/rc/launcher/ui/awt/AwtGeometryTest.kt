package com.rc.launcher.ui.awt

import com.rc.launcher.ui.ScreenOrientation
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Unit tests for the AWT viewport (task 18).
 *
 * These mirror the Rust `launch::awt::Viewport` tests on purpose: the two
 * implementations must agree *exactly*, otherwise the pixel we draw and the pixel
 * the core thinks was touched drift apart.
 */
class AwtGeometryTest {

    /** A 4:3 desktop (640x480) on a 16:9 surface (1600x900). */
    private fun viewport(mode: AwtScaleMode) = AwtViewport(640, 480, 1600, 900, mode)

    @Test
    fun fit_letterboxesAndCentres() {
        val p = viewport(AwtScaleMode.FIT).placement()
        assertEquals(AwtPlacement(200, 0, 1200, 900), p)
    }

    @Test
    fun stretch_fillsTheSurfaceExactly() {
        assertEquals(AwtPlacement(0, 0, 1600, 900), viewport(AwtScaleMode.STRETCH).placement())
    }

    @Test
    fun center_keepsOnePixelPerPixel() {
        assertEquals(AwtPlacement(480, 210, 640, 480), viewport(AwtScaleMode.CENTER).placement())
    }

    @Test
    fun fillCrop_coversTheSurfaceAndOverflows() {
        val p = viewport(AwtScaleMode.FILL_CROP).placement()
        assertEquals(AwtPlacement(0, -150, 1600, 1200), p)
        assertTrue("the picture extends past the top/bottom edges", p.y < 0)
    }

    @Test
    fun degenerateSizesYieldAnEmptyPlacement() {
        assertTrue(AwtViewport(0, 480, 1600, 900).placement().isEmpty)
        assertTrue(AwtViewport(640, 480, 0, 0).placement().isEmpty)
        assertNull(AwtViewport(640, 480, 0, 0).mapPointer(1f, 1f))
        assertEquals(AwtPoint(0, 0), AwtViewport(640, 480, 0, 0).mapPointerClamped(1f, 1f))
    }

    @Test
    fun aTapInTheMiddleMapsToTheDesktopCentre() {
        assertEquals(AwtPoint(320, 240), viewport(AwtScaleMode.FIT).mapPointer(800f, 450f))
    }

    @Test
    fun aTapOnTheLetterboxBarIsNotForwarded() {
        val v = viewport(AwtScaleMode.FIT)
        assertNull("left bar", v.mapPointer(100f, 450f))
        assertNull("right bar", v.mapPointer(1500f, 450f))
        // The right edge is exclusive: 200 + 1200 = 1400 already misses.
        assertNull("exclusive right edge", v.mapPointer(1400f, 450f))
        assertTrue(v.contains(1399f, 450f))
        assertFalse(v.contains(1400f, 450f))
    }

    @Test
    fun draggingOffThePictureClampsInsteadOfDropping() {
        val v = viewport(AwtScaleMode.FIT)
        assertEquals(AwtPoint(0, 240), v.mapPointerClamped(100f, 450f))
        assertEquals(AwtPoint(639, 479), v.mapPointerClamped(9000f, 9000f))
    }

    @Test
    fun nonFiniteCoordinatesAreRejected() {
        val v = viewport(AwtScaleMode.FIT)
        assertNull(v.mapPointer(Float.NaN, 10f))
        assertNull(v.mapPointer(10f, Float.POSITIVE_INFINITY))
        assertEquals(AwtPoint(0, 0), v.mapPointerClamped(Float.NaN, Float.NaN))
    }

    @Test
    fun scaleFactorsMatchThePlacement() {
        val (sx, sy) = viewport(AwtScaleMode.FIT).scale()
        assertEquals(1200f / 640f, sx, 1e-4f)
        assertEquals(900f / 480f, sy, 1e-4f)
        // A degenerate viewport reports 1:1 instead of dividing by zero.
        assertEquals(1f to 1f, AwtViewport(0, 0, 0, 0).scale())
    }

    @Test
    fun scaleModeIdsRoundTripAndFallBackToFit() {
        for (mode in AwtScaleMode.values()) {
            assertEquals(mode, AwtScaleMode.fromId(mode.id))
        }
        assertEquals(AwtScaleMode.FIT, AwtScaleMode.fromId(null))
        assertEquals(AwtScaleMode.FIT, AwtScaleMode.fromId("diagonally"))
    }

    @Test
    fun mapToSurfaceIsTheInverseOfMapPointer() {
        // Same integer math as `Viewport::map_to_surface` in the core: the pointer
        // overlay and the IME anchor must land exactly where the core thinks the
        // desktop pixel is drawn.
        val v = viewport(AwtScaleMode.FIT)
        val p = v.placement()
        assertEquals(p.x.toFloat() to p.y.toFloat(), v.mapToSurface(0, 0))
        val (sx, sy) = v.mapToSurface(320, 240)
        assertEquals(p.x + 320f * p.width / 640, sx, 1e-3f)
        assertEquals(p.y + 240f * p.height / 480, sy, 1e-3f)
        // Round-tripping a desktop pixel through both mappings returns it.
        val back = v.mapPointer(sx, sy)
        assertEquals(AwtPoint(320, 240), back)
        // A degenerate viewport answers with the origin instead of dividing by zero.
        assertEquals(0f to 0f, AwtViewport(0, 0, 0, 0).mapToSurface(5, 5))
    }

    @Test
    fun rectHelpers() {
        assertTrue(AwtRect(0, 0, 0, 4).isEmpty)
        assertEquals(8L, AwtRect(0, 0, 4, 2).area)
        assertEquals(AwtRect(0, 0, 4, 2), AwtRect.whole(4, 2))
    }
    // ---- Rotation (task 9) -------------------------------------------------

    @Test
    fun orientationOfTheSurfaceAndOfTheDesktop() {
        val v = AwtViewport(640, 480, 1080, 2400)
        assertEquals(ScreenOrientation.PORTRAIT, v.surfaceOrientation())
        assertEquals(ScreenOrientation.LANDSCAPE, v.screenOrientation())
        assertEquals(ScreenOrientation.SQUARE, AwtViewport(64, 64, 0, 0).surfaceOrientation())
    }

    @Test
    fun rotatesToDetectsOnlyQuarterTurns() {
        val portrait = AwtViewport(640, 480, 1080, 2400)
        assertTrue(portrait.rotatesTo(2400, 1080))
        // Soft keyboard / split screen inside one orientation.
        assertFalse(portrait.rotatesTo(1080, 1400))
        assertFalse(portrait.rotatesTo(1080, 2400))
        // Not measured yet.
        assertFalse(AwtViewport(640, 480, 0, 0).rotatesTo(1080, 2400))
    }

    @Test
    fun rotatingTheSurfaceMovesTheLetterboxBarsNotThePixels() {
        // The same tap position means a *different* desktop pixel after a
        // rotation — which is exactly why an in-flight gesture has to be dropped
        // instead of re-mapped (task 9).
        val wide = AwtViewport(640, 480, 1200, 900)
        val tall = wide.rotatedSurface()
        assertEquals(AwtViewport(640, 480, 900, 1200), tall)
        assertEquals(ScreenOrientation.LANDSCAPE, wide.surfaceOrientation())
        assertEquals(ScreenOrientation.PORTRAIT, tall.surfaceOrientation())
        assertTrue(wide.rotatesTo(tall.surfaceWidth, tall.surfaceHeight))
        val before = wide.mapPointer(600f, 450f)
        val after = tall.mapPointer(600f, 450f)
        assertEquals(AwtPoint(320, 240), before)
        assertEquals(AwtPoint(426, 133), after)
        assertTrue("the same finger position is a different desktop pixel", before != after)
        // Rotating twice is the identity.
        assertEquals(wide, wide.rotatedSurface().rotatedSurface())
    }

    // ---- Physical mouse geometry (task 12) --------------------------------

    @Test
    fun theCentreIsWhereACapturedPointerStarts() {
        val viewport = AwtViewport(320, 240, 640, 480)
        assertEquals(AwtPoint(160, 120), viewport.centre())
        // A 1x1 desktop still has a valid pixel to sit on.
        assertEquals(AwtPoint(0, 0), AwtViewport(1, 1, 10, 10).centre())
    }

    @Test
    fun relativeMotionIsClampedToTheDesktop() {
        val viewport = AwtViewport(320, 240, 640, 480)
        assertEquals(AwtPoint(110, 130), viewport.movePointer(AwtPoint(100, 120), 10, 10))
        // A mouse cannot push the pointer out of the desktop, in either direction.
        assertEquals(AwtPoint(319, 239), viewport.movePointer(AwtPoint(300, 200), 900, 900))
        assertEquals(AwtPoint(0, 0), viewport.movePointer(AwtPoint(10, 10), -900, -900))
        assertEquals(AwtPoint(5, 5), viewport.clampToScreen(5, 5))
        assertEquals(AwtPoint(0, 239), viewport.clampToScreen(-1, 9_000))
    }

    @Test
    fun deltasAreScaledByTheSensitivity() {
        val viewport = AwtViewport(320, 240, 640, 480)
        assertEquals(2 to 4, viewport.scaleDelta(AwtMouseSensitivity.DEFAULT, 2f, 4f))
        assertEquals(4 to 8, viewport.scaleDelta(AwtMouseSensitivity.uniform(2f), 2f, 4f))
        // Rounding, not truncation: a slow mouse must stay visible.
        assertEquals(1 to 0, viewport.scaleDelta(AwtMouseSensitivity.uniform(0.5f), 1f, 0f))
        assertEquals(
            2 to -4,
            viewport.scaleDelta(AwtMouseSensitivity.DEFAULT.copy(invertY = true), 2f, 4f),
        )
        // Garbage contributes nothing at all.
        assertEquals(0 to 0, viewport.scaleDelta(AwtMouseSensitivity.DEFAULT, Float.NaN, 1f))
    }
}
