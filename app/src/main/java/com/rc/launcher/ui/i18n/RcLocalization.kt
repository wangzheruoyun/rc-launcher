package com.rc.launcher.ui.i18n

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.LayoutDirection
import androidx.lifecycle.compose.collectAsStateWithLifecycle

/**
 * The string table for the current UI language (task 20).
 *
 * `static` because a language switch changes *every* string on screen: a static
 * local invalidates the whole subtree once instead of tracking thousands of
 * individual reads.
 *
 * Defaults to an empty table (lookups echo their key), so a composable used
 * outside [RcLocalizationProvider] — a `@Preview`, a test — still renders.
 */
val LocalRcStrings = staticCompositionLocalOf { RcStrings.empty() }

/**
 * Binds [LocaleEngine] to the composition (task 20).
 *
 * Provides three things at once, which is what makes the switch *instant* and
 * complete:
 *  * [LocalRcStrings] — the core-backed table read by [rcString];
 *  * [LocalContext] / [LocalConfiguration] overridden with a locale-specific
 *    `Context`, so plain `stringResource(...)` and any Android-formatted text
 *    (dates, numbers) follow the in-app choice as well as the device language;
 *  * [LocalLayoutDirection] derived from the language, so adding an RTL
 *    translation later cannot silently keep an LTR layout.
 */
@Composable
fun RcLocalizationProvider(content: @Composable () -> Unit) {
    val strings by LocaleEngine.strings.collectAsStateWithLifecycle()
    val effective by LocaleEngine.effective.collectAsStateWithLifecycle()
    val context = LocalContext.current

    val localizedContext = remember(context, effective) {
        runCatching { RcLocaleContext.localizedContext(context, effective) }.getOrDefault(context)
    }
    val configuration = remember(localizedContext, effective) {
        localizedContext.resources.configuration
    }
    val layoutDirection = if (effective.isRtl()) LayoutDirection.Rtl else LayoutDirection.Ltr

    CompositionLocalProvider(
        LocalRcStrings provides strings,
        LocalContext provides localizedContext,
        LocalConfiguration provides configuration,
        LocalLayoutDirection provides layoutDirection,
        content = content,
    )
}

/** None of the shipped languages is RTL; kept explicit so adding `ar`/`fa` is safe. */
private fun AppLanguage.isRtl(): Boolean = false

/** The localised message for [key] (echoes the key when it is missing). */
@Composable
fun rcString(key: String): String = LocalRcStrings.current[key]

/** The localised message for [key] with `{name}` placeholders filled in. */
@Composable
fun rcString(key: String, vararg args: Pair<String, String>): String =
    LocalRcStrings.current.format(key, *args)

/** The plural-correct message for [base] and [count] (supplies `{count}`). */
@Composable
fun rcPlural(base: String, count: Long, vararg args: Pair<String, String>): String =
    LocalRcStrings.current.plural(base, count, *args)

/** The display name of [language] in the *current* language. */
@Composable
fun rcLanguageName(language: AppLanguage): String =
    LocalRcStrings.current[RcStringKeys.nameKeyOf(language)]
