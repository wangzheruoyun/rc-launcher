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

    /** Restore the persisted custom-background config (task 11). */
    fun getBackgroundConfig(): BackgroundConfig

    /** Persist the custom-background config (task 11). */
    fun setBackgroundConfig(config: BackgroundConfig)
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

    override fun getBackgroundConfig(): BackgroundConfig {
        val enabled = prefs.getBoolean(KEY_BG_ENABLED, false)
        val uri = prefs.getString(KEY_BG_URI, null)
        val effect = BackgroundEffect.fromValue(prefs.getInt(KEY_BG_EFFECT, BackgroundEffect.NONE.value))
        val blur = prefs.getInt(KEY_BG_BLUR, BackgroundConfig.DEFAULT.blurRadiusDp)
        val darken = prefs.getFloat(KEY_BG_DARKEN, BackgroundConfig.DEFAULT.darkenAlpha)
        val follow = prefs.getBoolean(KEY_BG_FOLLOW, BackgroundConfig.DEFAULT.followTheme)
        return BackgroundConfig(
            enabled = enabled,
            uri = uri,
            effect = effect,
            blurRadiusDp = blur,
            darkenAlpha = darken,
            followTheme = follow,
        ).normalized()
    }

    override fun setBackgroundConfig(config: BackgroundConfig) {
        val c = config.normalized()
        // NB: use the member-chain form, not `prefs.edit().apply { .. }` — the
        // lambda form clashes with Editor.apply() and fails to compile.
        prefs.edit()
            .putBoolean(KEY_BG_ENABLED, c.enabled)
            .putString(KEY_BG_URI, c.uri)
            .putInt(KEY_BG_EFFECT, c.effect.value)
            .putInt(KEY_BG_BLUR, c.blurRadiusDp)
            .putFloat(KEY_BG_DARKEN, c.darkenAlpha)
            .putBoolean(KEY_BG_FOLLOW, c.followTheme)
            .apply()
    }

    companion object {
        private const val NAME = "rc_theme"
        private const val KEY_THEME_ID = "theme_id"
        private const val KEY_NIGHT_MODE = "night_mode"
        private const val KEY_BG_ENABLED = "bg_enabled"
        private const val KEY_BG_URI = "bg_uri"
        private const val KEY_BG_EFFECT = "bg_effect"
        private const val KEY_BG_BLUR = "bg_blur"
        private const val KEY_BG_DARKEN = "bg_darken"
        private const val KEY_BG_FOLLOW = "bg_follow"
    }
}
