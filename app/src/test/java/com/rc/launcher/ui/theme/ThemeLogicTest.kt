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

/**
 * Unit tests for the custom-background logic added by task 11. All types under
 * test ([BackgroundConfig], [BackgroundEffect], [BackgroundValidator],
 * [BackgroundLimits]) are plain Kotlin with no Android dependency, so these run
 * on the JVM unit-test runner without a device or Robolectric.
 */
class BackgroundLogicTest {

    @Test
    fun effect_fromValue_knownAndUnknown() {
        assertEquals(BackgroundEffect.NONE, BackgroundEffect.fromValue(0))
        assertEquals(BackgroundEffect.BLUR, BackgroundEffect.fromValue(1))
        assertEquals(BackgroundEffect.DARKEN, BackgroundEffect.fromValue(2))
        assertEquals(BackgroundEffect.BLUR_DARKEN, BackgroundEffect.fromValue(3))
        // Unknown values fall back to NONE (no crash, no null).
        assertEquals(BackgroundEffect.NONE, BackgroundEffect.fromValue(99))
        assertEquals(BackgroundEffect.NONE, BackgroundEffect.fromValue(-1))
    }

    @Test
    fun effect_blurs_and_darkens_flags() {
        assertFalse(BackgroundEffect.NONE.blurs)
        assertFalse(BackgroundEffect.NONE.darkens)
        assertTrue(BackgroundEffect.BLUR.blurs)
        assertFalse(BackgroundEffect.BLUR.darkens)
        assertTrue(BackgroundEffect.DARKEN.darkens)
        assertFalse(BackgroundEffect.DARKEN.blurs)
        assertTrue(BackgroundEffect.BLUR_DARKEN.blurs)
        assertTrue(BackgroundEffect.BLUR_DARKEN.darkens)
    }

    @Test
    fun config_defaultsAreDisabledAndSafe() {
        val c = BackgroundConfig.DEFAULT
        assertFalse(c.enabled)
        assertNull(c.uri)
        assertEquals(BackgroundEffect.NONE, c.effect)
        // A disabled config is always valid, even with no uri.
        assertTrue(c.isValid())
    }

    @Test
    fun config_normalizedClampsBlurAndDarken() {
        val c = BackgroundConfig(blurRadiusDp = 999, darkenAlpha = 5f)
        val n = c.normalized()
        assertEquals(BackgroundLimits.MAX_BLUR_DP, n.blurRadiusDp)
        assertEquals(BackgroundLimits.MAX_DARKEN, n.darkenAlpha)

        val c2 = BackgroundConfig(blurRadiusDp = -10, darkenAlpha = -1f)
        val n2 = c2.normalized()
        assertEquals(BackgroundLimits.MIN_BLUR_DP, n2.blurRadiusDp)
        assertEquals(BackgroundLimits.MIN_DARKEN, n2.darkenAlpha)
    }

    @Test
    fun config_isValid_rejectsBadUriWhenEnabled() {
        // Enabled but blank uri is invalid.
        assertFalse(BackgroundConfig(enabled = true, uri = null).isValid())
        assertFalse(BackgroundConfig(enabled = true, uri = "   ").isValid())
        // Enabled with a vetted content uri is valid.
        assertTrue(
            BackgroundConfig(
                enabled = true,
                uri = "content://com.android.providers.media.documents/document/image%3A42",
            ).isValid(),
        )
        // Enabled with an http uri is rejected (would bypass the mirror policy).
        assertFalse(
            BackgroundConfig(enabled = true, uri = "http://evil.example/bg.png").isValid(),
        )
    }

    @Test
    fun validator_isValidUri_schemeAndPathRules() {
        // content:// with an authority is accepted.
        assertTrue(BackgroundValidator.isValidUri("content://media/external/images/1"))
        // content: with no authority is rejected.
        assertFalse(BackgroundValidator.isValidUri("content:something"))
        // file:// inside an allowed root is accepted.
        assertTrue(
            BackgroundValidator.isValidUri("file:///data/user/0/com.rc.launcher/files/background/bg.png"),
        )
        // http / https are rejected.
        assertFalse(BackgroundValidator.isValidUri("https://example.com/bg.png"))
        // blank is rejected.
        assertFalse(BackgroundValidator.isValidUri(""))
        assertFalse(BackgroundValidator.isValidUri(null))
    }

    @Test
    fun validator_isSafePath_blocksTraversalAndForeignRoots() {
        val root = "/data/user/0/com.rc.launcher/files/background/"
        // Inside the root is safe.
        assertTrue(BackgroundValidator.isSafePath("/data/user/0/com.rc.launcher/files/background/bg.png", root))
        // Traversal out of the root is rejected.
        assertFalse(BackgroundValidator.isSafePath("/data/user/0/com.rc.launcher/files/background/../../secrets", root))
        // A foreign absolute path is rejected.
        assertFalse(BackgroundValidator.isSafePath("/system/etc/hosts", root))
        // Structural checks run even without a root.
        assertFalse(BackgroundValidator.isSafePath("../x"))
        assertFalse(BackgroundValidator.isSafePath("content://x"))
        assertTrue(BackgroundValidator.isSafePath("/anything/allowed/when/no/root", null))
    }

    @Test
    fun config_effectiveDarkenAlpha_followsTheme() {
        val base = 0.4f
        val cfg = BackgroundConfig(darkenAlpha = base, followTheme = true)
        // Dark mode: overlay is softened (×0.6) but stays in [0,1].
        val dark = cfg.effectiveDarkenAlpha(isDark = true)
        assertTrue(dark in 0f..1f)
        assertEquals(base * 0.6f, dark, 1e-6f)
        // Light mode: overlay is strengthened (×1.25) but clamped to 1.
        val light = cfg.effectiveDarkenAlpha(isDark = false)
        assertTrue(light in 0f..1f)
        assertEquals((base * 1.25f).coerceIn(0f, 1f), light, 1e-6f)
        // followTheme off: returns the raw value unchanged.
        val noFollow = BackgroundConfig(darkenAlpha = base, followTheme = false)
        assertEquals(base, noFollow.effectiveDarkenAlpha(isDark = true), 1e-6f)
        assertEquals(base, noFollow.effectiveDarkenAlpha(isDark = false), 1e-6f)
    }
}
