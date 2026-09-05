package com.rc.launcher.ui.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import com.rc.launcher.ui.model.json.toJsonString
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

/** Unit tests for the control-layout import pipeline (task 15). */
class ControlLayoutImporterTest {

    // -----------------------------------------------------------------------
    // JSON decoding
    // -----------------------------------------------------------------------

    @Test
    fun decode_bareLayoutJson_roundTrips() {
        val layout = ControlLayoutCatalog.default()
        val payload = layout.toJsonString().toByteArray(Charsets.UTF_8)
        val r = ControlLayoutImporter.decode(payload, "default.json")
        assertTrue(r.ok)
        val pkg = r.pkg!!
        assertEquals(layout.id, pkg.id)
        assertEquals(layout.name, pkg.name)
        assertEquals(layout.elements.size, pkg.layout.elements.size)
        assertEquals(ControlLayoutPackage.FORMAT_JSON, pkg.format)
    }

    @Test
    fun decode_packageEnvelopeWithInlineLayout_extractsEverything() {
        val layout = ControlLayoutCatalog.gamepad()
        val envelope = """
            {
              "id": "community_pack",
              "name": "Community pack",
              "author": "alice",
              "version": "1.2.3",
              "description": "An awesome pack",
              "tags": ["PVP", "Quest"],
              "sha1": "0123456789abcdef0123456789abcdef01234567",
              "download_url": "https://example.invalid/pack.json",
              "format": "json",
              "layout": ${layout.toJsonString()}
            }
        """.trimIndent()
        val r = ControlLayoutImporter.decode(envelope.toByteArray(Charsets.UTF_8), "pack.json")
        assertTrue(r.ok)
        val pkg = r.pkg!!
        assertEquals("community_pack", pkg.id)
        assertEquals("Community pack", pkg.name)
        assertEquals("alice", pkg.author)
        assertEquals("1.2.3", pkg.version)
        assertEquals(listOf("PVP", "Quest"), pkg.tags)
        assertEquals("https://example.invalid/pack.json", pkg.downloadUrl)
        assertEquals(layout.elements.size, pkg.layout.elements.size)
    }

    @Test
    fun decode_invalidJson_returnsFailure() {
        val r = ControlLayoutImporter.decode("not json".toByteArray(), "bad.json")
        assertFalse(r.ok)
        assertNull(r.pkg)
        assertTrue(r.issues.isNotEmpty())
    }

    @Test
    fun decode_unsupportedRootArray_returnsFailure() {
        val r = ControlLayoutImporter.decode("[]".toByteArray(), "arr.json")
        assertFalse(r.ok)
        assertEquals(1, r.issues.size)
    }

    // -----------------------------------------------------------------------
    // ZIP decoding
    // -----------------------------------------------------------------------

    @Test
    fun decode_zipWithMetaAndLayout_loadsBoth() {
        val layout = ControlLayoutCatalog.wasd()
        val zipBytes = buildZip(
            "meta.json" to """
                {"id":"zip_pack","name":"ZIP pack","author":"bob","version":"2.0",
                 "tags":["Survival"],"format":"zip"}
            """.trimIndent(),
            "layout.json" to layout.toJsonString(),
        )
        val r = ControlLayoutImporter.decode(zipBytes, "pack.zip")
        assertTrue(r.ok)
        val pkg = r.pkg!!
        assertEquals("zip_pack", pkg.id)
        assertEquals("ZIP pack", pkg.name)
        assertEquals("bob", pkg.author)
        assertEquals(layout.elements.size, pkg.layout.elements.size)
    }

    @Test
    fun decode_zipWithoutMeta_emitsWarningButStillSucceeds() {
        val layout = ControlLayoutCatalog.default()
        val zipBytes = buildZip("layout.json" to layout.toJsonString())
        val r = ControlLayoutImporter.decode(zipBytes, "pack.zip")
        assertTrue(r.ok)
        val pkg = r.pkg!!
        assertEquals(layout.id, pkg.id)
        assertEquals(layout.name, pkg.name)
        assertTrue(r.issues.any { it.severity == IssueSeverity.WARNING })
    }

    @Test
    fun decode_zipMissingLayout_returnsFailure() {
        val zipBytes = buildZip("meta.json" to """{"id":"x","name":"X"}""")
        val r = ControlLayoutImporter.decode(zipBytes, "pack.zip")
        assertFalse(r.ok)
        assertNull(r.pkg)
    }

    @Test
    fun decode_zipWithScreenshots_ignoresThem() {
        val layout = ControlLayoutCatalog.default()
        val zipBytes = buildZip(
            "layout.json" to layout.toJsonString(),
            "screenshots/01.png" to "fake-png-bytes",
        )
        val r = ControlLayoutImporter.decode(zipBytes, "pack.zip")
        assertTrue(r.ok)
    }

    // -----------------------------------------------------------------------
    // Conflict detection
    // ---------------------------------------------------------------------

    @Test
    fun detectConflicts_builtInIdClash_flagsConflict() {
        val pkg = ControlLayoutPackage(
            id = ControlLayout.DEFAULT_ID, // built-in id
            name = "My fork",
            layout = ControlLayoutCatalog.default(),
        )
        val conflicts = ControlLayoutImporter.detectConflicts(pkg, InMemoryControlLayoutRepository())
        assertEquals(1, conflicts.size)
        assertEquals(ImportConflict.Kind.ID, conflicts.first().kind)
    }

    @Test
    fun detectConflicts_savedIdClash_flagsConflict() {
        val repo = InMemoryControlLayoutRepository().apply {
            save(ControlLayout("custom1", "My saved", editable = true))
        }
        val pkg = ControlLayoutPackage(
            id = "custom1",
            name = "My saved (new)",
            layout = ControlLayoutCatalog.default(),
        )
        val conflicts = ControlLayoutImporter.detectConflicts(pkg, repo)
        assertTrue(conflicts.any { it.kind == ImportConflict.Kind.ID })
    }

    @Test
    fun detectConflicts_noClash_returnsEmpty() {
        val pkg = ControlLayoutPackage(
            id = "unique_xyz",
            name = "Unique",
            layout = ControlLayoutCatalog.default(),
        )
        val conflicts = ControlLayoutImporter.detectConflicts(pkg, InMemoryControlLayoutRepository())
        assertTrue(conflicts.isEmpty())
    }

    // -----------------------------------------------------------------------
    // Resolution
    // -----------------------------------------------------------------------

    @Test
    fun resolve_overwrite_keepsIdAndName() {
        val pkg = ControlLayoutPackage(id = "dup", name = "Dup", layout = ControlLayoutCatalog.default())
        val c = ImportConflict(ImportConflict.Kind.ID, "dup", "Dup", "dup", "Dup")
        val resolved = ControlLayoutImporter.resolve(pkg, listOf(c)) { ConflictResolution.OVERWRITE }
        assertEquals("dup", resolved.pkg.id)
        assertTrue(resolved.saved)
        assertFalse(resolved.skipped)
    }

    @Test
    fun resolve_rename_changesToUniqueId() {
        val pkg = ControlLayoutPackage(id = "dup", name = "Dup", layout = ControlLayoutCatalog.default())
        val c = ImportConflict(ImportConflict.Kind.ID, "dup", "Dup", "dup", "Dup")
        val resolved = ControlLayoutImporter.resolve(pkg, listOf(c)) { ConflictResolution.RENAME }
        assertFalse(resolved.pkg.id == "dup")
        assertTrue(resolved.pkg.id.startsWith("custom_"))
        assertTrue(resolved.pkg.name.contains("\u526f\u672c"))
    }

    @Test
    fun resolve_skip_abortsImport() {
        val pkg = ControlLayoutPackage(id = "dup", name = "Dup", layout = ControlLayoutCatalog.default())
        val c = ImportConflict(ImportConflict.Kind.ID, "dup", "Dup", "dup", "Dup")
        val resolved = ControlLayoutImporter.resolve(pkg, listOf(c)) { ConflictResolution.SKIP }
        assertTrue(resolved.skipped)
        assertFalse(resolved.saved)
    }

    // -----------------------------------------------------------------------
    // Catalog
    // -----------------------------------------------------------------------

    @Test
    fun catalogContainsCommunityEntries() {
        val sources = ControlLayoutLibraryCatalog.all
        assertTrue(sources.isNotEmpty())
        // Every source has the minimum required fields.
        for (s in sources) {
            assertTrue(s.id.isNotBlank())
            assertTrue(s.name.isNotBlank())
            assertTrue(s.url.isNotBlank())
            assertTrue(s.format in ControlLayoutPackage.ALLOWED_FORMATS)
        }
    }

    @Test
    fun sourceToPackageRef_preservesMetadata() {
        val src = ControlLayoutLibraryCatalog.all.first()
        val pkg = src.toPackageRef()
        assertEquals(src.id, pkg.id)
        assertEquals(src.author, pkg.author)
        assertEquals(src.version, pkg.version)
        assertEquals(src.url, pkg.downloadUrl)
        assertEquals(src.mirrors, pkg.mirrors)
        assertEquals(src.format, pkg.format)
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    private fun buildZip(vararg entries: Pair<String, String>): ByteArray {
        val out = java.io.ByteArrayOutputStream()
        ZipOutputStream(out).use { zos ->
            for ((name, content) in entries) {
                zos.putNextEntry(ZipEntry(name))
                zos.write(content.toByteArray(Charsets.UTF_8))
                zos.closeEntry()
            }
        }
        return out.toByteArray()
    }
}
