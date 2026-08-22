package com.rc.launcher.ui.theme

import android.content.Context
import android.content.SharedPreferences

/**
 * Persistence contract for the theme engine. Swappable for tests / previews
 * (mirrors FCL's `fcllibrary/theme/ThemePreference.kt`). The default
 * implementation is backed by [SharedPreferences].
 */
interface ThemeStorage {
    fun getThemeId(): String?
    fun setThemeId(id: String)
    fun getNightMode(): Int
    fun setNightMode(mode: Int)
}

class SharedPreferencesThemeStorage(context: Context) : ThemeStorage {
    private val prefs: SharedPreferences =
        context.applicationContext.getSharedPreferences(NAME, Context.MODE_PRIVATE)

    override fun getThemeId(): String? = prefs.getString(KEY_THEME_ID, null)

    override fun setThemeId(id: String) {
        prefs.edit().putString(KEY_THEME_ID, id).apply()
    }

    override fun getNightMode(): Int =
        prefs.getInt(KEY_NIGHT_MODE, ThemeNightMode.SYSTEM.value)

    override fun setNightMode(mode: Int) {
        prefs.edit().putInt(KEY_NIGHT_MODE, mode).apply()
    }

    companion object {
        private const val NAME = "rc_theme"
        private const val KEY_THEME_ID = "theme_id"
        private const val KEY_NIGHT_MODE = "night_mode"
    }
}
