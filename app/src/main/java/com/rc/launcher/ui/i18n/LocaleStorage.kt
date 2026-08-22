package com.rc.launcher.ui.i18n

import android.content.Context
import android.content.SharedPreferences

/**
 * Persistence contract for the chosen UI language (task 20).
 *
 * Swappable so [LocaleEngine] is unit-testable without Android — the same split
 * as `ThemeStorage` (task 11) and `SettingsRepository` (task 14).
 */
interface LocaleStorage {
    /** The persisted [AppLanguage.tag], or `null` when the user never chose. */
    fun getLanguageTag(): String?

    /** Persist [tag] (an [AppLanguage.tag], including `"system"`). */
    fun setLanguageTag(tag: String)
}

/** [SharedPreferences]-backed implementation used by the app. */
class SharedPreferencesLocaleStorage(context: Context) : LocaleStorage {
    private val prefs: SharedPreferences =
        context.applicationContext.getSharedPreferences(NAME, Context.MODE_PRIVATE)

    override fun getLanguageTag(): String? = prefs.getString(KEY_LANGUAGE, null)

    override fun setLanguageTag(tag: String) {
        prefs.edit().putString(KEY_LANGUAGE, tag).apply()
    }

    companion object {
        private const val NAME = "rc_locale"
        private const val KEY_LANGUAGE = "language_tag"
    }
}

/** In-memory storage for tests and previews. */
class InMemoryLocaleStorage(private var tag: String? = null) : LocaleStorage {
    override fun getLanguageTag(): String? = tag
    override fun setLanguageTag(tag: String) {
        this.tag = tag
    }
}
