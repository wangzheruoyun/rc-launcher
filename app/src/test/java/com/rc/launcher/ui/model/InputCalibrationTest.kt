package com.rc.launcher.ui.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs
import kotlin.math.sqrt

/** Unit tests for the input-layer calibration (dead-zone / sensitivity). */
class InputCalibrationTest {

    private fun eq(a: Float, b: Float, tol: Float = 1e-5f) = assertTrue(
        "expected $b but was $a",
        abs(a - b) < tol,
    )

    @Test
    fun axisDeadzoneZeroesSmallValues() {
        val cal = InputCalibration(deadzone = 0.2f)
        eq(cal.calibrateAxis(0.0f), 0f)
        eq(cal.calibrateAxis(0.1f), 0f)
        eq(cal.calibrateAxis(-0.1f), 0f)
    }

    @Test
    fun axisRescalesOutsideDeadzone() {
        val cal = InputCalibration(deadzone = 0.2f)
        // Full deflection reaches 1.0.
        eq(cal.calibrateAxis(1.0f), 1f)
        // 0.6 -> (0.6 - 0.2) / 0.8 = 0.5
        eq(cal.calibrateAxis(0.6f), 0.5f)
    }

    @Test
    fun axisSensitivityScales() {
        val cal = InputCalibration(deadzone = 0.2f, sensitivity = 2.0f)
        // (0.6 - 0.2) / 0.8 * 2 = 1.0 (clamped).
        eq(cal.calibrateAxis(0.6f), 1f)
    }

    @Test
    fun axisInverts() {
        val cal = InputCalibration(deadzone = 0.2f, invertX = true)
        eq(cal.calibrateAxis(0.6f), -0.5f)
    }

    @Test
    fun axisSanitizedClampsOutOfRange() {
        val cal = InputCalibration(deadzone = 2f, sensitivity = -5f)
        val c = cal.sanitized()
        eq(c.deadzone, InputCalibration.MAX_DEADZONE)
        eq(c.sensitivity, InputCalibration.MIN_SENSITIVITY)
    }

    @Test
    fun stickRadialDeadzone() {
        val cal = InputCalibration(deadzone = 0.25f)
        // Magnitude 0.2 < 0.25 -> fully zeroed.
        assertEquals(Pair(0f, 0f), cal.calibrateStick(0.2f, 0.0f))
        assertEquals(Pair(0f, 0f), cal.calibrateStick(0.1414f, 0.1414f))
    }

    @Test
    fun stickFullDeflectionPreserved() {
        val cal = InputCalibration(deadzone = 0.25f)
        val (x, y) = cal.calibrateStick(0.0f, 1.0f)
        eq(x, 0f)
        eq(y, 1f)
    }

    @Test
    fun stickAxisInvert() {
        val cal = InputCalibration(deadzone = 0.1f, invertX = true)
        val (x, y) = cal.calibrateStick(0.5f, 0.5f)
        assertTrue(x < 0f)
        assertTrue(y > 0f)
    }

    @Test
    fun stickPreservesDirectionAndMagnitude() {
        val cal = InputCalibration(deadzone = 0.2f)
        val (x, y) = cal.calibrateStick(0.3f, 0.4f)
        // Direction (atan2) preserved.
        eq(kotlin.math.atan2(0.3f, 0.4f), kotlin.math.atan2(x, y))
        // Magnitude rescaled to (mag - dz) / (1 - dz) = (0.5 - 0.2) / 0.8 = 0.375.
        val mag = sqrt(x * x + y * y)
        eq(mag, 0.375f)
        assertTrue(mag > 0f && mag <= 1f)
    }

    @Test
    fun fromProfileUsesRecommendedDefaults() {
        val cal = InputCalibration.fromProfile(GamepadDatabase.PLAYSTATION)
        eq(cal.deadzone, 0.12f)
        eq(cal.sensitivity, 1.0f)
    }
}
