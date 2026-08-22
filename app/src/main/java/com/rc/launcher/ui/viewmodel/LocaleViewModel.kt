package com.rc.launcher.ui.viewmodel

import androidx.lifecycle.ViewModel
import com.rc.launcher.ui.i18n.AppLanguage
import com.rc.launcher.ui.i18n.LocaleEngine
import com.rc.launcher.ui.i18n.RcStrings
import com.rc.launcher.ui.i18n.RcStringKeys
import kotlinx.coroutines.flow.StateFlow

/**
 * ViewModel for the language picker (task 20).
 *
 * A thin, testable façade over the process-wide [LocaleEngine] — the same shape
 * as `ThemeViewModel` (task 11): the engine owns the state (so the picker, the
 * app shell and the Rust core never disagree) and this class only exposes it to
 * Compose and validates user intent.
 */
class LocaleViewModel : ViewModel() {

    /** What the user picked, possibly [AppLanguage.SYSTEM]. */
    val selected: StateFlow<AppLanguage> = LocaleEngine.selected

    /** The catalogue actually in use (never [AppLanguage.SYSTEM]). */
    val effective: StateFlow<AppLanguage> = LocaleEngine.effective

    /** The resolved string table. */
    val strings: StateFlow<RcStrings> = LocaleEngine.strings

    /** The entries the picker offers, "follow system" first. */
    val options: List<AppLanguage> = AppLanguage.selectable

    /** Switch the UI language and persist the choice. */
    fun setLanguage(language: AppLanguage) = LocaleEngine.setLanguage(language)

    /** Switch by persisted tag (`"system"`, `"zh-CN"`, ...). */
    fun setLanguageTag(tag: String?) = LocaleEngine.setLanguageTag(tag)

    /**
     * Human label for a picker row: the endonym, plus the catalogue that
     * "follow system" currently resolves to, so the choice is never ambiguous.
     */
    fun labelFor(language: AppLanguage, strings: RcStrings): String =
        if (language.isSystem) {
            val resolved = LocaleEngine.resolve(AppLanguage.SYSTEM)
            "${strings[RcStringKeys.LANGUAGE_SYSTEM]} · ${resolved.nativeName}"
        } else {
            language.nativeName
        }

    /** Re-load the table (after installing a translation overlay, say). */
    fun reload() = LocaleEngine.reload()
}
