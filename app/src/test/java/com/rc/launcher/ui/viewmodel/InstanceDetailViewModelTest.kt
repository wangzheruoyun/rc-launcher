package com.rc.launcher.ui.viewmodel

import com.rc.launcher.ui.model.GameDirectoryType
import com.rc.launcher.ui.model.GameInstance
import com.rc.launcher.ui.model.InstanceRepository
import com.rc.launcher.ui.model.ModLoader
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/** Unit tests for the instance-detail / settings editor (task 13). */
class InstanceDetailViewModelTest {

    private val sample = GameInstance(
        id = "fabric-1.20.1",
        name = "Fabric 整合包",
        version = "1.20.1",
        modLoader = ModLoader.FABRIC,
        loaderVersion = "0.16.0",
    )

    @Before
    fun setUp() {
        InstanceRepository.replaceAll(listOf(sample))
    }

    @Test
    fun load_existingInstanceIsAvailable() {
        val vm = InstanceDetailViewModel()
        vm.load("fabric-1.20.1")
        assertEquals("Fabric 整合包", vm.instance.value?.name)
    }

    @Test
    fun load_unknownIdIsNull() {
        val vm = InstanceDetailViewModel()
        vm.load("does-not-exist")
        assertNull(vm.instance.value)
    }

    @Test
    fun setName_persistsToRepository() {
        val vm = InstanceDetailViewModel()
        vm.load("fabric-1.20.1")
        vm.setName("重命名后的包")
        assertEquals("重命名后的包", vm.instance.value?.name)
        assertEquals("重命名后的包", InstanceRepository.getById("fabric-1.20.1")?.name)
    }

    @Test
    fun setGameDirectoryType_customClearsDirWhenBlank() {
        val vm = InstanceDetailViewModel()
        vm.load("fabric-1.20.1")
        vm.setGameDirectoryType(GameDirectoryType.CUSTOM)
        // customGameDir was blank -> stays null
        assertEquals(GameDirectoryType.CUSTOM, vm.instance.value?.gameDirectoryType)
        assertNull(vm.instance.value?.customGameDir)
    }

    @Test
    fun customDirIsClearedWhenSwitchingAwayFromCustom() {
        val vm = InstanceDetailViewModel()
        vm.load("fabric-1.20.1")
        vm.setGameDirectoryType(GameDirectoryType.CUSTOM)
        vm.setCustomGameDir("/sdcard/games/x")
        assertEquals("/sdcard/games/x", vm.instance.value?.customGameDir)
        vm.setGameDirectoryType(GameDirectoryType.ISOLATED)
        assertNull(vm.instance.value?.customGameDir)
    }

    @Test
    fun duplicate_createsCloneAndKeepsOriginal() {
        val vm = InstanceDetailViewModel()
        vm.load("fabric-1.20.1")
        val newId = vm.duplicate()
        assertNotNull(newId)
        // original stays loaded & untouched
        assertEquals("fabric-1.20.1", vm.instance.value?.id)
        // clone exists in repository with copied settings
        val clone = InstanceRepository.getById(newId!!)
        assertNotNull(clone)
        assertEquals("Fabric 整合包 副本", clone?.name)
        assertEquals(ModLoader.FABRIC, clone?.modLoader)
        assertEquals(0L, clone?.lastPlayed)
        // both original and clone are present
        assertEquals(2, InstanceRepository.instances.value.size)
    }

    @Test
    fun duplicate_returnsNullWhenNothingLoaded() {
        val vm = InstanceDetailViewModel()
        assertNull(vm.duplicate())
    }

    @Test
    fun delete_removesInstanceAndReturnsId() {
        val vm = InstanceDetailViewModel()
        vm.load("fabric-1.20.1")
        val deletedId = vm.delete()
        assertEquals("fabric-1.20.1", deletedId)
        assertNull(InstanceRepository.getById("fabric-1.20.1"))
        assertTrue(InstanceRepository.instances.value.isEmpty())
    }
}
