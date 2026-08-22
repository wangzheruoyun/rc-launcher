package com.rc.launcher.ui.i18n

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The observable locale engine (task 20).
 *
 * [LocaleEngine] is a process-wide singleton (like `ThemeEngine`), so every test
 * re-installs it with in-memory collaborators through [LocaleEngine.install] —
 * no Android, no native core.
 */
class LocaleEngineTest {

    /** A catalogue source that serves a one-key table per language. */
    private class FakeSource(
        private val fail: Boolean = false,
        private val source: RcStrings.Source = RcStrings.Source.CORE,
    ) : StringsSource {
        var loads = 0
            private set
        val requested = mutableListOf<AppLanguage>()

        override fun load(language: AppLanguage): RcStrings? {
            loads++
            requested += language
            if (fail) return null
            return RcStrings.of(
                language,
                mapOf("nav.home" to "home:${language.tag}"),
                source,
            )
        }
    }

    private fun install(
        saved: String? = null,
        systemTags: List<String> = listOf("zh-CN"),
        source: StringsSource = FakeSource(),
    ): Pair<InMemoryLocaleStorage, MutableList<AppLanguage>> {
        val storage = InMemoryLocaleStorage(saved)
        val applied = mutableListOf<AppLanguage>()
        LocaleEngine.install(storage, source, systemTags) { applied += it }
        return storage to applied
    }

    @Test
    fun freshInstallFollowsTheSystemAndDoesNotPersist() {
        val (storage, applied) = install(saved = null, systemTags = listOf("en-US"))
        assertEquals(AppLanguage.SYSTEM, LocaleEngine.selected.value)
        assertEquals(AppLanguage.EN, LocaleEngine.effective.value)
        // Restoring must not write a preference the user never made.
        assertNull(storage.getLanguageTag())
        assertTrue(applied.isEmpty())
    }

    @Test
    fun restoresAnExplicitSavedSelection() {
        install(saved = "zh-Hant", systemTags = listOf("en-US"))
        assertEquals(AppLanguage.ZH_HANT, LocaleEngine.selected.value)
        assertEquals(AppLanguage.ZH_HANT, LocaleEngine.effective.value)
        assertEquals("home:zh-Hant", LocaleEngine.strings.value["nav.home"])
    }

    @Test
    fun corruptSavedSelectionDegradesToFollowSystem() {
        install(saved = "not-a-language", systemTags = listOf("zh-TW"))
        assertEquals(AppLanguage.SYSTEM, LocaleEngine.selected.value)
        assertEquals(AppLanguage.ZH_HANT, LocaleEngine.effective.value)
    }

    @Test
    fun unsupportedSystemLocaleFallsBackToChinese() {
        install(saved = null, systemTags = listOf("ja-JP", "ko-KR"))
        assertEquals(AppLanguage.BASE, LocaleEngine.effective.value)
    }

    @Test
    fun setLanguagePersistsReloadsAndNotifiesThePlatform() {
        val source = FakeSource()
        val (storage, applied) = install(saved = null, source = source)
        val before = source.loads

        LocaleEngine.setLanguage(AppLanguage.EN)

        assertEquals(AppLanguage.EN, LocaleEngine.selected.value)
        assertEquals(AppLanguage.EN, LocaleEngine.effective.value)
        assertEquals("home:en", LocaleEngine.strings.value["nav.home"])
        assertEquals("en", storage.getLanguageTag())
        assertEquals(listOf(AppLanguage.EN), applied)
        assertEquals("the table must be reloaded once", before + 1, source.loads)
    }

    @Test
    fun switchingByTagAcceptsSystemAndUnknownValues() {
        val (storage, _applied) = install(saved = "en", systemTags = listOf("zh-TW"))
        LocaleEngine.setLanguageTag("system")
        assertEquals(AppLanguage.SYSTEM, LocaleEngine.selected.value)
        assertEquals(AppLanguage.ZH_HANT, LocaleEngine.effective.value)
        assertEquals("system", storage.getLanguageTag())
        // An unknown tag must not throw; it means "follow the system".
        LocaleEngine.setLanguageTag("qqq")
        assertEquals(AppLanguage.SYSTEM, LocaleEngine.selected.value)
    }

    @Test
    fun systemLocaleChangeReResolvesOnlyWhenFollowingTheSystem() {
        install(saved = null, systemTags = listOf("zh-CN"))
        assertEquals(AppLanguage.ZH_CN, LocaleEngine.effective.value)
        LocaleEngine.onSystemLocalesChanged(listOf("en-GB"))
        assertEquals(AppLanguage.EN, LocaleEngine.effective.value)

        // With an explicit choice the device language must be ignored.
        LocaleEngine.setLanguage(AppLanguage.ZH_HANT)
        LocaleEngine.onSystemLocalesChanged(listOf("en-US"))
        assertEquals(AppLanguage.ZH_HANT, LocaleEngine.effective.value)
    }

    @Test
    fun aFailingSourceLeavesAUsableEmptyTable() {
        install(saved = "en", source = FakeSource(fail = true))
        val strings = LocaleEngine.strings.value
        assertEquals(AppLanguage.EN, strings.language)
        assertEquals(RcStrings.Source.EMPTY, strings.source)
        // Lookups echo the key, so the screen still renders.
        assertEquals("nav.home", strings["nav.home"])
    }

    @Test
    fun aThrowingSourceIsContainedToo() {
        val throwing = object : StringsSource {
            override fun load(language: AppLanguage): RcStrings =
                throw UnsatisfiedLinkError("librc_launcher.so missing")
        }
        install(saved = "en", source = throwing)
        assertEquals(RcStrings.Source.EMPTY, LocaleEngine.strings.value.source)
        assertEquals(AppLanguage.EN, LocaleEngine.effective.value)
    }

    @Test
    fun resolveNeverReturnsTheSystemPseudoLanguage() {
        install(saved = null, systemTags = listOf("ja"))
        for (l in AppLanguage.selectable) {
            val resolved = LocaleEngine.resolve(l)
            assertTrue("resolve($l) = $resolved", !resolved.isSystem)
        }
        assertEquals(AppLanguage.BASE, LocaleEngine.resolve(AppLanguage.SYSTEM))
    }

    @Test
    fun reloadKeepsTheSelectionButRefetchesTheTable() {
        val source = FakeSource()
        val (storage, applied) = install(saved = "en", source = source)
        val loads = source.loads
        LocaleEngine.reload()
        assertEquals(loads + 1, source.loads)
        assertEquals(AppLanguage.EN, LocaleEngine.selected.value)
        // reload() is not a user choice: nothing is persisted or re-applied.
        assertEquals("en", storage.getLanguageTag())
        assertTrue(applied.isEmpty())
    }

    @Test
    fun theSourceIsAlwaysAskedForAConcreteCatalogue() {
        val source = FakeSource()
        install(saved = null, systemTags = listOf("zh-HK"), source = source)
        assertTrue(source.requested.isNotEmpty())
        assertTrue(source.requested.none { it.isSystem })
        assertEquals(AppLanguage.ZH_HANT, source.requested.last())
    }
}
