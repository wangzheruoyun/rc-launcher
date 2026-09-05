package com.rc.launcher.ui.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/** Unit tests for the control-layout downloader (task 15). */
class ControlLayoutDownloaderTest {

    @Test
    fun sha1Of_knownString_matchesExpectedHex() {
        val tmp = File.createTempFile("rc-sha1-", ".bin").apply {
            // `writeBytes` (not `writeText`) so no trailing newline is added;
            // the SHA-1 then matches the well-known "hello" digest.
            writeBytes(byteArrayOf('h'.code.toByte(), 'e'.code.toByte(), 'l'.code.toByte(), 'l'.code.toByte(), 'o'.code.toByte()))
        }
        try {
            // echo -n hello | sha1sum -> aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d
            assertEquals("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d", JvmControlLayoutDownloader.sha1Of(tmp))
        } finally {
            tmp.delete()
        }
    }

    @Test
    fun downloaders_pureJvm_whenBridgeUnavailable() {
        // When the bridge is null (preview / unit tests) we always get the JVM
        // fallback so the library stays usable without a loaded native library.
        val d = ControlLayoutDownloaders.fromBridge(null)
        assertTrue(d is JvmControlLayoutDownloader)
    }

    @Test
    fun packageRef_roundTripsSourceFields() {
        val src = ControlLayoutLibraryCatalog.all.first()
        val pkg = src.toPackageRef()
        assertEquals(src.id, pkg.id)
        assertEquals(src.url, pkg.downloadUrl)
    }

    @Test
    fun detectConflicts_nameClashOnDifferentIds_returnsConflict() {
        val repo = InMemoryControlLayoutRepository().apply {
            save(ControlLayout("local_aaa", "\u540c\u540d\u5e03\u5c40"))
        }
        val pkg = ControlLayoutPackage(
            id = "external_bbb",
            name = "\u540c\u540d\u5e03\u5c40",
            layout = ControlLayoutCatalog.default(),
        )
        val conflicts = ControlLayoutImporter.detectConflicts(pkg, repo)
        assertTrue(conflicts.any { it.kind == ImportConflict.Kind.NAME })
    }

    @Test
    fun uniqueId_slugifiesAndPrefixed() {
        val id = ControlLayoutImporter.uniqueId("\u4e2d\u6587 layout!")
        assertTrue(id.startsWith("custom_"))
        assertFalse(id.contains(' '))
        assertFalse(id.contains('\u4e2d'))
    }
}
