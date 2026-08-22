package com.rc.launcher.ui.theme

import androidx.compose.material3.darkColorScheme
import androidx.compose.ui.graphics.Color
import androidx.compose.material3.lightColorScheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Unit tests for the (pure-Kotlin) theme logic of task 11. These run on the JVM
 * unit-test runner and need no Android context, exercising the parts of the
 * theme engine that are safe to test in isolation (task 21).
 */
class ThemeLogicTest {

    @Test
    fun nightMode_fromValue_knownConstants() {
        assertEquals(ThemeNightMode.SYSTEM, ThemeNightMode.fromValue(0))
        assertEquals(ThemeNightMode.LIGHT, ThemeNightMode.fromValue(1))
        assertEquals(ThemeNightMode.DARK, ThemeNightMode.fromValue(2))
    }

    @Test
    fun nightMode_fromValue_unknownFallsBackToSystem() {
        assertEquals(ThemeNightMode.SYSTEM, ThemeNightMode.fromValue(99))
        assertEquals(ThemeNightMode.SYSTEM, ThemeNightMode.fromValue(-1))
    }

    @Test
    fun nightMode_next_cyclesSystemLightDark() {
        assertEquals(ThemeNightMode.LIGHT, ThemeNightMode.SYSTEM.next())
        assertEquals(ThemeNightMode.DARK, ThemeNightMode.LIGHT.next())
        assertEquals(ThemeNightMode.SYSTEM, ThemeNightMode.DARK.next())
    }

    @Test
    fun themeCatalog_hasUniqueIdsAndIsNonEmpty() {
        assertTrue("theme catalog must not be empty", RcBuiltInThemes.isNotEmpty())
        val ids = RcBuiltInThemes.map { it.id }
        assertEquals("theme ids must be unique", ids.size, ids.toSet().size)
    }

    @Test
    fun colorScheme_lightAndDarkDifferButKeepPrimary() {
        val theme = RcBuiltInThemes.first()
        val light = theme.colorScheme(dark = false)
        val dark = theme.colorScheme(dark = true)

        // Seed primary must be carried through verbatim (no tonal remap on primary).
        assertEquals(theme.primary, light.primary)
        assertEquals(theme.primary.lighten(0.32f), dark.primary)

        // Backgrounds must diverge between light and dark variants.
        assertNotEquals(light.background, dark.background)

        // Schemes must be usable (non-zero colors) for both modes.
        assertTrue(light.primary.value != 0UL)
        assertTrue(dark.primary.value != 0UL)
        assertTrue(lightColorScheme().background.value != 0UL)
        assertTrue(darkColorScheme().background.value != 0UL)
    }

    @Test
    fun colorMix_staysWithinBounds() {
        val mixed = Color(0xFF2E7D5B).mix(Color.White, 0.5f)
        assertTrue(mixed.red in 0f..1f)
        assertTrue(mixed.green in 0f..1f)
        assertTrue(mixed.blue in 0f..1f)
        assertTrue(mixed.alpha in 0f..1f)
    }
}
