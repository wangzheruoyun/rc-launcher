package com.rc.launcher.ui.viewmodel

import com.rc.launcher.ui.model.InMemorySettingsRepository
import com.rc.launcher.ui.model.LauncherSettings
import com.rc.launcher.ui.model.MirrorCatalog
import com.rc.launcher.ui.model.RendererOption
import com.rc.launcher.ui.model.ResolutionMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Unit tests for the Settings Center state container (task 14). */
class SettingsViewModelTest {

    @Test
    fun initial_loadsDefaultsFromRepository() {
        val repo = InMemorySettingsRepository()
        val vm = SettingsViewModel(repo)
        assertEquals(LauncherSettings(), vm.settings.value)
        assertEquals(MirrorCatalog.all, vm.mirrors)
        assertTrue(vm.renderers.contains(RendererOption.DEFAULT))
    }

    @Test
    fun setMirror_updatesStateAndPersists() {
        val repo = InMemorySettingsRepository()
        val vm = SettingsViewModel(repo)
        vm.setMirror(MirrorCatalog.ALIYUN.id)

        assertEquals(MirrorCatalog.ALIYUN.id, vm.settings.value.mirrorId)
        // A fresh ViewModel sharing the repository must observe the persisted value.
        val restored = SettingsViewModel(repo)
        assertEquals(MirrorCatalog.ALIYUN.id, restored.settings.value.mirrorId)
    }

    @Test
    fun setJavaHeapMb_isClampedBySanitize() {
        val vm = SettingsViewModel(InMemorySettingsRepository())
        vm.setJavaHeapMb(1_000_000)
        assertEquals(LauncherSettings.MAX_HEAP_MB, vm.settings.value.javaHeapMb)
        assertEquals(null, vm.settings.value.validationError())
    }

    @Test
    fun setRenderer_invalidIdFallsBackToDefault() {
        val vm = SettingsViewModel(InMemorySettingsRepository())
        vm.setRenderer("not-a-real-renderer")
        assertEquals(RendererOption.DEFAULT.id, vm.settings.value.rendererId)
    }

    @Test
    fun setJavaMinHeapMb_aboveMaxClampedAndClearable() {
        val vm = SettingsViewModel(InMemorySettingsRepository())
        vm.setJavaHeapMb(1024)
        vm.setJavaMinHeapMb(4096)
        // sanitize clamps min to max (1024) and the value is kept as 1024.
        assertEquals(1024, vm.settings.value.javaMinHeapMb)
        assertEquals(null, vm.settings.value.validationError())
        // Setting 0 clears the field.
        vm.setJavaMinHeapMb(0)
        assertNull(vm.settings.value.javaMinHeapMb)
    }

    @Test
    fun togglingSwitches_updatesState() {
        val vm = SettingsViewModel(InMemorySettingsRepository())
        assertTrue(vm.settings.value.autoAllocateMemory)
        vm.setAutoAllocateMemory(false)
        assertEquals(false, vm.settings.value.autoAllocateMemory)

        vm.setUseDoh(false)
        assertEquals(false, vm.settings.value.useDoh)

        vm.setFullscreen(true)
        assertEquals(true, vm.settings.value.fullscreen)
    }

    @Test
    fun setResolutionMode_customPersistsCustomSize() {
        val vm = SettingsViewModel(InMemorySettingsRepository())
        vm.setResolutionMode(ResolutionMode.CUSTOM)
        vm.setCustomResolution(1920, 1080)
        assertEquals(ResolutionMode.CUSTOM, vm.settings.value.resolutionMode)
        assertEquals(1920, vm.settings.value.customWidth)
        assertEquals(1080, vm.settings.value.customHeight)
    }

    @Test
    fun resetToDefaults_restoresFactorySettings() {
        val repo = InMemorySettingsRepository()
        val vm = SettingsViewModel(repo)
        vm.setMirror(MirrorCatalog.MCBBS.id)
        vm.setJavaHeapMb(4096)
        vm.resetToDefaults()
        assertEquals(LauncherSettings(), vm.settings.value)
        // Persisted too.
        assertEquals(LauncherSettings(), SettingsViewModel(repo).settings.value)
    }

    @Test
    fun controllerSettings_roundTrip() {
        val vm = SettingsViewModel(InMemorySettingsRepository())
        vm.setControllerEnabled(true)
        vm.setControllerLayout("gamepad")
        vm.setControllerDeadzone(0.5f)
        assertEquals(true, vm.settings.value.controllerEnabled)
        assertEquals("gamepad", vm.settings.value.controllerLayoutId)
        assertEquals(0.5f, vm.settings.value.controllerDeadzone)
    }
}
