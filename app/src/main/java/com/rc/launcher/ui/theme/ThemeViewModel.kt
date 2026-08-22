package com.rc.launcher.ui.theme

import androidx.lifecycle.ViewModel
import kotlinx.coroutines.flow.StateFlow

/**
 * [ViewModel] exposing the theme engine to Compose through [StateFlow]s and
 * wrapping the mutation actions. This is the "ViewModel / StateFlow state
 * container" required by task 11 — the UI only ever observes immutable state and
 * dispatches intents back through the exposed methods.
 */
class ThemeViewModel : ViewModel() {
    val availableThemes: StateFlow<List<ThemeData>> = ThemeEngine.availableThemes
    val currentTheme: StateFlow<ThemeData> = ThemeEngine.currentTheme
    val nightMode: StateFlow<ThemeNightMode> = ThemeEngine.nightMode

    fun selectTheme(id: String) = ThemeEngine.setTheme(id)
    fun setNightMode(mode: ThemeNightMode) = ThemeEngine.setNightMode(mode)
    fun cycleNightMode() = ThemeEngine.cycleNightMode()
}
