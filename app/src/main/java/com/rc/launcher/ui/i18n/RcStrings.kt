package com.rc.launcher.ui.i18n

/**
 * `{name}` interpolation and plural selection — a faithful port of the Rust
 * core's `crate::i18n::format` (task 20).
 *
 * Both sides must agree, because the very same message may be rendered by the
 * core (a crash verdict, an error) or by Compose (UI chrome). The rules are
 * therefore identical, including the deliberate lenience:
 *
 *  * an **unknown** placeholder is left verbatim (`{foo}`) so the mistake shows
 *    up in the UI and in tests instead of silently deleting text;
 *  * an **unterminated** `{` is emitted verbatim;
 *  * `{{` / `}}` are literal braces;
 *  * extra arguments are ignored.
 */
object RcStringFormat {

    /** CLDR plural categories used by the shipped languages. */
    enum class Plural(val suffix: String) { ONE("one"), OTHER("other") }

    /** The plural category of [count] in [language] (CLDR cardinal rules). */
    fun pluralCategory(language: AppLanguage, count: Long): Plural = when (language) {
        // English distinguishes exactly 1; 0 and negatives are `other`.
        AppLanguage.EN -> if (count == 1L) Plural.ONE else Plural.OTHER
        // Chinese has a single form. SYSTEM is resolved before we get here, but
        // treat it as Chinese (the base locale) for safety.
        else -> Plural.OTHER
    }

    /** `"download.files"` + 1 -> `"download.files.one"`. */
    fun pluralKey(language: AppLanguage, base: String, count: Long): String =
        "$base.${pluralCategory(language, count).suffix}"

    /** Substitute `{name}` placeholders in [template] from [args]. */
    fun interpolate(template: String, args: Map<String, String>): String {
        if (!template.contains('{') && !template.contains('}')) return template
        val out = StringBuilder(template.length + 16)
        var i = 0
        while (i < template.length) {
            val c = template[i]
            when {
                c == '{' && i + 1 < template.length && template[i + 1] == '{' -> {
                    out.append('{'); i += 2
                }
                c == '{' -> {
                    val end = template.indexOf('}', startIndex = i + 1)
                    if (end < 0) {
                        // Unterminated: emit the remainder verbatim.
                        out.append(template, i, template.length)
                        i = template.length
                    } else {
                        val name = template.substring(i + 1, end)
                        val value = args[name]
                        if (value != null) out.append(value) else out.append('{').append(name).append('}')
                        i = end + 1
                    }
                }
                c == '}' && i + 1 < template.length && template[i + 1] == '}' -> {
                    out.append('}'); i += 2
                }
                else -> {
                    out.append(c); i++
                }
            }
        }
        return out.toString()
    }

    /** The `{name}` placeholder set of [template] (used by the consistency tests). */
    fun placeholders(template: String): Set<String> {
        val out = LinkedHashSet<String>()
        var i = 0
        while (i < template.length) {
            if (template[i] == '{') {
                if (i + 1 < template.length && template[i + 1] == '{') {
                    i += 2
                    continue
                }
                val end = template.indexOf('}', startIndex = i + 1)
                if (end < 0) break
                val name = template.substring(i + 1, end)
                if (name.isNotBlank()) out.add(name)
                i = end + 1
                continue
            }
            i++
        }
        return out
    }
}

/**
 * An immutable, resolved string table for one language (task 20).
 *
 * This is what Compose reads. It is intentionally a plain map plus the
 * [language] it belongs to, so it is cheap to hold in a `StateFlow`, trivial to
 * snapshot for a `@Preview`, and unit-testable without Android.
 *
 * Resolution order (see [RcStringsLoader]):
 *  1. the live catalogue handed over by the **Rust core** (`i18nBundle`) — the
 *     single source of truth, overlay-aware, so a hot-fixed translation appears
 *     without an app update;
 *  2. the generated **Android resources** (the `values-...` files) when the
 *     native core is unavailable (task-19 degradation);
 *  3. the key itself, which keeps the UI readable and the bug greppable.
 */
data class RcStrings(
    val language: AppLanguage,
    private val messages: Map<String, String>,
    /** Where the table came from — surfaced in the settings diagnostics card. */
    val source: Source = Source.CORE,
) {
    enum class Source { CORE, RESOURCES, EMPTY }

    /** Number of messages in the table. */
    val size: Int get() = messages.size

    /** True when [key] resolves. */
    fun has(key: String): Boolean = messages.containsKey(key)

    /**
     * The message for [key], or the key itself when it is missing (never throws,
     * never returns null — a missing string must not blank out the UI).
     */
    operator fun get(key: String): String = messages[key] ?: key

    /** [get] plus `{name}` interpolation. */
    fun format(key: String, vararg args: Pair<String, String>): String =
        RcStringFormat.interpolate(get(key), args.toMap())

    /** [get] plus `{name}` interpolation from a map. */
    fun format(key: String, args: Map<String, String>): String =
        RcStringFormat.interpolate(get(key), args)

    /**
     * Plural-aware lookup: picks `<base>.one` / `<base>.other` per [language]'s
     * rules and supplies `{count}` automatically.
     */
    fun plural(base: String, count: Long, vararg args: Pair<String, String>): String {
        val key = RcStringFormat.pluralKey(language, base, count)
        val merged = HashMap<String, String>(args.size + 1)
        merged["count"] = count.toString()
        for ((k, v) in args) merged[k] = v
        return RcStringFormat.interpolate(get(key), merged)
    }

    /** The keys this table knows, sorted — for diagnostics and tests. */
    fun keys(): List<String> = messages.keys.sorted()

    /** The keys of [required] this table cannot resolve. */
    fun missingFrom(required: Collection<String>): List<String> =
        required.filterNot { has(it) }.sorted()

    companion object {
        /** An empty table: every lookup echoes the key. Used before the first load. */
        fun empty(language: AppLanguage = AppLanguage.BASE): RcStrings =
            RcStrings(language, emptyMap(), Source.EMPTY)

        /** A table built from an explicit map (tests, previews). */
        fun of(language: AppLanguage, messages: Map<String, String>, source: Source = Source.CORE): RcStrings =
            RcStrings(language, messages, source)
    }
}
