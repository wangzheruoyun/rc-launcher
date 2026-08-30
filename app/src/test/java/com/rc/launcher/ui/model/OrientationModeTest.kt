package com.rc.launcher.ui.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The screen-orientation preference (task 9).
 *
 * The ids are a **cross-language contract**: they are persisted in the settings,
 * accepted by the Rust core (`display::OrientationPolicy::from_id`, which is what
 * makes a forced orientation also orient the game window) and asserted by
 * `scripts/check_layout_parity.py`. They must therefore never change.
 */
class OrientationModeTest {

    @Test
    fun idsAndLabelsAreStable() {
        assertEquals(
            listOf("system", "landscape", "portrait"),
            OrientationMode.entries.map { it.id },
        )
        assertEquals(OrientationMode.FOLLOW_SYSTEM, OrientationMode.DEFAULT)
        assertEquals("跟随系统", OrientationMode.FOLLOW_SYSTEM.label)
        assertEquals("强制横屏", OrientationMode.LANDSCAPE.label)
        assertEquals("强制竖屏", OrientationMode.PORTRAIT.label)
        // Every mode explains itself in the settings screen.
        for (mode in OrientationMode.entries) {
            assertTrue(mode.description.isNotBlank())
        }
    }

    @Test
    fun androidScreenOrientationMatchesTheCore() {
        // `user`, not `unspecified`: the device rotation lock must be honoured.
        assertEquals("user", OrientationMode.FOLLOW_SYSTEM.androidScreenOrientation)
        // `sensor*`: pin the axis, allow both of its directions.
        assertEquals("sensorLandscape", OrientationMode.LANDSCAPE.androidScreenOrientation)
        assertEquals("sensorPortrait", OrientationMode.PORTRAIT.androidScreenOrientation)
        assertFalse(OrientationMode.FOLLOW_SYSTEM.isForced)
        assertTrue(OrientationMode.LANDSCAPE.isForced)
        assertTrue(OrientationMode.PORTRAIT.isForced)
    }

    @Test
    fun parsingIsLenientAndNeverThrows() {
        for (mode in OrientationMode.entries) {
            assertEquals(mode, OrientationMode.fromId(mode.id))
            assertTrue(OrientationMode.isValidId(mode.id))
        }
        // A corrupt / stale settings value degrades to "follow system".
        assertEquals(OrientationMode.DEFAULT, OrientationMode.fromId("sideways"))
        assertEquals(OrientationMode.DEFAULT, OrientationMode.fromId(null))
        assertEquals(OrientationMode.DEFAULT, OrientationMode.fromId(""))
        assertFalse(OrientationMode.isValidId("sideways"))
        assertFalse(OrientationMode.isValidId(null))
    }

    @Test
    fun settingsPersistSanitiseAndRoundTripTheOrientation() {
        val forced = LauncherSettings(orientationModeId = OrientationMode.PORTRAIT.id)
        assertEquals(OrientationMode.PORTRAIT, forced.orientationMode())
        assertEquals(forced, forced.sanitized())

        // A bogus id is repaired instead of crashing the settings screen.
        val broken = LauncherSettings(orientationModeId = "upside-down").sanitized()
        assertEquals(OrientationMode.DEFAULT, broken.orientationMode())

        // The backup format carries the preference across a reinstall.
        val backup = forced.toBackupString()
        assertTrue(backup.contains("orientationModeId=portrait"))
        val restored = LauncherSettings.fromBackupString(backup)
        assertEquals(OrientationMode.PORTRAIT, restored?.orientationMode())
    }

    @Test
    fun theDefaultSettingsFollowTheSystem() {
        assertEquals(OrientationMode.FOLLOW_SYSTEM, LauncherSettings().orientationMode())
    }
}
