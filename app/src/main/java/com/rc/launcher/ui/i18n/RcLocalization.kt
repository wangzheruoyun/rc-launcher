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

// --- Locale-aware values (task 20) ----------------------------------------
//
// Derived copy ("1.4 GB", "剩余 3 分 20 秒", "42.5%") must follow the in-app
// language just like prose does, so it goes through the same table rather than
// through a private English unit ladder. See [RcValueFormat].

/** A byte size in the current language (`1.4 GB`). */
@Composable
fun rcBytes(value: Long): String = RcValueFormat.bytes(LocalRcStrings.current, value)

/** A transfer rate in the current language (`1.2 MB/秒`). */
@Composable
fun rcRate(bytesPerSecond: Long): String =
    RcValueFormat.rate(LocalRcStrings.current, bytesPerSecond)

/** A byte-progress pair in the current language (`1.0 MB / 4.0 MB`). */
@Composable
fun rcByteProgress(done: Long, total: Long): String =
    RcValueFormat.byteProgress(LocalRcStrings.current, done, total)

/** A humanised duration in the current language (`3 分 20 秒`). */
@Composable
fun rcDuration(seconds: Long): String = RcValueFormat.duration(LocalRcStrings.current, seconds)

/** A download ETA in the current language (`剩余 3 分 20 秒`). */
@Composable
fun rcEta(seconds: Long): String = RcValueFormat.eta(LocalRcStrings.current, seconds)

/** A relative timestamp in the current language (`3 分前`). */
@Composable
fun rcRelativeTime(deltaSeconds: Long): String =
    RcValueFormat.relativeTime(LocalRcStrings.current, deltaSeconds)

/** A percentage in the current language (`42.5%`). */
@Composable
fun rcPercent(value: Double, fractionDigits: Int = 1): String =
    RcValueFormat.percent(LocalRcStrings.current, value, fractionDigits)

/** A frame rate in the current language (`59.9 FPS`). */
@Composable
fun rcFps(value: Double): String = RcValueFormat.fps(LocalRcStrings.current, value)

/** A grouped integer in the current language (`1,234,567`). */
@Composable
fun rcInteger(value: Long): String = RcValueFormat.int(LocalRcStrings.current, value)
