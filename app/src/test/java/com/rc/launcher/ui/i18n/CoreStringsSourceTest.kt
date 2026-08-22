package com.rc.launcher.ui.i18n

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Parsing the Rust core's `i18nBundle` payload (task 20).
 *
 * The core is the single source of truth for user-facing copy, so this is the
 * primary path; it must be tolerant of every failure mode the FFI can produce
 * (missing library, malformed JSON, empty catalogue) because the fallback to
 * Android resources depends on returning `null` rather than throwing.
 */
class CoreStringsSourceTest {

    /** A scripted stand-in for the JNI bridge. */
    private class FakeBridge(
        private val bundleJson: String,
        private val throwOnBundle: Throwable? = null,
    ) : CoreI18nBridge {
        val setLanguageCalls = mutableListOf<String>()
        val bundleCalls = mutableListOf<String>()

        override fun setLanguage(requestJson: String): String {
            setLanguageCalls += requestJson
            return """{"tag":"zh-CN"}"""
        }

        override fun bundle(requestJson: String): String {
            bundleCalls += requestJson
            throwOnBundle?.let { throw it }
            return bundleJson
        }
    }

    @Test
    fun parsesABundleIntoAStringTable() {
        val bridge = FakeBridge(
            """{"language":"en","messages":{"nav.home":"Home","nav.settings":"Settings"}}""",
        )
        val strings = CoreStringsSource(bridge).load(AppLanguage.EN)!!
        assertEquals(AppLanguage.EN, strings.language)
        assertEquals(RcStrings.Source.CORE, strings.source)
        assertEquals("Home", strings["nav.home"])
        assertEquals(2, strings.size)
    }

    @Test
    fun appliesTheLanguageToTheCoreBeforeReadingIt() {
        // Order matters: the core must be switched first so its *own* messages
        // (crash verdicts, errors) are localised too.
        val bridge = FakeBridge("""{"language":"zh-Hant","messages":{"nav.home":"主頁"}}""")
        CoreStringsSource(bridge).load(AppLanguage.ZH_HANT)
        assertEquals(listOf("""{"tag":"zh-Hant"}"""), bridge.setLanguageCalls)
        assertEquals(listOf("""{"language":"zh-Hant"}"""), bridge.bundleCalls)
    }

    @Test
    fun theSystemPseudoLanguageIsRequestedAsTheBaseCatalogue() {
        val bridge = FakeBridge("""{"language":"zh-CN","messages":{"nav.home":"主页"}}""")
        CoreStringsSource(bridge).load(AppLanguage.SYSTEM)
        assertTrue(bridge.bundleCalls.single().contains(AppLanguage.BASE.tag))
    }

    @Test
    fun returnsNullSoTheResourceFallbackCanTakeOver() {
        // Malformed / empty / wrong-shaped payloads, and a missing native library.
        val cases = listOf(
            "",
            "   ",
            "not json",
            "[]",
            """{"language":"en"}""",
            """{"messages":[]}""",
            """{"messages":{}}""",
        )
        for (payload in cases) {
            assertNull("payload=$payload", CoreStringsSource(FakeBridge(payload)).load(AppLanguage.EN))
        }
        assertNull(
            CoreStringsSource(
                FakeBridge("{}", throwOnBundle = UnsatisfiedLinkError("no lib")),
            ).load(AppLanguage.EN),
        )
    }

    @Test
    fun nonStringMessageValuesAreSkippedNotFatal() {
        val bridge = FakeBridge(
            """{"language":"en","messages":{"a":"A","b":42,"c":null,"d":"D"}}""",
        )
        val strings = CoreStringsSource(bridge).load(AppLanguage.EN)!!
        assertEquals(2, strings.size)
        assertEquals("A", strings["a"])
        assertEquals("D", strings["d"])
        assertEquals("b", strings["b"])
    }

    @Test
    fun parseBundleIsIndependentlyTestable() {
        assertEquals(
            mapOf("k" to "v"),
            CoreStringsSource.parseBundle("""{"language":"en","messages":{"k":"v"}}"""),
        )
        assertNull(CoreStringsSource.parseBundle(null))
        assertNull(CoreStringsSource.parseBundle("{"))
    }

    @Test
    fun compositeSourceFallsThroughAndNeverReturnsNull() {
        val failing = object : StringsSource {
            override fun load(language: AppLanguage): RcStrings? = null
        }
        val throwing = object : StringsSource {
            override fun load(language: AppLanguage): RcStrings = throw IllegalStateException()
        }
        val working = object : StringsSource {
            override fun load(language: AppLanguage) =
                RcStrings.of(language, mapOf("x" to "y"), RcStrings.Source.RESOURCES)
        }
        assertEquals(
            "y",
            CompositeStringsSource(listOf(failing, throwing, working)).load(AppLanguage.EN)["x"],
        )
        // Every source failing still yields a usable (empty) table.
        val all = CompositeStringsSource(listOf(failing, throwing)).load(AppLanguage.EN)
        assertEquals(RcStrings.Source.EMPTY, all.source)
        assertEquals("x", all["x"])
        assertEquals(
            RcStrings.Source.EMPTY,
            CompositeStringsSource(emptyList()).load(AppLanguage.EN).source,
        )
    }
}
