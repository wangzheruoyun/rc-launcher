package com.rc.launcher.ui.i18n

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Locale negotiation (task 20).
 *
 * These cases are deliberately the *same* ones the Rust core asserts in
 * `crate::i18n::language::tests`, because a device must resolve to the same
 * catalogue on both sides of the FFI.
 */
class AppLanguageTest {

    @Test
    fun baseLocaleIsChineseAndListedFirst() {
        assertEquals(AppLanguage.ZH_CN, AppLanguage.BASE)
        assertEquals(AppLanguage.SYSTEM, AppLanguage.selectable.first())
        assertEquals(AppLanguage.ZH_CN, AppLanguage.catalogues.first())
        assertEquals(3, AppLanguage.catalogues.size)
        // Chinese-first: the base catalogue owns the default `values/` directory.
        assertNull(AppLanguage.ZH_CN.androidQualifier)
        assertEquals("zh-rTW", AppLanguage.ZH_HANT.androidQualifier)
        assertEquals("en", AppLanguage.EN.androidQualifier)
    }

    @Test
    fun tagsRoundTripAndUnknownTagsMeanSystem() {
        for (l in AppLanguage.entries) {
            assertEquals(l, AppLanguage.fromTag(l.tag))
        }
        assertEquals(AppLanguage.ZH_CN, AppLanguage.fromTag("ZH-cn"))
        // Anything we cannot make sense of must not leave the UI untranslated.
        assertEquals(AppLanguage.SYSTEM, AppLanguage.fromTag(null))
        assertEquals(AppLanguage.SYSTEM, AppLanguage.fromTag(""))
        assertEquals(AppLanguage.SYSTEM, AppLanguage.fromTag("   "))
        assertEquals(AppLanguage.SYSTEM, AppLanguage.fromTag("klingon"))
    }

    @Test
    fun negotiatesSimplifiedVersusTraditionalChinese() {
        for (tag in listOf("zh", "zh-CN", "zh_CN", "zh-Hans", "zh-Hans-TW", "zh-SG", "cmn-Hans")) {
            assertEquals("tag=$tag", AppLanguage.ZH_CN, AppLanguage.negotiate(tag))
        }
        for (tag in listOf("zh-TW", "zh_TW", "zh-HK", "zh-MO", "zh-Hant", "zh-Hant-CN", "yue")) {
            assertEquals("tag=$tag", AppLanguage.ZH_HANT, AppLanguage.negotiate(tag))
        }
    }

    @Test
    fun scriptBeatsRegionBothWays() {
        assertEquals(AppLanguage.ZH_CN, AppLanguage.negotiate("zh-Hans-TW"))
        assertEquals(AppLanguage.ZH_HANT, AppLanguage.negotiate("zh-Hant-CN"))
    }

    @Test
    fun negotiatesEnglishAndRejectsUnshippedLanguages() {
        for (tag in listOf("en", "en-US", "en_GB", "EN-au")) {
            assertEquals("tag=$tag", AppLanguage.EN, AppLanguage.negotiate(tag))
        }
        for (tag in listOf("fr", "de-DE", "ja", "ko", "ru", "", "xx", null)) {
            assertNull("tag=$tag", AppLanguage.negotiate(tag))
        }
    }

    @Test
    fun acceptsLegacyAndPosixStyleTags() {
        assertEquals(AppLanguage.ZH_CN, AppLanguage.negotiate("zh_CN.UTF-8"))
        assertEquals(AppLanguage.ZH_CN, AppLanguage.negotiate("zh-CN@pinyin"))
        assertEquals(AppLanguage.ZH_HANT, AppLanguage.negotiate("zh-Hant-TW-u-ca-chinese"))
    }

    @Test
    fun negotiateListHonoursOrderAndFallsBackToChinese() {
        assertEquals(
            AppLanguage.EN,
            AppLanguage.negotiateList(listOf("ja-JP", "en-US", "zh-CN")),
        )
        assertEquals(AppLanguage.ZH_HANT, AppLanguage.negotiateList(listOf("zh-HK", "en")))
        // Chinese-first fallback for an entirely unsupported device.
        assertEquals(AppLanguage.BASE, AppLanguage.negotiateList(listOf("ja", "ko")))
        assertEquals(AppLanguage.BASE, AppLanguage.negotiateList(emptyList()))
        assertEquals(AppLanguage.BASE, AppLanguage.negotiateList(listOf(null, "")))
    }

    @Test
    fun rejectsTagsWithoutAUsableLanguageSubtag() {
        for (bad in listOf("", "   ", "-", "_", "1", "x", "C", "POSIX", "12-CN", "@euro")) {
            assertNull("bad=$bad", LanguageTagParts.parse(bad))
            assertNull("bad=$bad", AppLanguage.negotiate(bad))
        }
        assertNull(LanguageTagParts.parse("C"))
        assertNull(LanguageTagParts.parse("POSIX"))
        assertNull(LanguageTagParts.parse("12"))
        assertNull(LanguageTagParts.parse(null))
    }

    @Test
    fun parsesSubtagsLikeTheRustCore() {
        val p = LanguageTagParts.parse("ZH_hant_tw")!!
        assertEquals("zh", p.language)
        assertEquals("hant", p.script)
        assertEquals("tw", p.region)
        // Numeric (UN M.49) regions are accepted; variants are ignored.
        assertEquals("419", LanguageTagParts.parse("es-419")!!.region)
        assertNull(LanguageTagParts.parse("en")!!.region)
    }

    @Test
    fun platformTagsMatchTheResourceQualifiers() {
        // SYSTEM resets the platform's per-app locale to "follow the system".
        assertNull(AppLanguage.SYSTEM.platformTag)
        assertEquals("zh-CN", AppLanguage.ZH_CN.platformTag)
        // Must agree with values-zh-rTW.
        assertEquals("zh-Hant-TW", AppLanguage.ZH_HANT.platformTag)
        assertEquals("en", AppLanguage.EN.platformTag)
        assertTrue(AppLanguage.SYSTEM.isSystem)
        assertFalse(AppLanguage.ZH_CN.isSystem)
    }

    @Test
    fun endonymsAreNeverTranslated() {
        // The picker must stay readable even when the current language is one the
        // user cannot read (that is how they get *out* of a misclick).
        assertEquals("简体中文", AppLanguage.ZH_CN.nativeName)
        assertEquals("繁體中文", AppLanguage.ZH_HANT.nativeName)
        assertEquals("English", AppLanguage.EN.nativeName)
        assertTrue(AppLanguage.entries.all { it.nativeName.isNotBlank() })
        assertTrue(AppLanguage.entries.all { it.englishName.isNotBlank() })
    }
}
