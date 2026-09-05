package com.rc.launcher.ui.model

import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.parseJson

/**
 * A packaged control layout as it appears in the community / built-in library
 * (task 15).
 *
 * A [ControlLayoutPackage] is what gets *distributed*: in addition to the plain
 * [ControlLayout] payload it carries the publisher, version, summary, tags and
 * a SHA-1 checksum so the launcher can verify a downloaded file before applying
 * it. The same struct also covers `.zip` packages whose payload is a single
 * `layout.json` (plus optional `meta.json`) - the format used by FCL-Controllers
 * and ZalithLauncher community layouts.
 *
 * The model is plain Kotlin (no Android imports) so it is unit-testable on the
 * JVM and round-trips through [MiniJson].
 */
data class ControlLayoutPackage(
    val id: String,
    val name: String,
    val author: String = "",
    val version: String = "",
    val description: String = "",
    val tags: List<String> = emptyList(),
    val screenshots: List<String> = emptyList(),
    val layout: ControlLayout,
    /** Lower-case hex SHA-1 of the payload file (40 chars), or null if unknown. */
    val sha1: String? = null,
    /** Stable URL of the payload (used as the primary download target). */
    val downloadUrl: String? = null,
    /** Additional mirror URLs tried in order when the primary fails. */
    val mirrors: List<String> = emptyList(),
    /** "json" or "zip"; controls how the payload is unpacked. */
    val format: String = "json",
    /** Bytes of the payload file, if known ahead of time. */
    val size: Long? = null,
) {
    init {
        require(id.isNotBlank()) { "ControlLayoutPackage.id must not be blank" }
        require(name.isNotBlank()) { "ControlLayoutPackage.name must not be blank" }
        require(format in ALLOWED_FORMATS) {
            "Unsupported package format '$format' (allowed: $ALLOWED_FORMATS)"
        }
    }

    /** Author + version as a single display string ("alice@1.2.0"). */
    fun authorVersion(): String {
        val a = author.takeIf { it.isNotBlank() }
        val v = version.takeIf { it.isNotBlank() }
        return when {
            a != null && v != null -> "$a \u00b7 $v"
            a != null -> a
            v != null -> v
            else -> ""
        }
    }

    companion object {
        const val FORMAT_JSON = "json"
        const val FORMAT_ZIP = "zip"
        val ALLOWED_FORMATS = listOf(FORMAT_JSON, FORMAT_ZIP)
    }
}

/**
 * Severity of an [ImportConflict] when [ControlLayoutImporter.apply] detects the
 * incoming package would overwrite an existing layout. Mirrors FCL/Zalith's
 * "overwrite / rename / cancel" prompts.
 */
enum class ConflictResolution { OVERWRITE, RENAME, SKIP }

/**
 * A conflict reported when importing a [ControlLayoutPackage] into the
 * repository: an existing entry (built-in or user-saved) shares the package id
 * (or has a name collision). The UI lets the user pick a [resolution]; the
 * importer then turns the resolution into a final id/name pair.
 */
data class ImportConflict(
    val kind: Kind,
    val existingId: String,
    val existingName: String,
    val incomingId: String,
    val incomingName: String,
) {
    enum class Kind { ID, NAME }
}

/**
 * Source descriptor for a single community / built-in catalog entry (task 15).
 *
 * Mirrors the FCL-Controllers / ZalithLauncher convention of a flat
 * `repo_json/` index: each entry describes where to fetch a packaged layout, who
 * authored it, and what extra mirrors can substitute when the primary host is
 * down (essential on the mainland-China network, see task 3 / task 29).
 */
data class ControlLayoutSource(
    val id: String,
    val name: String,
    val author: String,
    val version: String,
    val description: String,
    val tags: List<String>,
    val url: String,
    val mirrors: List<String>,
    val format: String,
    val sha1: String?,
    val size: Long?,
) {
    /** Convert this source into a packaged-layout payload reference (no layout body yet). */
    fun toPackageRef(): ControlLayoutPackage = ControlLayoutPackage(
        id = id,
        name = name,
        author = author,
        version = version,
        description = description,
        tags = tags,
        layout = ControlLayout(id = id, name = name, editable = false),
        sha1 = sha1,
        downloadUrl = url,
        mirrors = mirrors,
        format = format,
        size = size,
    )

    companion object {
        /**
         * Parse a single source entry from a `repo_json/<id>/version.json`-style
         * JSON object (the field set matches FCL-Controllers' catalogue).
         */
        fun fromJson(obj: JsonValue.Obj): ControlLayoutSource? {
            val id = (obj.entries["id"] as? JsonValue.Str)?.value ?: return null
            val name = (obj.entries["name"] as? JsonValue.Str)?.value ?: id
            val url = (obj.entries["url"] as? JsonValue.Str)?.value ?: return null
            val tagsArr = (obj.entries["tags"] as? JsonValue.Arr)?.items
            val mirrorsArr = (obj.entries["mirrors"] as? JsonValue.Arr)?.items
            return ControlLayoutSource(
                id = id,
                name = name,
                author = (obj.entries["author"] as? JsonValue.Str)?.value ?: "",
                version = (obj.entries["version"] as? JsonValue.Str)?.value ?: "",
                description = (obj.entries["description"] as? JsonValue.Str)?.value ?: "",
                tags = tagsArr?.mapNotNull { (it as? JsonValue.Str)?.value } ?: emptyList(),
                url = url,
                mirrors = mirrorsArr?.mapNotNull { (it as? JsonValue.Str)?.value } ?: emptyList(),
                format = (obj.entries["format"] as? JsonValue.Str)?.value ?: "json",
                sha1 = (obj.entries["sha1"] as? JsonValue.Str)?.value,
                size = (obj.entries["size"] as? JsonValue.Num)?.value?.toLong(),
            )
        }
    }
}

/**
 * The built-in / community catalog served by the launcher (task 15).
 *
 * The shipped catalogue is deliberately small and offline: it lists a few
 * well-known community layouts (FCL-Controllers mirrors) so the library UI is
 * populated on a fresh install. Real users reach the network through the
 * configured [MirrorSource] (task 29) and download the actual payload through
 * [com.rc.launcher.core.RustBridge.downloadAsync] so resumable downloads,
 * checksum verification and mirror fallback all go through the Rust core.
 */
object ControlLayoutLibraryCatalog {
    val all: List<ControlLayoutSource> = listOf(
        ControlLayoutSource(
            id = "fcl_community_default",
            name = "FCL \u793e\u533a\u9ed8\u8ba4",
            author = "FCL-Team",
            version = "1.0",
            description = "FCL \u793e\u533a\u63a8\u8350\u7684\u9ed8\u8ba4\u89e6\u63a7\u5e03\u5c40\uff0c\u9002\u5408\u5927\u591a\u6570\u6e38\u620f\u573a\u666f\u3002",
            tags = listOf("\u63a8\u8350", "\u901a\u7528"),
            url = "https://raw.githubusercontent.com/FCL-Team/FCL-Controllers/main/repo_json/00000000/versions/1.json",
            mirrors = listOf(
                "https://gitee.com/FCL-Team/FCL-Controllers/raw/main/repo_json/00000000/versions/1.json",
            ),
            format = "json",
            sha1 = null,
            size = null,
        ),
        ControlLayoutSource(
            id = "fcl_community_wasd",
            name = "FCL \u793e\u533a WASD",
            author = "FCL-Team",
            version = "1.0",
            description = "\u4ee5 W/A/S/D + \u9f20\u6807\u4e3a\u4e3b\u7684\u5e03\u5c40\uff0c\u9002\u5408\u9ad8\u7cbe\u5ea6\u4efb\u52a1\u3002",
            tags = listOf("WASD", "\u9ad8\u7cbe\u5ea6"),
            url = "https://raw.githubusercontent.com/FCL-Team/FCL-Controllers/main/repo_json/007ade84/versions/1.json",
            mirrors = listOf(
                "https://gitee.com/FCL-Team/FCL-Controllers/raw/main/repo_json/007ade84/versions/1.json",
            ),
            format = "json",
            sha1 = null,
            size = null,
        ),
        ControlLayoutSource(
            id = "fcl_community_gamepad",
            name = "FCL \u793e\u533a\u5916\u63a5\u624b\u67c4",
            author = "FCL-Team",
            version = "1.0",
            description = "\u9488\u5bf9\u5916\u63a5\u624b\u67c4\u4f18\u5316\u7684\u5e03\u5c40\uff0c\u5305\u542b\u4e24\u4e2a\u6447\u6746\u4e0e\u62a5\u7b80\u6309\u94ae\u3002",
            tags = listOf("\u624b\u67c4", "\u9ad8\u7c92\u5ea6"),
            url = "https://raw.githubusercontent.com/FCL-Team/FCL-Controllers/main/repo_json/01e2cb7e/versions/1.json",
            mirrors = listOf(
                "https://gitee.com/FCL-Team/FCL-Controllers/raw/main/repo_json/01e2cb7e/versions/1.json",
            ),
            format = "json",
            sha1 = null,
            size = null,
        ),
    )

    fun byId(id: String): ControlLayoutSource? = all.firstOrNull { it.id == id }
}

// ============================================================================
// MiniJson helpers used by the package / source models
// ============================================================================
//
// Note: ControlLayout.kt already defines `JsonValue.Obj.str/dbl/bool/arr` as
// private helpers for its own parser; we reuse them by parsing the manifest via
// the JSON object's entries map directly. No redefinition here.

/**
 * Try to convert a "manifest.json" / "meta.json" object into a package
 * reference (no payload layout yet). Returns null when the manifest is missing
 * the mandatory id / name pair.
 */
/**
 * Resolve a string-or-null field, returning [fallback] when missing or wrong type.
 */
private fun JsonValue.Obj.getStr(key: String, fallback: String = ""): String =
    (entries[key] as? JsonValue.Str)?.value ?: fallback

private fun JsonValue.Obj.getStrOrNull(key: String): String? =
    (entries[key] as? JsonValue.Str)?.value

private fun JsonValue.Obj.getNum(key: String): Double? =
    (entries[key] as? JsonValue.Num)?.value

private fun JsonValue.Obj.getStrArr(key: String): List<String> =
    (entries[key] as? JsonValue.Arr)?.items
        ?.mapNotNull { (it as? JsonValue.Str)?.value }
        ?: emptyList()

internal fun parsePackageManifest(text: String): ControlLayoutPackage? {
    val root = parseJson(text) as? JsonValue.Obj ?: return null
    val id = root.getStrOrNull("id") ?: return null
    val name = root.getStrOrNull("name") ?: return null
    val layoutId = root.getStrOrNull("layout_id") ?: id
    val layoutName = root.getStrOrNull("layout_name") ?: name
    return ControlLayoutPackage(
        id = id,
        name = name,
        author = root.getStr("author"),
        version = root.getStr("version"),
        description = root.getStr("description"),
        tags = root.getStrArr("tags"),
        screenshots = root.getStrArr("screenshots"),
        layout = ControlLayout(id = layoutId, name = layoutName, editable = false),
        sha1 = root.getStrOrNull("sha1"),
        downloadUrl = root.getStrOrNull("download_url"),
        mirrors = root.getStrArr("mirrors"),
        format = root.getStr("format", "json"),
        size = root.getNum("size")?.toLong(),
    )
}
