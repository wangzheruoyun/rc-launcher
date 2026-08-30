package com.rc.launcher.ui.theme

import androidx.lifecycle.ViewModel
import kotlinx.coroutines.flow.StateFlow

/**
 * [ViewModel] exposing the theme engine to Compose through [StateFlow]s and
 * wrapping the mutation actions. This is the "ViewModel / StateFlow state
 * container" required by task 11 — the UI only ever observes immutable state and
 * dispatches intents back through the exposed methods.
 *
 * Task 11 adds the custom-background [StateFlow] and the matching mutators so the
 * Settings screen can drive the home / launch background without touching the
 * engine singleton directly.
 */
class ThemeViewModel : ViewModel() {
    val availableThemes: StateFlow<List<ThemeData>> = ThemeEngine.availableThemes
    val currentTheme: StateFlow<ThemeData> = ThemeEngine.currentTheme
    val nightMode: StateFlow<ThemeNightMode> = ThemeEngine.nightMode
    val backgroundConfig: StateFlow<BackgroundConfig> = ThemeEngine.backgroundConfig

    fun selectTheme(id: String) = ThemeEngine.setTheme(id)
    fun setNightMode(mode: ThemeNightMode) = ThemeEngine.setNightMode(mode)
    fun cycleNightMode() = ThemeEngine.cycleNightMode()

    // ---- Background (task 11) ---------------------------------------------

    fun setBackgroundEnabled(enabled: Boolean) = ThemeEngine.setBackgroundEnabled(enabled)
    fun setBackgroundUri(uri: String?) = ThemeEngine.setBackgroundUri(uri)
    fun setBackgroundEffect(effect: BackgroundEffect) = ThemeEngine.setBackgroundEffect(effect)
    fun setBlurRadius(dp: Int) = ThemeEngine.setBlurRadius(dp)
    fun setDarkenAlpha(alpha: Float) = ThemeEngine.setDarkenAlpha(alpha)
    fun setFollowTheme(follow: Boolean) = ThemeEngine.setFollowTheme(follow)
    fun clearBackground() = ThemeEngine.clearBackground()
    fun setBackgroundConfig(config: BackgroundConfig) = ThemeEngine.setBackgroundConfig(config)
}
