package com.rc.launcher.ui.theme

import androidx.compose.material3.ColorScheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.ui.graphics.Color

/**
 * Dark/light strategy for the launcher, mirroring the night-mode handling of
 * FCL's `fcllibrary/theme/ThemeEngine.kt` (SYSTEM / LIGHT / DARK).
 */
enum class ThemeNightMode(val value: Int) {
    SYSTEM(0),
    LIGHT(1),
    DARK(2);

    companion object {
        fun fromValue(v: Int): ThemeNightMode =
            entries.firstOrNull { it.value == v } ?: SYSTEM
    }
}

/** SYSTEM -> LIGHT -> DARK -> SYSTEM (pure, testable cycle). */
fun ThemeNightMode.next(): ThemeNightMode = when (this) {
    ThemeNightMode.SYSTEM -> ThemeNightMode.LIGHT
    ThemeNightMode.LIGHT -> ThemeNightMode.DARK
    ThemeNightMode.DARK -> ThemeNightMode.SYSTEM
}

/** Mix this color toward [other] by [ratio] in [0, 1]. */
fun Color.mix(other: Color, ratio: Float): Color {
    val r = red + (other.red - red) * ratio
    val g = green + (other.green - green) * ratio
    val b = blue + (other.blue - blue) * ratio
    val a = alpha + (other.alpha - alpha) * ratio
    return Color(r, g, b, a)
}

/** Lighten toward white by [ratio]. */
fun Color.lighten(ratio: Float): Color = mix(Color.White, ratio.coerceIn(0f, 1f))

/** Darken toward black by [ratio]. */
fun Color.darken(ratio: Float): Color = mix(Color.Black, ratio.coerceIn(0f, 1f))

/**
 * A theme definition — the "ThemeData" concept borrowed from FCL's
 * `fcllibrary/theme/ThemeData.kt`. Instead of shipping a hand-tuned Material
 * palette we keep a compact seed palette and derive a full Material 3
 * [ColorScheme] for light and dark from it.
 */
data class ThemeData(
    val id: String,
    val name: String,
    val primary: Color,
    val secondary: Color,
    val tertiary: Color,
    val neutral: Color,        // tonal base for background / surface
    val neutralVariant: Color, // tonal base for surfaceVariant / outline
    val dynamic: Boolean = false,
) {
    fun colorScheme(dark: Boolean): ColorScheme =
        if (dark) darkScheme() else lightScheme()

    private fun lightScheme(): ColorScheme = lightColorScheme().copy(
        primary = primary,
        onPrimary = Color.White,
        primaryContainer = primary.lighten(0.84f),
        onPrimaryContainer = primary.darken(0.55f),
        secondary = secondary,
        onSecondary = Color.White,
        secondaryContainer = secondary.lighten(0.84f),
        onSecondaryContainer = secondary.darken(0.55f),
        tertiary = tertiary,
        onTertiary = Color.White,
        tertiaryContainer = tertiary.lighten(0.84f),
        onTertiaryContainer = tertiary.darken(0.55f),
        background = neutral.lighten(0.96f),
        onBackground = neutral.darken(0.82f),
        surface = neutral.lighten(0.98f),
        onSurface = neutral.darken(0.85f),
        surfaceVariant = neutralVariant.lighten(0.70f),
        onSurfaceVariant = neutralVariant.darken(0.55f),
        outline = neutralVariant.lighten(0.45f),
        outlineVariant = neutralVariant.lighten(0.80f),
        error = ERROR_LIGHT,
        onError = Color.White,
        errorContainer = ERROR_LIGHT.lighten(0.80f),
        onErrorContainer = ERROR_LIGHT.darken(0.60f),
    )

    private fun darkScheme(): ColorScheme = darkColorScheme().copy(
        primary = primary.lighten(0.32f),
        onPrimary = primary.darken(0.72f),
        primaryContainer = primary.darken(0.62f),
        onPrimaryContainer = primary.lighten(0.78f),
        secondary = secondary.lighten(0.32f),
        onSecondary = secondary.darken(0.72f),
        secondaryContainer = secondary.darken(0.62f),
        onSecondaryContainer = secondary.lighten(0.78f),
        tertiary = tertiary.lighten(0.32f),
        onTertiary = tertiary.darken(0.72f),
        tertiaryContainer = tertiary.darken(0.62f),
        onTertiaryContainer = tertiary.lighten(0.78f),
        background = neutral.darken(0.92f),
        onBackground = neutral.lighten(0.85f),
        surface = neutral.darken(0.88f),
        onSurface = neutral.lighten(0.88f),
        surfaceVariant = neutralVariant.darken(0.55f),
        onSurfaceVariant = neutralVariant.lighten(0.55f),
        outline = neutralVariant.darken(0.35f),
        outlineVariant = neutralVariant.darken(0.70f),
        error = ERROR_DARK,
        onError = ERROR_DARK.darken(0.75f),
        errorContainer = ERROR_DARK.darken(0.60f),
        onErrorContainer = ERROR_DARK.lighten(0.75f),
    )

    companion object {
        private val ERROR_LIGHT = Color(0xFFBA1A1A)
        private val ERROR_DARK = Color(0xFFF2B8B5)
    }
}

/**
 * Built-in theme catalog (task 11). Additional themes can be registered later;
 * the engine reads from [ThemeEngine.availableThemes].
 */
val RcBuiltInThemes: List<ThemeData> = listOf(
    ThemeData(
        id = "rc_emerald",
        name = "翡翠 (默认)",
        primary = Color(0xFF2E7D5B),
        secondary = Color(0xFF4CAF93),
        tertiary = Color(0xFF8BC34A),
        neutral = Color(0xFFE7EFEA),
        neutralVariant = Color(0xFFC2D2C8),
    ),
    ThemeData(
        id = "rc_ocean",
        name = "海洋",
        primary = Color(0xFF1565C0),
        secondary = Color(0xFF4F83CC),
        tertiary = Color(0xFF00897B),
        neutral = Color(0xFFE6ECF2),
        neutralVariant = Color(0xFFC3CEDA),
    ),
    ThemeData(
        id = "rc_sunset",
        name = "晚霞",
        primary = Color(0xFFE65100),
        secondary = Color(0xFFF2803C),
        tertiary = Color(0xFFC2185B),
        neutral = Color(0xFFFBEEE3),
        neutralVariant = Color(0xFFE6C9B8),
    ),
    ThemeData(
        id = "rc_graphite",
        name = "石墨",
        primary = Color(0xFF37474F),
        secondary = Color(0xFF607D8B),
        tertiary = Color(0xFF78909C),
        neutral = Color(0xFFECEFF1),
        neutralVariant = Color(0xFFCFD8DC),
    ),
    // Dynamic (Material You) — uses the system wallpaper palette on Android 12+,
    // gracefully falling back to the seed palette on older devices (see RcTheme).
    ThemeData(
        id = "rc_dynamic",
        name = "动态 (系统配色)",
        primary = Color(0xFF6750A4),
        secondary = Color(0xFF625B71),
        tertiary = Color(0xFF7D5260),
        neutral = Color(0xFFEDE6F0),
        neutralVariant = Color(0xFFE3DDE6),
        dynamic = true,
    ),
)
