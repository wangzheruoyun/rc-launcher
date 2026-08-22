package com.rc.launcher.ui.i18n

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The string table + formatter (task 20).
 *
 * The interpolation cases mirror the Rust `crate::i18n::format::tests` one for
 * one: the same message may be rendered by the core or by Compose, so both
 * implementations must agree — including on the deliberate lenience.
 */
class RcStringsTest {

    private val zh = RcStrings.of(
        AppLanguage.ZH_CN,
        mapOf(
            "nav.home" to "主页",
            "error.checksum" to "文件校验失败：{path}",
            "settings.language.applied" to "界面语言已切换为{language}",
            "download.files.one" to "共 {count} 个文件",
            "download.files.other" to "共 {count} 个文件",
        ),
    )

    private val en = RcStrings.of(
        AppLanguage.EN,
        mapOf(
            "nav.home" to "Home",
            "download.files.one" to "{count} file",
            "download.files.other" to "{count} files",
        ),
    )

    @Test
    fun resolvesKnownKeys() {
        assertEquals("主页", zh["nav.home"])
        assertTrue(zh.has("nav.home"))
        assertEquals(AppLanguage.ZH_CN, zh.language)
        assertEquals(5, zh.size)
    }

    @Test
    fun missingKeyEchoesTheKeyInsteadOfBlankingTheUi() {
        assertEquals("no.such.key", zh["no.such.key"])
        assertFalse(zh.has("no.such.key"))
        // An empty table must still render something for every key.
        val empty = RcStrings.empty()
        assertEquals("nav.home", empty["nav.home"])
        assertEquals(RcStrings.Source.EMPTY, empty.source)
        assertEquals(0, empty.size)
    }

    @Test
    fun interpolatesNamedPlaceholders() {
        assertEquals(
            "文件校验失败：/sdcard/mods/a.jar",
            zh.format("error.checksum", "path" to "/sdcard/mods/a.jar"),
        )
        assertEquals(
            "界面语言已切换为English",
            zh.format("settings.language.applied", mapOf("language" to "English")),
        )
    }

    @Test
    fun keepsUnknownPlaceholdersVisibleAndIgnoresExtras() {
        // Losing the text silently would hide the bug; keep it greppable.
        assertEquals("文件校验失败：{path}", zh.format("error.checksum"))
        assertEquals(
            "文件校验失败：/x",
            zh.format("error.checksum", "path" to "/x", "unused" to "y"),
        )
    }

    @Test
    fun handlesEscapedStrayAndUnterminatedBraces() {
        assertEquals("{literal}", RcStringFormat.interpolate("{{literal}}", emptyMap()))
        assertEquals("{X}", RcStringFormat.interpolate("{{{a}}}", mapOf("a" to "X")))
        assertEquals("100%} done", RcStringFormat.interpolate("100%} done", emptyMap()))
        assertEquals("oops {name", RcStringFormat.interpolate("oops {name", mapOf("name" to "x")))
        assertEquals("{", RcStringFormat.interpolate("{", emptyMap()))
        assertEquals("", RcStringFormat.interpolate("", emptyMap()))
    }

    @Test
    fun isMultibyteSafe() {
        assertEquals(
            "路径：/存储/a.jar（已损坏）",
            RcStringFormat.interpolate("路径：{path}（已损坏）", mapOf("path" to "/存储/a.jar")),
        )
        assertEquals("表情🎮！🎮", RcStringFormat.interpolate("表情🎮{x}🎮", mapOf("x" to "！")))
    }

    @Test
    fun extractsPlaceholderSets() {
        assertEquals(setOf("a", "b"), RcStringFormat.placeholders("{b} then {a} then {b}"))
        assertTrue(RcStringFormat.placeholders("no placeholders").isEmpty())
        assertTrue(RcStringFormat.placeholders("{{a}}").isEmpty())
        assertTrue(RcStringFormat.placeholders("{}").isEmpty())
        assertTrue(RcStringFormat.placeholders("{ }").isEmpty())
        assertTrue(RcStringFormat.placeholders("{oops").isEmpty())
    }

    @Test
    fun pluralRulesFollowTheLanguage() {
        // English distinguishes one/other ...
        assertEquals("1 file", en.plural("download.files", 1))
        assertEquals("3 files", en.plural("download.files", 3))
        assertEquals("0 files", en.plural("download.files", 0))
        assertEquals("-1 files", en.plural("download.files", -1))
        // ... Chinese has a single form.
        assertEquals("共 1 个文件", zh.plural("download.files", 1))
        assertEquals("共 7 个文件", zh.plural("download.files", 7))
        assertEquals(
            RcStringFormat.Plural.ONE,
            RcStringFormat.pluralCategory(AppLanguage.EN, 1),
        )
        assertEquals(
            RcStringFormat.Plural.OTHER,
            RcStringFormat.pluralCategory(AppLanguage.ZH_HANT, 1),
        )
        assertEquals(
            "download.files.other",
            RcStringFormat.pluralKey(AppLanguage.ZH_CN, "download.files", 1),
        )
    }

    @Test
    fun reportsMissingKeysForDiagnostics() {
        val missing = zh.missingFrom(listOf("nav.home", "nav.settings", "a.b"))
        assertEquals(listOf("a.b", "nav.settings"), missing)
        assertTrue(zh.missingFrom(listOf("nav.home")).isEmpty())
        assertEquals(listOf("download.files.one", "download.files.other", "error.checksum", "nav.home", "settings.language.applied"), zh.keys())
    }
}
