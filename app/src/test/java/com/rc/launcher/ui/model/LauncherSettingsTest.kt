package com.rc.launcher.ui.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Unit tests for the settings data model (task 14). */
class LauncherSettingsTest {

    @Test
    fun defaults_areValid() {
        val s = LauncherSettings()
        assertEquals(null, s.validationError())
        assertEquals(MirrorCatalog.BMCLAPI.id, s.mirrorId)
        assertEquals(RendererOption.DEFAULT.id, s.rendererId)
        assertEquals(LauncherSettings.DEFAULT_HEAP_MB, s.javaHeapMb)
    }

    @Test
    fun sanitize_clampsHeapAboveMax() {
        val dirty = LauncherSettings(javaHeapMb = 999_999)
        val clean = dirty.sanitized()
        assertEquals(LauncherSettings.MAX_HEAP_MB, clean.javaHeapMb)
        assertEquals(null, clean.validationError())
    }

    @Test
    fun sanitize_clampsHeapBelowMin() {
        val clean = LauncherSettings(javaHeapMb = 1).sanitized()
        assertEquals(LauncherSettings.MIN_HEAP_MB, clean.javaHeapMb)
    }

    @Test
    fun sanitize_repairsInvalidMirror() {
        val clean = LauncherSettings(mirrorId = "does-not-exist").sanitized()
        assertEquals(MirrorCatalog.BMCLAPI.id, clean.mirrorId)
    }

    @Test
    fun sanitize_repairsInvalidRenderer() {
        val clean = LauncherSettings(rendererId = "bogus").sanitized()
        assertEquals(RendererOption.DEFAULT.id, clean.rendererId)
    }

    @Test
    fun sanitize_dropsMinHeapAboveMax() {
        val clean = LauncherSettings(javaHeapMb = 1024, javaMinHeapMb = 4096).sanitized()
        // min was larger than max -> clamped to max, then validation passes.
        assertEquals(1024, clean.javaMinHeapMb)
        assertEquals(null, clean.validationError())
    }

    @Test
    fun sanitize_zeroMinHeapBecomesNull() {
        val clean = LauncherSettings(javaMinHeapMb = 0).sanitized()
        assertEquals(null, clean.javaMinHeapMb)
    }

    @Test
    fun sanitize_clampsResolutionScale() {
        assertEquals(2f, LauncherSettings(resolutionScale = 99f).sanitized().resolutionScale)
        assertEquals(0.25f, LauncherSettings(resolutionScale = -5f).sanitized().resolutionScale)
    }

    @Test
    fun sanitize_clampsFramerate() {
        assertEquals(LauncherSettings.MAX_FRAMERATE, LauncherSettings(framerateLimit = 9999).sanitized().framerateLimit)
        assertEquals(0, LauncherSettings(framerateLimit = -10).sanitized().framerateLimit)
    }

    @Test
    fun sanitize_fixesBrokenDohWhenEnabled() {
        val clean = LauncherSettings(useDoh = true, dohServerUrl = "ftp://nope").sanitized()
        assertEquals(DohCatalog.ALIYUN.url, clean.dohServerUrl)
    }

    @Test
    fun validation_reportsMinAboveMax() {
        val bad = LauncherSettings(javaHeapMb = 512, javaMinHeapMb = 1024)
        assertEquals("Java 初始内存不能大于最大内存", bad.validationError())
    }

    @Test
    fun resolutionMode_custom_usesCustomSize() {
        val s = LauncherSettings(
            resolutionMode = ResolutionMode.CUSTOM,
            customWidth = 1920,
            customHeight = 1080,
        )
        assertEquals(WindowSize(1920, 1080), s.windowSize())
        val auto = LauncherSettings(resolutionMode = ResolutionMode.AUTO)
        assertEquals(WindowSize.DEFAULT, auto.windowSize())
    }

    @Test
    fun autoAllocateMemory_selectsHeapOrFallsBackToStored() {
        val auto = LauncherSettings(autoAllocateMemory = true, javaHeapMb = 2048)
        // device with 4 GiB -> ~1.3 GiB, clamped to 8192 max but 1331 is fine.
        val picked = auto.effectiveHeapMb(4 * 1024)
        assertTrue(picked in LauncherSettings.MIN_HEAP_MB..LauncherSettings.MAX_HEAP_MB)
        assertTrue(picked <= auto.javaHeapMb) // auto never exceeds the stored cap intent? not required; just sane

        val manual = LauncherSettings(autoAllocateMemory = false, javaHeapMb = 512)
        assertEquals(512, manual.effectiveHeapMb(8 * 1024))
    }

    @Test
    fun autoHeapFor_clamps() {
        assertEquals(LauncherSettings.DEFAULT_HEAP_MB, LauncherSettings.autoHeapFor(0))
        assertTrue(LauncherSettings.autoHeapFor(512) <= LauncherSettings.MAX_HEAP_MB)
    }

    @Test
    fun catalogFallbacks_neverReturnNull() {
        assertEquals(MirrorCatalog.BMCLAPI, MirrorCatalog.fromId(null))
        assertEquals(MirrorCatalog.BMCLAPI, MirrorCatalog.fromId("???"))
        assertEquals(RendererOption.DEFAULT, RendererOption.fromId(null))
        assertEquals(RendererOption.DEFAULT, RendererOption.fromId("???"))
        assertEquals(DohCatalog.ALIYUN, DohCatalog.fromUrl(null))
    }

    @Test
    fun mirror_officialReturnsNullRenderer() {
        val official = LauncherSettings(mirrorId = MirrorCatalog.OFFICIAL.id)
        assertEquals(null, official.mirror())
        assertEquals(MirrorCatalog.BMCLAPI, LauncherSettings().mirror())
    }

    @Test
    fun windowSize_scaledClampsToBounds() {
        val big = WindowSize.DEFAULT.scaled(10f)
        assertTrue(big.width <= WindowSize.MAX_W)
        val small = WindowSize.DEFAULT.scaled(0.01f)
        assertTrue(small.width >= WindowSize.MIN_W)
        assertFalse(small.width == 0)
    }
}
