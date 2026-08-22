package com.rc.launcher.ui.i18n

/**
 * A language the launcher's UI can be switched to (task 20).
 *
 * Mirrors the Rust core's `crate::i18n::Language` **plus** a [SYSTEM] entry: the
 * core only knows concrete catalogues, while the UI additionally has to model
 * "follow the device language". Keeping the two enums in lock-step is asserted
 * by the unit tests (the tags and the Android qualifiers must match
 * `RustBridge.i18nLanguages()`).
 *
 * The launcher is **Chinese-first** (中文优先): [ZH_CN] is the base catalogue, it
 * owns the default `values/` resource directory, and every unknown device locale
 * resolves to it.
 *
 * @param tag canonical BCP-47 tag, or `"system"` for [SYSTEM]. This is the value
 *   persisted by [LocaleStorage], so it must stay stable across releases.
 * @param nativeName the language's *endonym* — deliberately never translated so
 *   the picker stays readable whatever the current language is.
 * @param androidQualifier the `values-<qualifier>` directory holding this
 *   catalogue, or `null` for the default (Chinese-first) one.
 */
enum class AppLanguage(
    val tag: String,
    val nativeName: String,
    val englishName: String,
    val androidQualifier: String?,
) {
    /** Follow the device language, negotiated against the shipped catalogues. */
    SYSTEM("system", "跟随系统", "Follow system", null),

    /** 简体中文 — the base catalogue (`values/`). */
    ZH_CN("zh-CN", "简体中文", "Simplified Chinese", null),

    /** 繁體中文 (`values-zh-rTW/`). */
    ZH_HANT("zh-Hant", "繁體中文", "Traditional Chinese", "zh-rTW"),

    /** English (`values-en/`). */
    EN("en", "English", "English", "en");

    /** True for the pseudo "follow the system" entry. */
    val isSystem: Boolean get() = this == SYSTEM

    /**
     * The BCP-47 tag to hand to Android's per-app locale APIs
     * (`LocaleManager.setApplicationLocales`). `null` means "reset to system",
     * which is exactly what an empty `LocaleList` does.
     */
    val platformTag: String?
        get() = when (this) {
            SYSTEM -> null
            ZH_HANT -> "zh-Hant-TW" // matches values-zh-rTW
            else -> tag
        }

    companion object {
        /** The base (fallback) catalogue — Chinese-first. */
        val BASE: AppLanguage = ZH_CN

        /** Selectable entries, "follow system" first, then the catalogues. */
        val selectable: List<AppLanguage> = listOf(SYSTEM, ZH_CN, ZH_HANT, EN)

        /** Only the concrete catalogues (what the Rust core knows about). */
        val catalogues: List<AppLanguage> = listOf(ZH_CN, ZH_HANT, EN)

        /**
         * Parse a persisted [tag]. Unknown / blank values degrade to [SYSTEM] so
         * a corrupt preference file can never leave the UI untranslated.
         */
        fun fromTag(tag: String?): AppLanguage {
            if (tag.isNullOrBlank()) return SYSTEM
            val t = tag.trim()
            return entries.firstOrNull { it.tag.equals(t, ignoreCase = true) } ?: SYSTEM
        }

        /**
         * Resolve one device locale tag onto a shipped catalogue, or `null` when
         * we ship nothing for that language.
         *
         * Deliberately a faithful port of the Rust `Language::negotiate` so both
         * sides pick the same catalogue for the same device:
         *  * script beats region (`zh-Hant-CN` is Traditional, `zh-Hans-TW` is not);
         *  * `zh-TW` / `zh-HK` / `zh-MO` (and bare `yue`) are Traditional;
         *  * every other `zh` (including `zh-SG`, `zh-Hans`) is Simplified;
         *  * legacy `_` separators, `.charset` / `@modifier` suffixes and casing
         *    are all accepted, because that is what real devices send.
         */
        fun negotiate(tag: String?): AppLanguage? {
            val parsed = LanguageTagParts.parse(tag) ?: return null
            return when (parsed.language) {
                "zh", "cmn", "yue" -> {
                    val traditional = when (parsed.script) {
                        "hant" -> true
                        "hans" -> false
                        else -> parsed.region in TRADITIONAL_REGIONS ||
                            (parsed.language == "yue" && parsed.region == null)
                    }
                    if (traditional) ZH_HANT else ZH_CN
                }
                "en" -> EN
                else -> null
            }
        }

        /**
         * Best catalogue for an ordered list of device preferences (an Android
         * `LocaleList`), falling back to the Chinese-first [BASE].
         */
        fun negotiateList(preferred: List<String?>): AppLanguage =
            preferred.firstNotNullOfOrNull { negotiate(it) } ?: BASE

        private val TRADITIONAL_REGIONS = setOf("tw", "hk", "mo")
    }
}

/**
 * The subtags of a lenient BCP-47 tag — a port of the Rust `LanguageTag`.
 *
 * Extracted so [AppLanguage.negotiate] stays readable *and* so the parsing rules
 * can be unit-tested directly against the Rust test-cases.
 */
data class LanguageTagParts(
    val language: String,
    val script: String?,
    val region: String?,
) {
    companion object {
        fun parse(tag: String?): LanguageTagParts? {
            if (tag == null) return null
            // Drop a `.charset` / `@modifier` suffix, accept `_` for `-`.
            val cleaned = tag.trim()
                .substringBefore('.')
                .substringBefore('@')
                .replace('_', '-')
            val parts = cleaned.split('-').filter { it.isNotEmpty() }
            val language = parts.firstOrNull()?.lowercase() ?: return null
            if (language.length !in 2..3 || !language.all { it in 'a'..'z' }) return null

            var script: String? = null
            var region: String? = null
            for (raw in parts.drop(1)) {
                val p = raw.lowercase()
                val alpha = p.all { it in 'a'..'z' }
                val digit = p.all { it.isDigit() }
                when {
                    p.length == 4 && alpha && script == null -> script = p
                    (p.length == 2 && alpha) || (p.length == 3 && digit) ->
                        if (region == null) region = p
                    // Variants / extensions / private use are ignored.
                }
            }
            return LanguageTagParts(language, script, region)
        }
    }
}
