package com.rc.launcher.ui.theme

import androidx.activity.ComponentActivity
import android.os.Build
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat

/**
 * The launcher's Material 3 [MaterialTheme] wrapper.
 *
 * It resolves the effective dark mode from [ThemeNightMode] (SYSTEM follows the
 * OS via [isSystemInDarkTheme]) and applies the chosen [ThemeData]'s
 * [ColorScheme]. It also tints the system status / navigation bars to match the
 * surface colors, exactly like FCL's theme engine drives the Android chrome.
 */
@Composable
fun RcTheme(
    theme: ThemeData = ThemeEngine.currentTheme.collectAsState().value,
    nightMode: ThemeNightMode = ThemeEngine.nightMode.collectAsState().value,
    content: @Composable () -> Unit,
) {
    val systemDark = isSystemInDarkTheme()
    val dark = when (nightMode) {
        ThemeNightMode.SYSTEM -> systemDark
        ThemeNightMode.LIGHT -> false
        ThemeNightMode.DARK -> true
    }
    val view = LocalView.current
    // Base scheme from the seed palette (always available, no Android context needed).
    val baseScheme = remember(theme, dark) { theme.colorScheme(dark) }
    // On Android 12+ a "dynamic" theme is replaced by the system wallpaper palette.
    val colorScheme = if (theme.dynamic && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        if (dark) dynamicDarkColorScheme(view.context) else dynamicLightColorScheme(view.context)
    } else {
        baseScheme
    }

    if (!view.isInEditMode) {
        SideEffect {
            val activity = view.context as? ComponentActivity ?: return@SideEffect
            activity.window.statusBarColor = colorScheme.background.toArgb()
            activity.window.navigationBarColor = colorScheme.surface.toArgb()
            WindowCompat.getInsetsController(activity.window, view)
                .isAppearanceLightStatusBars = !dark
        }
    }

    MaterialTheme(
        colorScheme = colorScheme,
        typography = Typography(),
        content = content,
    )
}
