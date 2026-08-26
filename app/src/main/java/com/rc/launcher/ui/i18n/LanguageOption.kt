package com.rc.launcher.ui.i18n

import com.rc.launcher.core.RustBridge
import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.parseJson

/**
 * One row of the language picker (task 20, dynamic language loading).
 *
 * The picker used to be `AppLanguage.selectable` — a compiled-in enum — which
 * meant a community translation could not appear in it at all. A **language pack**
 * (`i18n::pack` in the core: a `.properties` file dropped into the app's `i18n/`
 * data directory) has no enum entry, so the picker is driven by this value object
 * instead: [builtin] is non-null for the three shipped catalogues and null for a
 * pack.
 *
 * Everything here comes from the core (`RustBridge.i18nLanguages`), so the picker
 * cannot disagree with what the core will actually render.
 */
data class LanguageOption(
    /** Canonical BCP-47 tag, or [SYSTEM_TAG] for "follow system". */
    val tag: String,
    /** Endonym — never translated, so a user who picked wrong can get back. */
    val nativeName: String,
    /** English name, for logs / bug reports. */
    val englishName: String = nativeName,
    /** The compiled-in language, or `null` when this is a dynamic pack. */
    val builtin: AppLanguage? = null,
    /** Fraction of the base catalogue this language translates (0.0–1.0). */
    val completeness: Float = 1f,
    /** Messages the catalogue itself provides. */
    val messages: Int = 0,
    /** True for the base (fallback) locale. */
    val base: Boolean = false,
    /** Right-to-left script. */
    val rtl: Boolean = false,
    /** Plural rule — a pack declares its own with `_meta.plural`. */
    val pluralRule: RcPluralRule = RcPluralRule.OTHER_ONLY,
    /** For a pack, the built-in language its untranslated keys fall back to. */
    val parent: AppLanguage = AppLanguage.BASE,
) {
    /** True when this language was loaded at runtime rather than compiled in. */
    val dynamic: Boolean get() = builtin == null && tag != SYSTEM_TAG

    /** True for the synthetic "follow system" row. */
    val isSystem: Boolean get() = tag == SYSTEM_TAG

    /**
     * A partially translated pack is worth flagging in the UI: the rest renders in
     * Chinese, which is correct but surprising if the user is not told.
     */
    val partial: Boolean get() = dynamic && completeness < 0.999f

    /**
     * Would a device asking for [deviceTag] be served by this row?
     *
     * Language subtag equality, with an exact tag preferred by the caller — the
     * same rule the core's `pack::negotiate` uses, so "follow system" agrees with
     * what the core would pick.
     */
    fun matches(deviceTag: String?): Boolean {
        val wanted = deviceTag?.trim()?.lowercase()?.replace('_', '-') ?: return false
        val mine = tag.lowercase()
        if (mine == wanted) return true
        val myLang = mine.substringBefore('-')
        val theirLang = wanted.substringBefore('-')
        return myLang.isNotEmpty() && myLang == theirLang
    }

    companion object {
        /** The pseudo-tag [AppLanguage.SYSTEM] persists. */
        const val SYSTEM_TAG = "system"

        /** The "follow system" row (its label is translated, unlike the others). */
        fun system(): LanguageOption =
            LanguageOption(tag = SYSTEM_TAG, nativeName = "", builtin = AppLanguage.SYSTEM)

        /** A row for a compiled-in language. */
        fun of(language: AppLanguage): LanguageOption = LanguageOption(
            tag = language.tag,
            nativeName = language.nativeName,
            englishName = language.nativeName,
            builtin = language,
            base = language == AppLanguage.BASE,
            pluralRule = RcPluralRule.of(language),
        )

        /** The compiled-in fallback list, used when the core is unavailable. */
        fun builtins(): List<LanguageOption> =
            listOf(system()) + AppLanguage.catalogues.map { of(it) }

        /**
         * Parse `RustBridge.i18nLanguages()`.
         *
         * Returns `null` (rather than an empty list) when the payload is missing or
         * unusable, so the caller can fall back to [builtins] instead of rendering
         * an empty picker (task-19 degradation).
         */
        fun parseLanguages(raw: String?): List<LanguageOption>? {
            if (raw.isNullOrBlank()) return null
            val root = parseJson(raw) as? JsonValue.Obj ?: return null
            val list = root.entries["languages"] as? JsonValue.Arr ?: return null
            val out = ArrayList<LanguageOption>(list.items.size + 1)
            out.add(system())
            for (item in list.items) {
                val obj = item as? JsonValue.Obj ?: continue
                val tag = (obj.entries["tag"] as? JsonValue.Str)?.value?.takeIf { it.isNotBlank() }
                    ?: continue
                val native = (obj.entries["native_name"] as? JsonValue.Str)?.value
                    ?.takeIf { it.isNotBlank() } ?: tag
                val isDynamic = (obj.entries["dynamic"] as? JsonValue.Bool)?.value == true
                // A built-in row must map onto the enum; if it does not, the core
                // and the app disagree about what is shipped — treat it as dynamic
                // so the row still works instead of vanishing.
                val builtin = if (isDynamic) null else AppLanguage.fromTagOrNull(tag)
                out.add(
                    LanguageOption(
                        tag = tag,
                        nativeName = native,
                        englishName = (obj.entries["english_name"] as? JsonValue.Str)?.value
                            ?: native,
                        builtin = builtin,
                        completeness = (obj.entries["completeness"] as? JsonValue.Num)
                            ?.value?.toFloat() ?: 1f,
                        messages = (obj.entries["messages"] as? JsonValue.Num)?.value?.toInt() ?: 0,
                        base = (obj.entries["base"] as? JsonValue.Bool)?.value == true,
                        rtl = (obj.entries["rtl"] as? JsonValue.Bool)?.value == true,
                        pluralRule = RcPluralRule.parse(
                            (obj.entries["plural"] as? JsonValue.Str)?.value,
                        ),
                        parent = AppLanguage.fromTagOrNull(
                            (obj.entries["parent"] as? JsonValue.Str)?.value,
                        ) ?: AppLanguage.BASE,
                    ),
                )
            }
            return out.takeIf { it.size > 1 }
        }

        /** The `loaded` tags from `RustBridge.i18nLanguagePacks`. */
        fun parseLoaded(raw: String?): List<String> {
            val root = parseJson(raw ?: return emptyList()) as? JsonValue.Obj ?: return emptyList()
            val arr = root.entries["loaded"] as? JsonValue.Arr ?: return emptyList()
            return arr.items.mapNotNull { (it as? JsonValue.Str)?.value }
        }

        /** `skipped` reasons from `RustBridge.i18nLanguagePacks` — shown verbatim. */
        fun parseSkipped(raw: String?): List<String> {
            val root = parseJson(raw ?: return emptyList()) as? JsonValue.Obj ?: return emptyList()
            val arr = root.entries["skipped"] as? JsonValue.Arr ?: return emptyList()
            return arr.items.mapNotNull { (it as? JsonValue.Str)?.value }
        }
    }
}

/**
 * The slice of [RustBridge] used to manage dynamic language packs — an interface
 * so a plain JVM unit test can exercise the engine without the native library.
 */
interface CoreLanguagePacks {
    /** `RustBridge.i18nLanguages()`. */
    fun languages(): String

    /** `RustBridge.i18nLanguagePacks(requestJson)`. */
    fun packs(requestJson: String): String
}

/** The real bridge. */
object RustBridgeLanguagePacks : CoreLanguagePacks {
    override fun languages(): String = RustBridge.i18nLanguages()
    override fun packs(requestJson: String): String = RustBridge.i18nLanguagePacks(requestJson)
}
