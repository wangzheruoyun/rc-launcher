package com.rc.launcher.ui.i18n

import android.content.Context
import com.rc.launcher.core.RustBridge
import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.parseJson

/**
 * Where a [RcStrings] table comes from (task 20).
 *
 * Two implementations, tried in order by [CompositeStringsSource]:
 *
 *  1. [CoreStringsSource] — the Rust core's catalogue. The core owns the
 *     resource files, so this is the *single source of truth* and it is
 *     overlay-aware (a hot-fixed or community translation shows up without an
 *     app update).
 *  2. [ResourcesStringsSource] — the generated `values-...` resources. Used when
 *     the native core is missing or throws, which is exactly the degraded case
 *     task 19 requires the UI to survive.
 */
interface StringsSource {
    /** Load the table for [language], or `null` when this source cannot serve it. */
    fun load(language: AppLanguage): RcStrings?
}

/**
 * A source that can also serve a **dynamically loaded language pack** (task 20).
 *
 * A pack has no [AppLanguage] entry, so it is addressed by tag; only the core can
 * resolve it (it owns the pack catalogue, its parent and its plural rule).
 * Implemented by [CoreStringsSource] and forwarded by [CompositeStringsSource];
 * [ResourcesStringsSource] cannot serve one (there are no generated resources for
 * a language that did not exist at build time) and returns `null`.
 */
interface PackAwareStringsSource : StringsSource {
    /** The fully resolved table for [option], or `null` when unavailable. */
    fun loadTag(option: LanguageOption): RcStrings?
}

/**
 * The slice of [RustBridge] the i18n framework needs — top level so it can be
 * faked in a plain JVM unit test without loading the native library.
 */
interface CoreI18nBridge {
    fun setLanguage(requestJson: String): String
    fun bundle(requestJson: String): String
}

/** The real bridge, backed by the JNI entry points of task 20. */
object RustBridgeI18n : CoreI18nBridge {
    override fun setLanguage(requestJson: String): String =
        RustBridge.i18nSetLanguage(requestJson)

    override fun bundle(requestJson: String): String =
        RustBridge.i18nBundle(requestJson)
}

/**
 * Reads the whole catalogue from the Rust core in a single JNI crossing
 * (`RustBridge.i18nBundle`) and also *applies* the language to the core, so
 * core-generated text (crash verdicts, errors) is localised too.
 */
class CoreStringsSource(
    private val bridge: CoreI18nBridge = RustBridgeI18n,
) : PackAwareStringsSource {

    /**
     * Load a dynamically loaded pack by tag.
     *
     * The core has already resolved the pack through its parent chain, so the
     * bundle is as complete as a built-in one — untranslated keys arrive as
     * Chinese, never as raw keys.
     */
    override fun loadTag(option: LanguageOption): RcStrings? = runCatching {
        bridge.setLanguage("""{"tag":"${option.tag}"}""")
        val raw = bridge.bundle("""{"language":"${option.tag}"}""")
        val messages = parseBundle(raw) ?: return null
        if (messages.isEmpty()) return null
        RcStrings.ofPack(
            tag = option.tag,
            parent = option.parent,
            messages = messages,
            // Prefer what the bundle reports; fall back to the picker row.
            pluralRule = parsePluralRule(raw) ?: option.pluralRule,
        )
    }.getOrNull()

    override fun load(language: AppLanguage): RcStrings? {
        // SYSTEM is resolved to a catalogue by the caller; guard anyway.
        val tag = (if (language.isSystem) AppLanguage.BASE else language).tag
        return runCatching {
            // Tell the core first: from now on its own messages are localised.
            bridge.setLanguage("""{"tag":"$tag"}""")
            val raw = bridge.bundle("""{"language":"$tag"}""")
            val messages = parseBundle(raw) ?: return null
            if (messages.isEmpty()) return null
            RcStrings.of(language, messages, RcStrings.Source.CORE)
        }.getOrNull() // UnsatisfiedLinkError / any native failure -> fall back.
    }

    companion object {
        /** The `plural` rule id reported by `i18nBundle`, when present. */
        fun parsePluralRule(raw: String?): RcPluralRule? {
            val root = parseJson(raw ?: return null) as? JsonValue.Obj ?: return null
            val id = (root.entries["plural"] as? JsonValue.Str)?.value ?: return null
            return RcPluralRule.parse(id)
        }

        /** Extract `{"language":...,"messages":{k:v,...}}` into a flat map. */
        fun parseBundle(raw: String?): Map<String, String>? {
            if (raw.isNullOrBlank()) return null
            val root = parseJson(raw) as? JsonValue.Obj ?: return null
            val messages = root.entries["messages"] as? JsonValue.Obj ?: return null
            val out = LinkedHashMap<String, String>(messages.entries.size)
            for ((k, v) in messages.entries) {
                if (v is JsonValue.Str) out[k] = v.value
            }
            return out
        }
    }
}

/**
 * Reads the generated Android string resources for a specific language.
 *
 * The lookup goes through a locale-specific [Context] (`createConfigurationContext`)
 * so the *chosen* language wins over the device language, and through the
 * generated [RcStringResources] id map so no reflection is involved.
 */
class ResourcesStringsSource(context: Context) : StringsSource {
    private val appContext = context.applicationContext

    override fun load(language: AppLanguage): RcStrings? = runCatching {
        val localized = RcLocaleContext.localizedContext(appContext, language)
        val out = LinkedHashMap<String, String>(RcStringResources.ids.size)
        for ((key, id) in RcStringResources.ids) {
            runCatching { localized.getString(id) }.getOrNull()?.let { out[key] = it }
        }
        if (out.isEmpty()) null else RcStrings.of(language, out, RcStrings.Source.RESOURCES)
    }.getOrNull()
}

/**
 * Tries each source in order and returns the first table that loads; if all of
 * them fail the caller still gets a non-null, *empty* table whose lookups echo
 * the key — the UI must never crash because a translation is unavailable.
 */
class CompositeStringsSource(private val sources: List<StringsSource>) : PackAwareStringsSource {
    override fun load(language: AppLanguage): RcStrings =
        sources.firstNotNullOfOrNull { runCatching { it.load(language) }.getOrNull() }
            ?: RcStrings.empty(language)

    /**
     * Only a [PackAwareStringsSource] can serve a pack; `null` when none can, so
     * [LocaleEngine] falls back to the pack's parent language instead of showing
     * an empty screen.
     */
    override fun loadTag(option: LanguageOption): RcStrings? =
        sources.filterIsInstance<PackAwareStringsSource>()
            .firstNotNullOfOrNull { runCatching { it.loadTag(option) }.getOrNull() }
}
