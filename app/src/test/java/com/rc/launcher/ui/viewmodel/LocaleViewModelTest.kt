package com.rc.launcher.ui.viewmodel

import com.rc.launcher.ui.i18n.AppLanguage
import com.rc.launcher.ui.i18n.InMemoryLocaleStorage
import com.rc.launcher.ui.i18n.LocaleEngine
import com.rc.launcher.ui.i18n.RcStrings
import com.rc.launcher.ui.i18n.RcStringKeys
import com.rc.launcher.ui.i18n.StringsSource
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Unit tests for the language-picker state container (task 20). */
class LocaleViewModelTest {

    private class Source : StringsSource {
        override fun load(language: AppLanguage) = RcStrings.of(
            language,
            mapOf(
                RcStringKeys.LANGUAGE_SYSTEM to when (language) {
                    AppLanguage.EN -> "Follow system"
                    else -> "跟随系统"
                },
                RcStringKeys.NAV_HOME to when (language) {
                    AppLanguage.EN -> "Home"
                    AppLanguage.ZH_HANT -> "主頁"
                    else -> "主页"
                },
            ),
        )
    }

    private fun engine(saved: String? = null, systemTags: List<String> = listOf("zh-CN")) {
        LocaleEngine.install(InMemoryLocaleStorage(saved), Source(), systemTags)
    }

    @Test
    fun exposesTheEngineState() {
        engine(saved = "en")
        val vm = LocaleViewModel()
        assertEquals(AppLanguage.EN, vm.selected.value)
        assertEquals(AppLanguage.EN, vm.effective.value)
        assertEquals("Home", vm.strings.value[RcStringKeys.NAV_HOME])
    }

    @Test
    fun offersSystemFirstThenEveryCatalogue() {
        engine()
        val vm = LocaleViewModel()
        assertEquals(AppLanguage.selectable, vm.options)
        assertEquals(AppLanguage.SYSTEM, vm.options.first())
        assertTrue(vm.options.containsAll(AppLanguage.catalogues))
    }

    @Test
    fun switchingLanguageUpdatesTheTableImmediately() {
        engine(saved = null, systemTags = listOf("zh-CN"))
        val vm = LocaleViewModel()
        assertEquals("主页", vm.strings.value[RcStringKeys.NAV_HOME])

        vm.setLanguage(AppLanguage.ZH_HANT)
        assertEquals(AppLanguage.ZH_HANT, vm.selected.value)
        assertEquals("主頁", vm.strings.value[RcStringKeys.NAV_HOME])

        vm.setLanguageTag("en")
        assertEquals("Home", vm.strings.value[RcStringKeys.NAV_HOME])

        // Back to following the device.
        vm.setLanguage(AppLanguage.SYSTEM)
        assertEquals(AppLanguage.ZH_CN, vm.effective.value)
        assertEquals("主页", vm.strings.value[RcStringKeys.NAV_HOME])
    }

    @Test
    fun systemRowLabelDisclosesWhatItResolvedTo() {
        engine(saved = null, systemTags = listOf("zh-HK"))
        val vm = LocaleViewModel()
        val strings = vm.strings.value
        // "Follow system" alone is ambiguous, so the resolved endonym is appended.
        val label = vm.labelFor(AppLanguage.SYSTEM, strings)
        assertTrue(label, label.startsWith("跟随系统"))
        assertTrue(label, label.endsWith(AppLanguage.ZH_HANT.nativeName))
        // Concrete rows use the endonym, which is never translated.
        assertEquals("English", vm.labelFor(AppLanguage.EN, strings))
        assertEquals("简体中文", vm.labelFor(AppLanguage.ZH_CN, strings))
    }

    @Test
    fun reloadKeepsTheSelection() {
        engine(saved = "zh-Hant")
        val vm = LocaleViewModel()
        vm.reload()
        assertEquals(AppLanguage.ZH_HANT, vm.selected.value)
        assertEquals("主頁", vm.strings.value[RcStringKeys.NAV_HOME])
    }
}
