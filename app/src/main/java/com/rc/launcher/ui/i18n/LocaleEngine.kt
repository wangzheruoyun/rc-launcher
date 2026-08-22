package com.rc.launcher.ui.i18n

import android.content.Context
import kotlin.jvm.Volatile
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * App-wide, observable **locale engine** (task 20).
 *
 * Deliberately shaped like `ThemeEngine` (task 11): a process-wide singleton
 * holding [StateFlow]s that Compose collects, initialised from
 * `RcApplication.onCreate` and persisted through a swappable [LocaleStorage].
 *
 * Responsibilities:
 *  * resolve [AppLanguage.SYSTEM] against the device's locale list,
 *  * load the string table for the effective language (core first, Android
 *    resources as a fallback — see [CompositeStringsSource]),
 *  * tell the **Rust core** which language to speak, so crash verdicts and error
 *    messages coming out of the core are localised too,
 *  * hand the choice to the platform's per-app language service on Android 13+.
 *
 * Switching is a single call: the flows emit and Compose recomposes. No Activity
 * recreation, no process restart — which is the "多语言切换" requirement.
 */
object LocaleEngine {

    /** What the user picked (may be [AppLanguage.SYSTEM]). */
    private val _selected = MutableStateFlow(AppLanguage.SYSTEM)
    val selected: StateFlow<AppLanguage> = _selected.asStateFlow()

    /** The catalogue actually in use (never [AppLanguage.SYSTEM]). */
    private val _effective = MutableStateFlow(AppLanguage.BASE)
    val effective: StateFlow<AppLanguage> = _effective.asStateFlow()

    /** The resolved string table for [effective]. */
    private val _strings = MutableStateFlow(RcStrings.empty(AppLanguage.BASE))
    val strings: StateFlow<RcStrings> = _strings.asStateFlow()

    @Volatile
    private var storage: LocaleStorage? = null

    @Volatile
    private var source: StringsSource? = null

    /** Device locale preferences; re-read on [init] and on a configuration change. */
    @Volatile
    private var systemTags: List<String> = emptyList()

    @Volatile
    private var platformApply: ((AppLanguage) -> Unit)? = null

    /**
     * Wire the engine to a [Context] and restore the saved selection. Idempotent,
     * and safe to call before the native core exists (the table then comes from
     * the Android resources).
     */
    fun init(context: Context) {
        val app = context.applicationContext
        install(
            storage = SharedPreferencesLocaleStorage(app),
            source = CompositeStringsSource(
                listOf(CoreStringsSource(), ResourcesStringsSource(app)),
            ),
            systemTags = RcLocaleContext.systemPreferredTags(app),
            platformApply = { language -> RcLocaleContext.applyPerAppLocale(app, language) },
        )
    }

    /**
     * Framework-free variant used by unit tests (and previews): inject the
     * storage, the catalogue source and the device locale list directly.
     */
    fun install(
        storage: LocaleStorage,
        source: StringsSource,
        systemTags: List<String>,
        platformApply: ((AppLanguage) -> Unit)? = null,
    ) {
        this.storage = storage
        this.source = source
        this.systemTags = systemTags
        this.platformApply = platformApply
        // Restore, without re-persisting (a fresh install must stay on SYSTEM).
        apply(AppLanguage.fromTag(storage.getLanguageTag()), persist = false)
    }

    /** Re-read the device locale list (call from `onConfigurationChanged`). */
    fun onSystemLocalesChanged(tags: List<String>) {
        systemTags = tags
        if (_selected.value.isSystem) apply(AppLanguage.SYSTEM, persist = false)
    }

    /** The catalogue [selection] resolves to, honouring the device preferences. */
    fun resolve(selection: AppLanguage): AppLanguage =
        if (selection.isSystem) AppLanguage.negotiateList(systemTags) else selection

    /** Switch the UI language and persist the choice. */
    fun setLanguage(selection: AppLanguage) = apply(selection, persist = true)

    /** Switch by tag (`"system"`, `"zh-CN"`, ...); unknown tags mean "system". */
    fun setLanguageTag(tag: String?) = setLanguage(AppLanguage.fromTag(tag))

    /** Re-load the current table — e.g. after installing a translation overlay. */
    fun reload() = apply(_selected.value, persist = false)

    private fun apply(selection: AppLanguage, persist: Boolean) {
        val effective = resolve(selection)
        _selected.value = selection
        _effective.value = effective
        // A failure here must never break the switch: fall back to an empty table
        // (lookups echo the key) rather than leaving a stale language on screen.
        _strings.value = runCatching { source?.load(effective) }.getOrNull()
            ?: RcStrings.empty(effective)
        if (persist) {
            runCatching { storage?.setLanguageTag(selection.tag) }
            runCatching { platformApply?.invoke(selection) }
        }
    }
}
