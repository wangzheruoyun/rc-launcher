package com.rc.launcher.ui.i18n

import android.app.LocaleManager
import android.content.Context
import android.content.res.Configuration
import android.os.Build
import android.os.LocaleList
import androidx.annotation.RequiresApi
import java.util.Locale

/**
 * The (only) place that touches Android's locale plumbing (task 20).
 *
 * Isolated behind a tiny surface so the rest of the i18n framework stays pure
 * Kotlin and unit-testable, and so the API-level branches live in one file:
 *
 *  * [localizedContext] — a `Context` whose `Resources` resolve to a *chosen*
 *    language regardless of the device language. This is what makes in-app
 *    switching work for `stringResource(...)` as well as for [RcStrings].
 *  * [applyPerAppLocale] — hands the choice to the platform on Android 13+
 *    (`LocaleManager`), so the system's own "App languages" screen agrees with
 *    the in-app picker and the choice survives a cold start before our code runs.
 *  * [systemPreferredTags] — the device's ordered locale preferences, used to
 *    resolve [AppLanguage.SYSTEM].
 */
object RcLocaleContext {

    /** `zh-Hant` -> `Locale("zh", "TW", script "Hant")`, `en` -> `Locale("en")`. */
    fun localeOf(language: AppLanguage): Locale {
        val tag = language.platformTag ?: Locale.getDefault().toLanguageTag()
        return Locale.forLanguageTag(tag)
    }

    /**
     * A `Context` whose resources are resolved for [language].
     *
     * Uses `createConfigurationContext`, which is the supported way to read a
     * different locale's resources without mutating global state (mutating
     * `Resources.updateConfiguration` would fight the system and leak into other
     * components).
     */
    fun localizedContext(context: Context, language: AppLanguage): Context {
        if (language.isSystem) return context
        val locale = localeOf(language)
        val config = Configuration(context.resources.configuration)
        config.setLocale(locale)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            config.setLocales(LocaleList(locale))
        }
        return context.createConfigurationContext(config)
    }

    /**
     * Persist the choice with the platform on Android 13+ (API 33).
     *
     * [AppLanguage.SYSTEM] passes an empty `LocaleList`, which is how the
     * platform is told "follow the system again". A no-op below API 33 — there
     * the in-app [localizedContext] path is what localises the UI.
     *
     * Never throws: on a device/ROM without the service the in-app path still
     * works, and an untranslatable failure must not break a settings toggle.
     */
    fun applyPerAppLocale(context: Context, language: AppLanguage) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        runCatching { applyPerAppLocaleApi33(context, language) }
    }

    @RequiresApi(Build.VERSION_CODES.TIRAMISU)
    private fun applyPerAppLocaleApi33(context: Context, language: AppLanguage) {
        val manager = context.getSystemService(LocaleManager::class.java) ?: return
        manager.applicationLocales = when (val tag = language.platformTag) {
            null -> LocaleList.getEmptyLocaleList()
            else -> LocaleList.forLanguageTags(tag)
        }
    }

    /**
     * The device's ordered locale preferences as BCP-47 tags (most preferred
     * first). Falls back to the JVM default locale on old API levels.
     */
    fun systemPreferredTags(context: Context): List<String> = runCatching {
        val config = context.resources.configuration
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            val list = config.locales
            (0 until list.size()).map { list[it].toLanguageTag() }
        } else {
            @Suppress("DEPRECATION")
            listOf(config.locale.toLanguageTag())
        }
    }.getOrElse { listOf(Locale.getDefault().toLanguageTag()) }
}
