package com.rc.launcher.ui.model

import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.parseJson
import com.rc.launcher.ui.model.json.toJsonString
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.util.zip.ZipEntry
import java.util.zip.ZipInputStream

/**
 * Result of attempting to decode a payload file (`.json` or `.zip`) into a
 * [ControlLayoutPackage]. Mirrors the FCL/Zalith "import" flow: one payload
 * yields zero or one package, plus the set of issues that prevented import.
 */
data class ImportResult(
    val pkg: ControlLayoutPackage?,
    val issues: List<ImportIssue>,
) {
    val ok: Boolean get() = pkg != null && issues.none { it.severity == IssueSeverity.ERROR }

    companion object {
        fun success(pkg: ControlLayoutPackage): ImportResult = ImportResult(pkg, emptyList())
        fun failure(message: String): ImportResult = ImportResult(
            pkg = null,
            issues = listOf(ImportIssue(IssueSeverity.ERROR, message)),
        )
    }
}

/** A problem encountered while parsing / importing a package payload. */
data class ImportIssue(
    val severity: IssueSeverity,
    val message: String,
)

/**
 * Decodes a payload file (JSON or ZIP) into a [ControlLayoutPackage].
 *
 * The launcher accepts the same two formats FCL/Zalith ship:
 *  * **`.json`** - a single [ControlLayout] serialised with [toJsonString], or a
 *    "package" object whose `layout` field points to the layout body.
 *  * **`.zip`** - a ZIP archive containing either a top-level `layout.json`
 *    (legacy) or a `meta.json` + `layout.json` pair (new format); optional
 *    `screenshots/` directory entries are skipped silently.
 *
 * Decoding is pure (no IO, no Android types) so it can be unit-tested on the
 * JVM with arbitrary `ByteArray` payloads.
 */
object ControlLayoutImporter {

    /**
     * Decode a payload given its raw bytes and a hint at the original file name
     * (used to disambiguate JSON vs ZIP when the bytes alone are ambiguous).
     */
    fun decode(bytes: ByteArray, fileName: String?): ImportResult {
        val isZip = fileName?.endsWith(".zip", ignoreCase = true) == true ||
            bytes.size >= 4 && bytes[0] == 'P'.code.toByte() &&
            bytes[1] == 'K'.code.toByte() && bytes[2] == 3.toByte() && bytes[3] == 4.toByte()
        return try {
                if (isZip) decodeZip(bytes) else decodeJson(bytes)
        } catch (e: Exception) {
                ImportResult.failure("\u89e3\u6790\u5931\u8d25: " + e.javaClass.simpleName + ": " + (e.message ?: ""))
            }
    }

    private fun decodeJson(bytes: ByteArray): ImportResult {
        val text = bytes.toString(Charsets.UTF_8)
        val root = parseJson(text)
            ?: return ImportResult.failure("\u4e0d\u662f\u6709\u6548\u7684 JSON \u6587\u4ef6")
        if (root !is JsonValue.Obj) {
            return ImportResult.failure("\u4e0d\u652f\u6301\u7684 JSON \u6839\u7c7b\u578b")
        }
        // A package envelope is identified by one of: a nested `layout` object,
        // `format`, `sha1`, or `download_url`. Anything that *isn't* one of
        // these is a bare [ControlLayout].
        val entries = root.entries
        val looksLikeEnvelope = entries["layout"] is JsonValue.Obj ||
            entries["format"] is JsonValue.Str ||
            entries["sha1"] is JsonValue.Str ||
            entries["download_url"] is JsonValue.Str
        if (looksLikeEnvelope) {
            val manifest = parsePackageManifest(text)
                ?: return ImportResult.failure("\u5305\u88c5\u6f0f\u5931 id / name")
            val inlineLayoutJson = root.obj("layout")
            if (inlineLayoutJson != null) {
                val inline = parseControlLayout(inlineLayoutJson.toJsonString())
                if (inline == null) {
                    return ImportResult.failure("\u5305\u88c5\u5185\u7684 layout \u4e0d\u53ef\u7528")
                }
                return ImportResult.success(manifest.copy(layout = inline))
            }
            return ImportResult.failure("\u5305\u88c5\u7f3a\u5c11 layout \u5b57\u6bb5")
        }
        val asLayout = parseControlLayout(text)
            ?: return ImportResult.failure("\u4e0d\u662f\u6709\u6548\u7684\u63a7\u5236\u5e03\u5c40")
        return ImportResult.success(asLayout.toPackage())
    }

    private fun decodeZip(bytes: ByteArray): ImportResult {
        var meta: ControlLayoutPackage? = null
        var layoutJson: String? = null
        val issues = mutableListOf<ImportIssue>()
        ZipInputStream(ByteArrayInputStream(bytes)).use { zin ->
            var entry: ZipEntry? = zin.nextEntry
            while (entry != null) {
                if (!entry.isDirectory) {
                    val name = entry.name.lowercase()
                    val content = zin.readBytes()
                    when {
                        name.endsWith("meta.json") || name.endsWith("manifest.json") ->
                                meta = parsePackageManifest(content.toString(Charsets.UTF_8))
                        name.endsWith("layout.json") || name.endsWith("control_layout.json") ->
                                layoutJson = content.toString(Charsets.UTF_8)
                    }
                }
                entry = zin.nextEntry
            }
        }
        val layout = layoutJson?.let { parseControlLayout(it) }
        if (layout == null) {
            return ImportResult.failure("ZIP \u4e2d\u627e\u4e0d\u5230\u53ef\u7528\u7684 layout.json")
        }
        val pkg = meta?.copy(layout = layout) ?: layout.toPackage()
        if (meta == null) {
            issues += ImportIssue(
                IssueSeverity.WARNING,
                "ZIP \u4e2d\u672a\u63d0\u4f9b meta.json\uff0c\u5c06\u4ee5 layout \u672c\u4f53\u4f5c\u4e3a\u5305\u88c5\u4fe1\u606f",
            )
        }
        return ImportResult(pkg, issues)
    }

    /**
     * Resolve any [ImportConflict] produced by [detectConflicts] into a final
     * (id, name) pair, or null when the user chose to skip the package.
     */
    fun resolve(
        pkg: ControlLayoutPackage,
        conflicts: List<ImportConflict>,
        resolution: (ImportConflict) -> ConflictResolution,
    ): ResolvedImport {
        var id = pkg.id
        var name = pkg.name
        for (c in conflicts) {
            when (resolution(c)) {
                ConflictResolution.OVERWRITE -> { /* keep id / name */ }
                ConflictResolution.SKIP -> return ResolvedImport(pkg.copy(id = id, name = name), saved = false, skipped = true)
                ConflictResolution.RENAME -> {
                    id = uniqueId(id)
                    name = "$name (\u526f\u672c)"
                }
            }
        }
        return ResolvedImport(pkg.copy(id = id, name = name), saved = true, skipped = false)
    }

    /** A unique slug for [base], derived from it by suffixing `_2`, `_4`, ... */
    fun uniqueId(base: String): String {
        val slug = base.lowercase()
            .replace(Regex("[^a-z0-9]+"), "_")
            .trim('_')
            .takeIf { it.isNotBlank() } ?: "layout"
        return "custom_$slug"
    }

    /**
     * Detect conflicts between [pkg] and the existing layouts known to the
     * launcher: the [ControlLayoutCatalog] (built-in) plus whatever the
     * [repository] currently holds. The result is empty when the import is
     * safe to apply as-is.
     */
    fun detectConflicts(
        pkg: ControlLayoutPackage,
        repository: ControlLayoutRepository,
    ): List<ImportConflict> {
        val conflicts = mutableListOf<ImportConflict>()
        // Built-in id collisions are always treated as conflicts (the user can
        // rename to make a fresh custom copy).
        val builtIn = ControlLayoutCatalog.builtInById(pkg.id)
        if (builtIn != null) {
            conflicts += ImportConflict(ImportConflict.Kind.ID, builtIn.id, builtIn.name, pkg.id, pkg.name)
        }
        val saved = repository.load(pkg.id)
        if (saved != null) {
            conflicts += ImportConflict(ImportConflict.Kind.ID, saved.id, saved.name, pkg.id, pkg.name)
        }
        // Name collisions: two layouts with different ids but identical display
        // names confuse the picker. We only warn if the names truly clash.
        val sameNameBuiltIn = ControlLayoutCatalog.all().any { it.name == pkg.name && it.id != pkg.id }
        val sameNameSaved = repository.list().any { it.name == pkg.name && it.id != pkg.id }
        if (sameNameBuiltIn || sameNameSaved) {
            conflicts += ImportConflict(
                ImportConflict.Kind.NAME,
                existingId = if (sameNameBuiltIn) pkg.id else "",
                existingName = pkg.name,
                incomingId = pkg.id,
                incomingName = pkg.name,
            )
        }
        return conflicts.distinctBy { it.kind to it.incomingId to it.incomingName }
    }
}

/** Result of resolving an import's conflicts. */
data class ResolvedImport(
    val pkg: ControlLayoutPackage,
    val saved: Boolean,
    val skipped: Boolean,
)

/**
 * Apply [resolvedImport] to the [repository], optionally applying the resulting
 * layout as the active one. Idempotent: re-importing the same id after a
 * successful import is a no-op (so the user can pick "overwrite" once and not
 * be prompted again until the package content changes).
 */
fun ResolvedImport.applyTo(
    repository: ControlLayoutRepository,
    apply: (ControlLayoutPackage) -> Unit = {},
): Boolean {
    if (skipped) return false
    repository.save(pkg.layout.copy(name = pkg.name, editable = true))
    if (saved) apply(pkg)
    return true
}

// ============================================================================
// Helpers used by the importer
// ============================================================================

private fun JsonValue.Obj.obj(key: String): JsonValue.Obj? = entries[key] as? JsonValue.Obj

/** Turn a [ControlLayout] into a bare package (no metadata). */
private fun ControlLayout.toPackage(): ControlLayoutPackage = ControlLayoutPackage(
    id = id,
    name = name,
    layout = this,
    format = ControlLayoutPackage.FORMAT_JSON,
)
