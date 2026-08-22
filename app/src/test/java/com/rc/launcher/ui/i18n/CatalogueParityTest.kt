package com.rc.launcher.ui.i18n

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

/**
 * Cross-checks the Kotlin side against the **shipped resource files** (task 20).
 *
 * `scripts/check_i18n.py` performs the full validation in CI; these tests make
 * the most important invariants fail in the *unit test* run as well, because that
 * is what a contributor runs locally:
 *
 *  * every [RcStringKeys] constant the UI references exists in every catalogue;
 *  * the generated `values-...` string resources cover the same keys;
 *  * the Kotlin `{name}` formatter agrees with the catalogue placeholders.
 *
 * The files are located by walking up from the module directory, so the tests
 * work from Gradle (`app/`) and from an IDE run configuration alike. If the
 * layout ever changes the tests skip rather than fail spuriously.
 */
class CatalogueParityTest {

    private val repoRoot: File? by lazy {
        var dir: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (dir != null) {
            if (File(dir, "rust/crates/rc-launcher-core/i18n/zh-CN.properties").exists()) return@lazy dir
            dir = dir.parentFile
        }
        null
    }

    /** Minimal `.properties` reader mirroring the Rust/Python parsers. */
    private fun readProperties(file: File): Map<String, String> {
        val out = LinkedHashMap<String, String>()
        var logical = StringBuilder()
        var pending = false
        file.readLines(Charsets.UTF_8).forEach { raw ->
            val line = raw.removeSuffix("\r")
            if (!pending) {
                val t = line.trimStart().removePrefix("\uFEFF")
                if (t.isEmpty() || t.startsWith("#") || t.startsWith("!")) return@forEach
                logical = StringBuilder(t)
            } else {
                logical.append(line.trimStart())
            }
            val trailing = logical.length - logical.toString().trimEnd('\\').length
            if (trailing % 2 == 1) {
                logical.setLength(logical.length - 1)
                pending = true
                return@forEach
            }
            pending = false
            val text = logical.toString()
            var idx = -1
            var escaped = false
            for ((i, c) in text.withIndex()) {
                if (escaped) { escaped = false; continue }
                if (c == '\\') escaped = true else if (c == '=') { idx = i; break }
            }
            if (idx < 0) return@forEach
            out[text.substring(0, idx).trim()] = text.substring(idx + 1).trim()
        }
        return out
    }

    private fun catalogue(tag: String): Map<String, String>? {
        val root = repoRoot ?: return null
        val f = File(root, "rust/crates/rc-launcher-core/i18n/$tag.properties")
        return if (f.exists()) readProperties(f) else null
    }

    @Test
    fun everyUiKeyExistsInEveryShippedCatalogue() {
        assumeTrue("repository layout not found", repoRoot != null)
        for (tag in listOf("zh-CN", "zh-Hant", "en")) {
            val entries = catalogue(tag)
            assumeTrue("catalogue $tag not found", entries != null)
            val missing = RcStringKeys.required.filterNot { entries!!.containsKey(it) }
            assertTrue("$tag is missing UI keys: $missing", missing.isEmpty())
        }
    }

    @Test
    fun theBaseCatalogueIsTheSupersetAndIsChinese() {
        assumeTrue(repoRoot != null)
        val base = catalogue("zh-CN") ?: return
        for (tag in listOf("zh-Hant", "en")) {
            val entries = catalogue(tag) ?: continue
            assertEquals(
                "$tag key set differs from the base catalogue",
                base.keys.sorted(),
                entries.keys.sorted(),
            )
        }
        // Chinese-first: the base catalogue is the one the UI falls back to.
        assertEquals(AppLanguage.ZH_CN, AppLanguage.BASE)
        assertTrue(base.getValue(RcStringKeys.NAV_HOME).isNotBlank())
    }

    @Test
    fun placeholdersAgreeAcrossCataloguesAndWithTheKotlinFormatter() {
        assumeTrue(repoRoot != null)
        val base = catalogue("zh-CN") ?: return
        for (tag in listOf("zh-Hant", "en")) {
            val entries = catalogue(tag) ?: continue
            for ((key, value) in base) {
                val translated = entries[key] ?: continue
                assertEquals(
                    "$tag / $key placeholder drift",
                    RcStringFormat.placeholders(value),
                    RcStringFormat.placeholders(translated),
                )
            }
        }
        // And the formatter really fills them in (no `{...}` left on screen).
        val table = RcStrings.of(AppLanguage.ZH_CN, base)
        val rendered = table.format(RcStringKeys.SETTINGS_LANGUAGE_APPLIED, "language" to "English")
        assertTrue(rendered, !rendered.contains('{'))
        assertTrue(rendered.contains("English"))
    }

    @Test
    fun generatedAndroidResourcesCoverEveryKey() {
        assumeTrue(repoRoot != null)
        val root = repoRoot!!
        val base = catalogue("zh-CN") ?: return
        for (dir in listOf("values", "values-en", "values-zh-rTW")) {
            val xml = File(root, "app/src/main/res/$dir/strings.xml")
            assumeTrue("$dir/strings.xml not generated yet", xml.exists())
            val text = xml.readText(Charsets.UTF_8)
            val names = Regex("<string name=\"([^\"]+)\"")
                .findAll(text).map { it.groupValues[1] }.toSet()
            val expected = base.keys.map { it.replace('.', '_') }.toSet()
            assertEquals("$dir/strings.xml key set", expected.sorted(), names.sorted())
        }
    }

    @Test
    fun theGeneratedResourceIdMapCoversTheSameKeys() {
        assumeTrue(repoRoot != null)
        val base = catalogue("zh-CN") ?: return
        // RcStringResources is generated from the base catalogue; the map must be
        // exactly as large, so a new key cannot be forgotten on the Android side.
        assertEquals(base.size, RcStringResources.size)
        val missing = base.keys.filterNot { RcStringResources.ids.containsKey(it) }
        assertTrue("keys absent from RcStringResources: $missing", missing.isEmpty())
    }
}
