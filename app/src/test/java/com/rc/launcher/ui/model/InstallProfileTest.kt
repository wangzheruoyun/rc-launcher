package com.rc.launcher.ui.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Unit tests for the version-installation data model (task 13). */
class InstallProfileTest {

    @Test
    fun catalog_vanillaHasNoLoaderVersions() {
        assertTrue(LoaderCatalog.loaderVersions(ModLoader.VANILLA, "1.20.1").isEmpty())
        assertTrue(LoaderCatalog.gameVersions.isNotEmpty())
        assertTrue(LoaderCatalog.gameVersions.contains("1.20.1"))
    }

    @Test
    fun catalog_moddedLoaderVersionsAreWellFormed() {
        for (loader in listOf(ModLoader.FABRIC, ModLoader.QUILT, ModLoader.FORGE, ModLoader.OPTIFINE)) {
            val versions = LoaderCatalog.loaderVersions(loader, "1.20.1")
            assertTrue("$loader should expose versions", versions.isNotEmpty())
            for (lv in versions) {
                assertEquals(loader.name, lv.gameVersion, "1.20.1")
                assertTrue("id must be non-blank for $lv", lv.id.isNotBlank())
                assertTrue("version must be non-blank for $lv", lv.version.isNotBlank())
            }
            // ids must be unique within a loader/game-version pair
            assertEquals(versions.size, versions.map { it.id }.toSet().size)
        }
    }

    @Test
    fun request_requiresLoaderVersionOnlyForModded() {
        assertFalse(InstallRequest(loader = ModLoader.VANILLA).requiresLoaderVersion)
        assertTrue(InstallRequest(loader = ModLoader.FABRIC).requiresLoaderVersion)
        assertTrue(InstallRequest(loader = ModLoader.FORGE).requiresLoaderVersion)
        assertTrue(InstallRequest(loader = ModLoader.QUILT).requiresLoaderVersion)
        assertTrue(InstallRequest(loader = ModLoader.OPTIFINE).requiresLoaderVersion)
    }

    @Test
    fun request_validationErrors() {
        // blank game version
        assertEquals("请选择游戏版本", InstallRequest(loader = ModLoader.VANILLA).validationError())
        // vanilla with version but no name is still invalid (name required)
        assertEquals(
            "请填写实例名称",
            InstallRequest(loader = ModLoader.VANILLA, gameVersion = "1.20.1").validationError(),
        )
        // modded requires loader version
        assertEquals(
            "请选择 Fabric 版本",
            InstallRequest(
                loader = ModLoader.FABRIC,
                gameVersion = "1.20.1",
                name = "x",
            ).validationError(),
        )
        // custom directory requires a path
        assertEquals(
            "自定义目录不能为空",
            InstallRequest(
                loader = ModLoader.VANILLA,
                gameVersion = "1.20.1",
                name = "x",
                gameDirectoryType = GameDirectoryType.CUSTOM,
            ).validationError(),
        )
        // valid vanilla request
        assertNull(
            InstallRequest(
                loader = ModLoader.VANILLA,
                gameVersion = "1.20.1",
                name = "x",
            ).validationError(),
        )
    }

    @Test
    fun request_buildInstanceMapsFieldsAndFillsDefaults() {
        val lv = LoaderVersion("fabric-0.16.0-1.20.1", "0.16.0", "1.20.1", stable = true)
        val req = InstallRequest(
            loader = ModLoader.FABRIC,
            gameVersion = "1.20.1",
            loaderVersion = lv,
            name = "",
            javaVersion = 17,
            gameDirectoryType = GameDirectoryType.ISOLATED,
        )
        val inst = req.buildInstance("my-id")
        assertEquals("my-id", inst.id)
        assertEquals("1.20.1", inst.version)
        assertEquals(ModLoader.FABRIC, inst.modLoader)
        assertEquals("0.16.0", inst.loaderVersion)
        assertEquals(17, inst.javaVersion)
        assertEquals(GameDirectoryType.ISOLATED, inst.gameDirectoryType)
        // blank name falls back to a generated default name
        assertTrue(inst.name.startsWith("Fabric 1.20.1"))
    }

    @Test
    fun request_defaultIdIsFilesystemSafe() {
        val lv = LoaderVersion("fabric-0.16.0-1.20.1", "0.16.0", "1.20.1")
        val id = InstallRequest(
            loader = ModLoader.FABRIC,
            gameVersion = "1.20.1",
            loaderVersion = lv,
            name = "x",
        ).defaultId()
        assertTrue(id.matches(Regex("^[a-z0-9.-]+$")))
        assertEquals("fabric-1.20.1-0.16.0", id)
    }

    @Test
    fun gameInstance_effectiveGameDirHonoursIsolation() {
        val base = "games/RC"
        val vanilla = GameInstance("v", "V", "1.20.1", gameDirectoryType = GameDirectoryType.DEFAULT)
        assertEquals(base, vanilla.effectiveGameDir(base))

        val isolated = GameInstance("f", "F", "1.20.1", gameDirectoryType = GameDirectoryType.ISOLATED)
        assertEquals("$base/instances/f", isolated.effectiveGameDir(base))

        val custom = GameInstance(
            "q",
            "Q",
            "1.20.1",
            gameDirectoryType = GameDirectoryType.CUSTOM,
            customGameDir = "/sdcard/my",
        )
        assertEquals("/sdcard/my", custom.effectiveGameDir(base))

        // CUSTOM with a blank path falls back to the isolated path
        val customBlank = GameInstance(
            "q2",
            "Q2",
            "1.20.1",
            gameDirectoryType = GameDirectoryType.CUSTOM,
            customGameDir = "   ",
        )
        assertEquals("$base/instances/q2", customBlank.effectiveGameDir(base))
    }

    @Test
    fun stepNavigation_skipsLoaderVersionForVanilla() {
        val vanilla = InstallRequest(loader = ModLoader.VANILLA, gameVersion = "1.20.1")
        assertEquals(InstallStep.GAME_VERSION, InstallStep.LOADER.next(vanilla))
        // from GAME_VERSION, vanilla jumps straight to CONFIGURE (no loader step)
        assertEquals(InstallStep.CONFIGURE, InstallStep.GAME_VERSION.next(vanilla))

        val fabric = InstallRequest(loader = ModLoader.FABRIC, gameVersion = "1.20.1")
        assertEquals(InstallStep.LOADER_VERSION, InstallStep.GAME_VERSION.next(fabric))
        assertEquals(InstallStep.CONFIGURE, InstallStep.LOADER_VERSION.next(fabric))
        assertEquals(InstallStep.REVIEW, InstallStep.CONFIGURE.next(fabric))
        assertNull(InstallStep.REVIEW.next(fabric))
    }

    @Test
    fun stepNavigation_previousMirrorsNext() {
        val fabric = InstallRequest(loader = ModLoader.FABRIC, gameVersion = "1.20.1")
        assertEquals(InstallStep.CONFIGURE, InstallStep.REVIEW.previous(fabric))
        assertEquals(InstallStep.LOADER_VERSION, InstallStep.CONFIGURE.previous(fabric))
        assertEquals(InstallStep.GAME_VERSION, InstallStep.LOADER_VERSION.previous(fabric))
        assertEquals(InstallStep.LOADER, InstallStep.GAME_VERSION.previous(fabric))
        assertNull(InstallStep.LOADER.previous(fabric))
    }

    @Test
    fun step_canProceedGatesAdvance() {
        val vanilla = InstallRequest(loader = ModLoader.VANILLA)
        assertTrue(InstallStep.LOADER.canProceed(vanilla))
        assertFalse(InstallStep.GAME_VERSION.canProceed(vanilla)) // blank version
        assertFalse(InstallStep.CONFIGURE.canProceed(vanilla))    // blank name
        val complete = vanilla.copy(gameVersion = "1.20.1", name = "x")
        assertTrue(InstallStep.GAME_VERSION.canProceed(complete))
        assertTrue(InstallStep.CONFIGURE.canProceed(complete))
        assertTrue(InstallStep.REVIEW.canProceed(complete))
    }

    @Test
    fun request_validationRejectsUnsupportedJavaVersion() {
        // 99 is not in SUPPORTED_JAVA_VERSIONS -> rejected.
        assertEquals(
            "Java 版本不受支持",
            InstallRequest(
                loader = ModLoader.VANILLA,
                gameVersion = "1.20.1",
                name = "x",
                javaVersion = 99,
            ).validationError(),
        )
        // 7 (too old) is also unsupported now.
        assertEquals(
            "Java 版本不受支持",
            InstallRequest(
                loader = ModLoader.VANILLA,
                gameVersion = "1.20.1",
                name = "x",
                javaVersion = 7,
            ).validationError(),
        )
        // A supported version is accepted.
        assertNull(
            InstallRequest(
                loader = ModLoader.VANILLA,
                gameVersion = "1.20.1",
                name = "x",
                javaVersion = 17,
            ).validationError(),
        )
    }

    @Test
    fun request_validationRejectsOverlongName() {
        val longName = "实例".repeat(40) // 80 chars > MAX_INSTANCE_NAME_LENGTH (64)
        assertEquals(
            "实例名称过长（最多 ${MAX_INSTANCE_NAME_LENGTH} 字）",
            InstallRequest(
                loader = ModLoader.VANILLA,
                gameVersion = "1.20.1",
                name = longName,
            ).validationError(),
        )
        // Exactly at the limit (64 chars) is still valid.
        val okName = "a".repeat(MAX_INSTANCE_NAME_LENGTH)
        assertNull(
            InstallRequest(
                loader = ModLoader.VANILLA,
                gameVersion = "1.20.1",
                name = okName,
            ).validationError(),
        )
    }

    @Test
    fun gameInstance_duplicateCopiesSettingsAndResetsPlayed() {
        val src = GameInstance(
            id = "fabric-1.20.1",
            name = "Fabric 整合包",
            version = "1.20.1",
            modLoader = ModLoader.FABRIC,
            loaderVersion = "0.16.0",
            javaVersion = 17,
            gameDirectoryType = GameDirectoryType.ISOLATED,
            lastPlayed = 123456L,
            isFavorite = true,
        )
        val clone = src.duplicate("fabric-1.20.1-copy", "Fabric 副本")
        assertEquals("fabric-1.20.1-copy", clone.id)
        assertEquals("Fabric 副本", clone.name)
        // settings preserved
        assertEquals(src.version, clone.version)
        assertEquals(src.modLoader, clone.modLoader)
        assertEquals(src.loaderVersion, clone.loaderVersion)
        assertEquals(src.javaVersion, clone.javaVersion)
        assertEquals(src.gameDirectoryType, clone.gameDirectoryType)
        // per-run state reset
        assertEquals(0L, clone.lastPlayed)
        assertEquals(false, clone.isFavorite)
        // original untouched
        assertEquals(123456L, src.lastPlayed)
    }

    @Test
    fun gameInstance_duplicateFallsBackToDefaultName() {
        val src = GameInstance("v", "原版", "1.20.1")
        val clone = src.duplicate("v-copy")
        assertEquals("原版 副本", clone.name)
    }

    @Test
    fun step_progressCounts() {
        val vanilla = InstallRequest(loader = ModLoader.VANILLA, gameVersion = "1.20.1")
        val fabric = vanilla.copy(loader = ModLoader.FABRIC)
        assertEquals(4, InstallStep.REVIEW.totalSteps(vanilla))
        assertEquals(5, InstallStep.REVIEW.totalSteps(fabric))
        assertEquals(4, InstallStep.REVIEW.stepNumber(vanilla))
        assertEquals(5, InstallStep.REVIEW.stepNumber(fabric))
    }
}
