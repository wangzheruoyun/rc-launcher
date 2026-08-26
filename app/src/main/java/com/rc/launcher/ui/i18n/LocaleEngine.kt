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

    /**
     * Every row the picker should show: the compiled-in catalogues **plus every
     * dynamically loaded language pack** (task 20).
     *
     * Comes from the core, so the picker can never offer a language the core
     * cannot render. Falls back to the compiled-in list when the native core is
     * unavailable.
     */
    private val _languages = MutableStateFlow(LanguageOption.builtins())
    val languages: StateFlow<List<LanguageOption>> = _languages.asStateFlow()

    /**
     * The tag actually being rendered — a pack tag (`ja`) or a built-in (`zh-CN`).
     *
     * [effective] cannot express a pack (it is an enum), so this is what the
     * picker highlights.
     */
    private val _effectiveTag = MutableStateFlow(AppLanguage.BASE.tag)
    val effectiveTag: StateFlow<String> = _effectiveTag.asStateFlow()

    /**
     * Human-readable reasons the core refused a pack file (too large, tag collides
     * with a built-in language, no messages, …). Surfaced in settings, because
     * "my translation does not show up" is otherwise unanswerable.
     */
    private val _packProblems = MutableStateFlow(emptyList<String>())
    val packProblems: StateFlow<List<String>> = _packProblems.asStateFlow()

    @Volatile
    private var packBridge: CoreLanguagePacks? = null

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
            packBridge = RustBridgeLanguagePacks,
        )
        // Dynamic language loading: pick up any community pack the user dropped
        // into <files>/i18n/ before the first frame, so it is already in the
        // picker (and already selectable by the device locale).
        runCatching { loadLanguagePacks(java.io.File(app.filesDir, "i18n").absolutePath) }
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
        packBridge: CoreLanguagePacks? = null,
    ) {
        this.storage = storage
        this.source = source
        this.systemTags = systemTags
        this.platformApply = platformApply
        this.packBridge = packBridge
        refreshLanguages()
        // Restore, without re-persisting (a fresh install must stay on SYSTEM).
        // The saved value may be a *pack* tag, which is not an AppLanguage.
        applyTag(storage.getLanguageTag(), persist = false)
    }

    /**
     * Load every `*.properties` language pack in [dir] and refresh the picker.
     *
     * This is the "dynamic language loading" entry point: a user drops a
     * community translation into the app's `i18n/` directory and it becomes a
     * selectable language, no rebuild and no app update. Returns the tags loaded.
     *
     * Never throws — a missing directory or an unusable file leaves the picker
     * exactly as it was and records the reason in [packProblems].
     */
    fun loadLanguagePacks(dir: String): List<String> {
        val bridge = packBridge ?: return emptyList()
        val raw = runCatching {
            bridge.packs("""{"action":"load","path":${quoteJson(dir)}}""")
        }.getOrNull()
        _packProblems.value = LanguageOption.parseSkipped(raw)
        refreshLanguages()
        // A pack may have arrived for the language the user asked to follow.
        if (_selected.value.isSystem) applyTag(LanguageOption.SYSTEM_TAG, persist = false)
        return LanguageOption.parseLoaded(raw)
    }

    /**
     * Unload a pack. If it was the one on screen the core reverts to its parent,
     * so this re-reads the table rather than assuming the selection survived.
     */
    fun removeLanguagePack(tag: String): Boolean {
        val bridge = packBridge ?: return false
        val ok = runCatching {
            bridge.packs("""{"action":"remove","tag":${quoteJson(tag)}}""")
        }.getOrNull() != null
        refreshLanguages()
        // The removed pack may have been the active one.
        if (_effectiveTag.value == tag) {
            applyTag(LanguageOption.SYSTEM_TAG, persist = true)
        } else {
            reload()
        }
        return ok
    }

    /** Re-read the language list from the core (after loading/removing packs). */
    fun refreshLanguages() {
        val fromCore = runCatching { packBridge?.languages() }
            .getOrNull()
            ?.let { LanguageOption.parseLanguages(it) }
        _languages.value = fromCore ?: LanguageOption.builtins()
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

    /**
     * Switch by tag (`"system"`, `"zh-CN"`, `"ja"`, …).
     *
     * A tag naming a **loaded language pack** selects that pack; anything
     * unrecognised means "follow system", so a stale persisted tag (a pack the
     * user has since deleted) degrades instead of dead-ending.
     */
    fun setLanguageTag(tag: String?) = applyTag(tag, persist = true)

    /** The picker row for [tag], when it exists. */
    fun optionFor(tag: String?): LanguageOption? =
        _languages.value.firstOrNull { it.tag.equals(tag?.trim(), ignoreCase = true) }

    /**
     * Resolve + apply a tag that may name a pack.
     *
     * Built-in tags (and `"system"`) go down the existing [apply] path unchanged;
     * a pack tag goes through the core, which is the only thing that knows the
     * pack's catalogue, parent and plural rule.
     */
    private fun applyTag(tag: String?, persist: Boolean) {
        val option = optionFor(tag)
        if (option != null && option.dynamic) {
            applyPack(option, persist)
            return
        }
        apply(AppLanguage.fromTag(tag), persist)
    }

    private fun applyPack(option: LanguageOption, persist: Boolean) {
        // The core owns pack resolution; ask it for the fully-resolved table.
        val table = runCatching { (source as? PackAwareStringsSource)?.loadTag(option) }
            .getOrNull()
        if (table == null) {
            // Cannot render the pack (no native core): fall back to its parent
            // rather than showing a half-translated or empty screen.
            apply(option.parent, persist)
            return
        }
        _selected.value = option.parent
        _effective.value = option.parent
        _effectiveTag.value = option.tag
        _strings.value = table
        if (persist) {
            runCatching { storage?.setLanguageTag(option.tag) }
            // The platform per-app locale list only knows real system locales; a
            // pack is ours alone, so hand the platform the parent language.
            runCatching { platformApply?.invoke(option.parent) }
        }
    }

    /** Re-load the current table — e.g. after installing a translation overlay. */
    fun reload() = apply(_selected.value, persist = false)

    private fun apply(selection: AppLanguage, persist: Boolean) {
        val effective = resolve(selection)
        // "Follow system" may resolve onto a *pack* when the device asks for a
        // language only a pack provides — that is the whole point of loading one.
        if (selection.isSystem) {
            val fromDevice = systemTags.firstNotNullOfOrNull { tag ->
                _languages.value.firstOrNull { it.dynamic && it.matches(tag) }
            }
            if (fromDevice != null && AppLanguage.negotiateList(systemTags) == AppLanguage.BASE) {
                applyPack(fromDevice, persist)
                _selected.value = AppLanguage.SYSTEM
                if (persist) runCatching { storage?.setLanguageTag(AppLanguage.SYSTEM.tag) }
                return
            }
        }
        _selected.value = selection
        _effective.value = effective
        _effectiveTag.value = effective.tag
        // A failure here must never break the switch: fall back to an empty table
        // (lookups echo the key) rather than leaving a stale language on screen.
        _strings.value = runCatching { source?.load(effective) }.getOrNull()
            ?: RcStrings.empty(effective)
        if (persist) {
            runCatching { storage?.setLanguageTag(selection.tag) }
            runCatching { platformApply?.invoke(selection) }
        }
    }

    /** Minimal JSON string literal — enough for a path or a tag. */
    private fun quoteJson(text: String): String {
        val sb = StringBuilder(text.length + 2)
        sb.append('"')
        for (c in text) {
            when (c) {
                '"' -> sb.append("\\\"")
                '\\' -> sb.append("\\\\")
                '\n' -> sb.append("\\n")
                '\r' -> sb.append("\\r")
                '\t' -> sb.append("\\t")
                else -> if (c < ' ') sb.append("\\u%04x".format(c.code)) else sb.append(c)
            }
        }
        sb.append('"')
        return sb.toString()
    }
}
