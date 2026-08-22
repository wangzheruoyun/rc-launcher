package com.rc.launcher.ui.viewmodel

import com.rc.launcher.ui.model.GameInstance
import com.rc.launcher.ui.model.InstallRequest
import com.rc.launcher.ui.model.InstallStep
import com.rc.launcher.ui.model.InstanceRepository
import com.rc.launcher.ui.model.LoaderVersion
import com.rc.launcher.ui.model.ModLoader
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/** Unit tests for the install-wizard state machine (task 13). */
class InstallViewModelTest {

    @Before
    fun setUp() {
        // The repository is a process-wide singleton; isolate each test.
        InstanceRepository.replaceAll(emptyList())
    }

    @Test
    fun initialState_startsAtLoaderStep() {
        val vm = InstallViewModel()
        assertEquals(InstallStep.LOADER, vm.step.value)
        assertEquals(InstallRequest(), vm.request.value)
        assertFalse(vm.canGoBack())
    }

    @Test
    fun vanillaFlow_skipsLoaderStepAndCreatesInstance() {
        val vm = InstallViewModel()
        assertTrue(vm.next()) // LOADER -> GAME_VERSION
        assertEquals(InstallStep.GAME_VERSION, vm.step.value)

        vm.setGameVersion("1.20.1")
        assertTrue(vm.next()) // GAME_VERSION -> CONFIGURE (vanilla skips loader)
        assertEquals(InstallStep.CONFIGURE, vm.step.value)

        vm.setName("我的世界")
        assertTrue(vm.next()) // CONFIGURE -> REVIEW
        assertEquals(InstallStep.REVIEW, vm.step.value)
        assertTrue(vm.canProceed())

        val created = vm.create()
        assertEquals("我的世界", created.name)
        assertEquals(ModLoader.VANILLA, created.modLoader)
        assertEquals("1.20.1", created.version)
        assertTrue(InstanceRepository.instances.value.any { it.id == created.id })
    }

    @Test
    fun fabricFlow_visitsLoaderVersionStep() {
        val vm = InstallViewModel()
        vm.next() // -> GAME_VERSION
        vm.setGameVersion("1.20.1")
        vm.next() // -> LOADER_VERSION (modded)
        assertEquals(InstallStep.LOADER_VERSION, vm.step.value)

        val versions = vm.availableLoaderVersions()
        assertTrue(versions.isNotEmpty())
        vm.setLoaderVersion(versions.first())
        vm.next() // -> CONFIGURE
        assertEquals(InstallStep.CONFIGURE, vm.step.value)
        vm.setName("Fabric 包")
        vm.next() // -> REVIEW
        assertTrue(vm.canProceed())

        val created = vm.create()
        assertEquals(ModLoader.FABRIC, created.modLoader)
        assertNotNull(created.loaderVersion)
    }

    @Test
    fun create_ensuresUniqueIdOnCollision() {
        // Pre-seed an instance with the id the fabric flow would produce.
        val collision = GameInstance(
            id = "fabric-1.20.1-0.16.0",
            name = "existing",
            version = "1.20.1",
            modLoader = ModLoader.FABRIC,
        )
        InstanceRepository.replaceAll(listOf(collision))

        val vm = InstallViewModel()
        vm.next()
        vm.setGameVersion("1.20.1")
        vm.next()
        val lv = LoaderVersion("fabric-0.16.0-1.20.1", "0.16.0", "1.20.1")
        vm.setLoaderVersion(lv)
        vm.next()
        vm.setName("新 Fabric")
        vm.next()
        val created = vm.create()

        assertEquals("fabric-1.20.1-0.16.0-1", created.id)
        assertTrue(InstanceRepository.instances.value.any { it.id == "fabric-1.20.1-0.16.0" })
        assertTrue(InstanceRepository.instances.value.any { it.id == created.id })
    }

    @Test
    fun setLoader_dropsStaleLoaderVersion() {
        val vm = InstallViewModel()
        vm.setLoader(ModLoader.FABRIC)
        vm.setGameVersion("1.20.1")
        vm.setLoaderVersion(vm.availableLoaderVersions().first())
        assertTrue(vm.request.value.loaderVersion != null)

        vm.setLoader(ModLoader.VANILLA)
        assertEquals(ModLoader.VANILLA, vm.request.value.loader)
        assertEquals(null, vm.request.value.loaderVersion)
    }

    @Test
    fun reset_returnsToStart() {
        val vm = InstallViewModel()
        vm.next()
        vm.setGameVersion("1.20.1")
        vm.reset()
        assertEquals(InstallStep.LOADER, vm.step.value)
        assertEquals(InstallRequest(), vm.request.value)
    }
}
