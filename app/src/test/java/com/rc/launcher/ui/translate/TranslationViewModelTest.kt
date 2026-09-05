package com.rc.launcher.ui.translate

import com.rc.launcher.ui.i18n.AppLanguage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Unit tests for [TranslationViewModel] (task 13).
 *
 * The viewmodel itself is pure-Kotlin: it does no JNI work until
 * [TranslationViewModel.ensureInitialised] is called. These tests
 * therefore exercise only the state-machine surface, which is what
 * the Compose layer depends on.
 */
class TranslationViewModelTest {

    @Test
    fun default_preferences_are_enabled_hybrid_show_original_auto() {
        val vm = TranslationViewModel()
        val prefs = vm.state.value.prefs
        assertTrue("translation is on by default", prefs.enabled)
        assertEquals(TranslationViewModel.TranslationMode.Hybrid, prefs.mode)
        assertTrue("show original on by default", prefs.showOriginal)
        assertEquals(TranslationViewModel.TranslationTarget.Auto, prefs.target)
    }

    @Test
    fun setEnabled_toggles_the_preference() {
        val vm = TranslationViewModel()
        vm.setEnabled(false)
        assertFalse(vm.state.value.prefs.enabled)
        vm.setEnabled(true)
        assertTrue(vm.state.value.prefs.enabled)
    }

    @Test
    fun setTarget_picks_the_target() {
        val vm = TranslationViewModel()
        vm.setTarget(TranslationViewModel.TranslationTarget.ZhCn)
        assertEquals(TranslationViewModel.TranslationTarget.ZhCn, vm.state.value.prefs.target)
        vm.setTarget(TranslationViewModel.TranslationTarget.ZhHant)
        assertEquals(TranslationViewModel.TranslationTarget.ZhHant, vm.state.value.prefs.target)
    }

    @Test
    fun setMode_picks_the_mode() {
        val vm = TranslationViewModel()
        vm.setMode(TranslationViewModel.TranslationMode.Offline)
        assertEquals(TranslationViewModel.TranslationMode.Offline, vm.state.value.prefs.mode)
    }

    @Test
    fun setShowOriginal_toggles_the_show_original_flag() {
        val vm = TranslationViewModel()
        vm.setShowOriginal(false)
        assertFalse(vm.state.value.prefs.showOriginal)
    }

    @Test
    fun translation_target_from_tag_is_tolerant_of_unknown() {
        assertEquals(
            TranslationViewModel.TranslationTarget.ZhCn,
            TranslationViewModel.TranslationTarget.fromTag("zh-CN"),
        )
        assertEquals(
            TranslationViewModel.TranslationTarget.ZhHant,
            TranslationViewModel.TranslationTarget.fromTag("zh-Hant"),
        )
        assertEquals(
            TranslationViewModel.TranslationTarget.En,
            TranslationViewModel.TranslationTarget.fromTag("en"),
        )
        // Unknown falls back to Auto.
        assertEquals(
            TranslationViewModel.TranslationTarget.Auto,
            TranslationViewModel.TranslationTarget.fromTag(null),
        )
        assertEquals(
            TranslationViewModel.TranslationTarget.Auto,
            TranslationViewModel.TranslationTarget.fromTag("klingon"),
        )
    }

    @Test
    fun translation_mode_from_id_is_tolerant_of_unknown() {
        assertEquals(
            TranslationViewModel.TranslationMode.Online,
            TranslationViewModel.TranslationMode.fromId("online"),
        )
        assertEquals(
            TranslationViewModel.TranslationMode.Hybrid,
            TranslationViewModel.TranslationMode.fromId("unknown"),
        )
    }

    @Test
    fun translation_source_from_id_handles_every_variant() {
        for (s in TranslationViewModel.TranslationSource.entries) {
            assertEquals(s, TranslationViewModel.TranslationSource.fromId(s.id))
        }
        assertEquals(
            TranslationViewModel.TranslationSource.Unknown,
            TranslationViewModel.TranslationSource.fromId("nope"),
        )
    }

    @Test
    fun default_target_resolves_ui_app_language() {
        assertEquals(
            TranslationViewModel.TranslationTarget.ZhCn,
            TranslationViewModel.defaultTarget(AppLanguage.ZH_CN),
        )
        assertEquals(
            TranslationViewModel.TranslationTarget.ZhHant,
            TranslationViewModel.defaultTarget(AppLanguage.ZH_HANT),
        )
        assertEquals(
            TranslationViewModel.TranslationTarget.En,
            TranslationViewModel.defaultTarget(AppLanguage.EN),
        )
        // SYSTEM resolves to Auto so the picker follows whatever the
        // gateway detects.
        assertEquals(
            TranslationViewModel.TranslationTarget.Auto,
            TranslationViewModel.defaultTarget(AppLanguage.SYSTEM),
        )
    }

    @Test
    fun translate_entry_carries_all_fields() {
        val entry = TranslationViewModel.TranslatedEntry(
            original = "Hello",
            translated = "你好",
            source = TranslationViewModel.TranslationSource.Dictionary,
            offline = true,
        )
        assertEquals("Hello", entry.original)
        assertEquals("你好", entry.translated)
        assertEquals(TranslationViewModel.TranslationSource.Dictionary, entry.source)
        assertTrue(entry.offline)
    }

    @Test
    fun cache_stats_data_class_carries_fields() {
        val stats = TranslationViewModel.CacheStats(
            root = "/sdcard/cache",
            entryCount = 10,
            totalBytes = 2048,
            maxEntries = 4096,
            maxBytes = 32 * 1024 * 1024,
        )
        assertNotNull(stats)
        assertEquals("/sdcard/cache", stats.root)
        assertEquals(10L, stats.entryCount)
        assertEquals(2048L, stats.totalBytes)
    }
}
