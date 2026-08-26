package com.rc.launcher.ui.viewmodel

import com.rc.launcher.ui.model.InMemorySettingsRepository
import com.rc.launcher.ui.model.LauncherSettings
import com.rc.launcher.ui.model.MirrorCatalog
import com.rc.launcher.ui.model.RendererOption
import com.rc.launcher.ui.model.MirrorLatency
import com.rc.launcher.ui.model.MirrorMeasurer
import com.rc.launcher.ui.model.MirrorProbeState
import com.rc.launcher.ui.model.MirrorSource
import com.rc.launcher.ui.model.RendererPluginConfig
import com.rc.launcher.ui.model.ResolutionMode
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertNotNull
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
    @Test
    fun rendererOptions_settersUpdateStateAndPersist() {
        val repo = InMemorySettingsRepository()
        val vm = SettingsViewModel(repo)
        vm.setRenderer(RendererOption.ZINK.id)
        vm.setZinkVulkanDriver("turnip")
        vm.setAngleBackend("gl")
        vm.setGl4esNoSrgb(true)
        vm.setVirglServer("host:1234")

        val s = vm.settings.value
        assertEquals(RendererOption.ZINK.id, s.rendererId)
        assertEquals("turnip", s.rendererOptions.zinkVulkanDriver)
        assertEquals("gl", s.rendererOptions.angleBackend)
        assertEquals(true, s.rendererOptions.gl4esNoSrgb)
        assertEquals("host:1234", s.rendererOptions.virglServer)

        // Persisted too.
        val restored = SettingsViewModel(repo).settings.value
        assertEquals("turnip", restored.rendererOptions.zinkVulkanDriver)
        assertEquals("host:1234", restored.rendererOptions.virglServer)
    }

    @Test
    fun exportImport_roundTrips() {
        val repo = InMemorySettingsRepository()
        val vm = SettingsViewModel(repo)
        vm.setMirror(MirrorCatalog.MCBBS.id)
        vm.setJavaHeapMb(3072)
        vm.setRenderer(RendererOption.ANGLE.id)
        vm.setAngleBackend("gl")

        val payload = vm.exportSettings()
        assertTrue(payload.contains("mirrorId=mcbbs"))
        assertTrue(payload.contains("rendererId=opengles3_angle"))
        assertTrue(payload.contains("renderer.angleBackend=gl"))

        // A fresh VM on an empty repo, then import.
        val vm2 = SettingsViewModel(InMemorySettingsRepository())
        assertTrue(vm2.importSettings(payload))
        assertEquals(MirrorCatalog.MCBBS.id, vm2.settings.value.mirrorId)
        assertEquals(3072, vm2.settings.value.javaHeapMb)
        assertEquals("gl", vm2.settings.value.rendererOptions.angleBackend)
    }

    @Test
    fun importSettings_rejectsBlankAndKeepsState() {
        val vm = SettingsViewModel(InMemorySettingsRepository())
        vm.setMirror(MirrorCatalog.ALIYUN.id)
        // Blank / whitespace-only payloads are no-ops.
        assertEquals(false, vm.importSettings("   "))
        assertEquals(false, vm.importSettings(""))
        // State is untouched.
        assertEquals(MirrorCatalog.ALIYUN.id, vm.settings.value.mirrorId)
    }

    @Test
    fun export_defaultsProducesAllKeys() {
        val vm = SettingsViewModel(InMemorySettingsRepository())
        val payload = vm.exportSettings()
        assertNotNull(payload)
        assertTrue(payload.contains("mirrorId="))
        assertTrue(payload.contains("renderer.zinkVulkanDriver="))
        assertTrue(payload.contains("renderer.angleBackend="))
        assertTrue(payload.contains("renderer.gl4esNoSrgb="))
        assertTrue(payload.contains("renderer.virglServer="))
    }

    private class FakeMirrorMeasurer(private val latencies: Map<String, Long?>) : MirrorMeasurer {
        override suspend fun probe(mirror: MirrorSource): MirrorLatency =
            MirrorLatency(mirror.id, latencies[mirror.id], null)
    }

    @Test
    fun measureAndSelectFastestMirror_selectsLowestLatency() = runBlocking {
        val vm = SettingsViewModel(InMemorySettingsRepository())
        vm.setMirror(MirrorCatalog.BMCLAPI.id)
        val fake = FakeMirrorMeasurer(
            mapOf(
                MirrorCatalog.BMCLAPI.id to 500L,
                MirrorCatalog.MCBBS.id to 120L,
                MirrorCatalog.ALIYUN.id to 900L,
            ),
        )
        val result = vm.measureAndSelectFastestMirror(fake)
        assertEquals(MirrorCatalog.MCBBS.id, result.bestId)
        assertEquals(MirrorCatalog.MCBBS.id, vm.settings.value.mirrorId)
        assertTrue(vm.mirrorProbe.value is MirrorProbeState.Done)
    }

    @Test
    fun measureAndSelectFastestMirror_noReachableKeepsCurrent() = runBlocking {
        val vm = SettingsViewModel(InMemorySettingsRepository())
        vm.setMirror(MirrorCatalog.ALIYUN.id)
        val fake = FakeMirrorMeasurer(
            mapOf(
                MirrorCatalog.BMCLAPI.id to null,
                MirrorCatalog.MCBBS.id to null,
                MirrorCatalog.ALIYUN.id to null,
            ),
        )
        val result = vm.measureAndSelectFastestMirror(fake)
        assertEquals(null, result.bestId)
        // Current selection is left untouched when nothing is reachable.
        assertEquals(MirrorCatalog.ALIYUN.id, vm.settings.value.mirrorId)
    }

    @Test
    fun measureAndSelectFastestMirror_updatesProbeState() = runBlocking {
        val vm = SettingsViewModel(InMemorySettingsRepository())
        val fake = FakeMirrorMeasurer(mapOf(MirrorCatalog.BMCLAPI.id to 100L))
        vm.measureAndSelectFastestMirror(fake)
        val state = vm.mirrorProbe.value
        assertTrue(state is MirrorProbeState.Done)
        assertEquals(MirrorCatalog.BMCLAPI.id, (state as MirrorProbeState.Done).bestId)
    }

}
