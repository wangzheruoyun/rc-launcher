package com.rc.launcher.ui.viewmodel

import android.content.Context
import androidx.lifecycle.ViewModel
import com.rc.launcher.ui.model.GameInstance
import com.rc.launcher.ui.model.InstanceRepository
import com.rc.launcher.ui.model.ResourcePackData
import com.rc.launcher.ui.model.effectiveGameDir
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import java.io.File

/**
 * State container for the resource-pack management screen (task 27).
 *
 * Scans the per-instance `resourcepacks/` directory, parses each pack's
 * `pack.mcmeta` for the `pack_format` and description, and exposes a sorted
 * [StateFlow] of [ResourcePackData]. Enable/disable is done via the `.disabled`
 * suffix rename convention (same as the Rust core's
 * [`crate::mods::resource_pack::ResourcePackManager::set_enabled`]), so the
 * two layers never disagree about state.
 */
class ResourcePackManagerViewModel(
    private val context: Context? = null,
    private val repository: InstanceRepository = InstanceRepository,
) : ViewModel() {

    private val scope: kotlinx.coroutines.CoroutineScope = kotlinx.coroutines.MainScope()

    private val _state = MutableStateFlow(ResourcePackUiState())
    val state: StateFlow<ResourcePackUiState> = _state.asStateFlow()

    /**
     * Load resource packs for the instance identified by [instanceId].
     * The game directory is resolved through [GameInstance.effectiveGameDir]
     * to honour the per-instance version-isolation strategy.
     */
    fun load(instanceId: String) {
        val inst = repository.getById(instanceId)
        val gameDir = inst?.let { it.effectiveGameDir(resolveBaseDir()) }
            ?: File(resolveBaseDir(), ".minecraft").absolutePath
        val rpDir = File(gameDir, "resourcepacks")

        scope.launch {
            val packs = scanResourcePacks(rpDir)
            _state.value = _state.value.copy(
                instanceId = instanceId,
                resourcePacks = packs,
                resourcePacksDir = rpDir,
            )
        }
    }

    /** Toggle a pack's enabled state by renaming with/without `.disabled`. */
    fun setEnabled(pack: ResourcePackData, enabled: Boolean) {
        scope.launch {
            val newName = if (enabled) {
                pack.path.name.removeSuffix(".disabled")
            } else {
                if (pack.path.name.lowercase().endsWith(".disabled")) {
                    pack.path.name
                } else "${pack.path.name}.disabled"
            }
            val newPath = File(pack.path.parentFile, newName)
            try {
                pack.path.renameTo(newPath)
                _state.value = _state.value.copy(lastError = null, lastSuccess = "Updated")
            } catch (e: Exception) {
                _state.value = _state.value.copy(
                    lastError = e.message ?: "Operation failed",
                    lastSuccess = null,
                )
            }
            reloadCurrent()
        }
    }

    /** Import a resource pack from a local `.zip` or folder. */
    fun importPack(source: File) {
        scope.launch {
            val rpDir = _state.value.resourcePacksDir ?: return@launch
            try {
                val dest = File(rpDir, source.name)
                if (source.isDirectory) {
                    copyDir(source, dest)
                } else {
                    source.copyTo(dest, overwrite = true)
                }
                _state.value = _state.value.copy(lastError = null, lastSuccess = "Imported")
            } catch (e: Exception) {
                _state.value = _state.value.copy(
                    lastError = e.message ?: "Import failed",
                    lastSuccess = null,
                )
            }
            reloadCurrent()
        }
    }

    /** Delete a resource pack. */
    fun delete(pack: ResourcePackData, confirm: Boolean) {
        if (!confirm) return
        scope.launch {
            try {
                if (pack.path.isDirectory) {
                    pack.path.deleteRecursively()
                } else {
                    pack.path.delete()
                }
                _state.value = _state.value.copy(lastError = null, lastSuccess = "Deleted")
            } catch (e: Exception) {
                _state.value = _state.value.copy(
                    lastError = e.message ?: "Delete failed",
                    lastSuccess = null,
                )
            }
            reloadCurrent()
        }
    }

    /** Download a resource pack from a mirror / online source URL. */
    fun downloadFromUrl(url: String) {
        // In production this goes through download::Manager (task 2 / task 30)
        // with mirror + range + checksum support. Here we stub the entry point.
        scope.launch {
            _state.value = _state.value.copy(lastError = "Download from URL not yet wired to the Rust core", lastSuccess = null)
        }
    }

    fun clearMessage() {
        _state.value = _state.value.copy(lastError = null, lastSuccess = null)
    }

    private fun reloadCurrent() {
        val current = _state.value
        if (current.instanceId.isNotBlank()) {
            load(current.instanceId)
        }
    }

    private fun resolveBaseDir(): String {
        val ctx = context ?: return "/sdcard/games"
        return ctx.getExternalFilesDir(null)?.parentFile?.parentFile?.absolutePath
            ?: "/sdcard/games"
    }

    private fun scanResourcePacks(dir: File): List<ResourcePackData> {
        if (!dir.exists() || !dir.isDirectory) return emptyList()
        val out = ArrayList<ResourcePackData>()
        val children = dir.listFiles() ?: return emptyList()
        for (child in children) {
            val isDisabled = child.name.lowercase().endsWith(".disabled") ||
                child.name.lowercase().endsWith(".disabled.zip")
            val ext = child.extension.lowercase()
            val isPack = child.isDirectory || ext == "zip"
            if (!isPack) continue
            val baseName = if (isDisabled) {
                child.name.removeSuffix(".disabled")
            } else child.name
            val enabled = !isDisabled

            val (packFormat, description) = readPackMeta(child)
            // Compatibility check is handled by the Rust core (pack_format_compatible);
            // the Kotlin side surfaces the raw format so the UI can show the warning.
            out.add(ResourcePackData(
                name = baseName,
                path = child,
                enabled = enabled,
                packFormat = packFormat,
                description = description,
            ))
        }
        return out.sortedBy { it.name.lowercase() }
    }

    private fun readPackMeta(path: File): Pair<Int?, String?> {
        val mcmeta = if (path.isDirectory) {
            File(path, "pack.mcmeta")
        } else {
            // For zip archives, read pack.mcmeta from the zip.
            readPackMcmetaFromZip(path)
        }
        if (mcmeta == null || !mcmeta.exists()) return Pair(null, null)
        return try {
            val text = if (path.isDirectory) mcmeta.readText() else readZipEntry(path, "pack.mcmeta") ?: ""
            // Very simple JSON extraction for "pack_format" and "description"
            val fmt = extractJsonInt(text, "pack_format")
            val desc = extractJsonString(text, "description")
            Pair(fmt, desc)
        } catch (_: Exception) {
            Pair(null, null)
        }
    }

    private fun readPackMcmetaFromZip(zip: File): File? {
        // Return a temporary file containing the entry, or null.
        try {
            java.util.zip.ZipFile(zip).use { zf ->
                val entry = zf.getEntry("pack.mcmeta") ?: return null
                val tmp = File(zip.parentFile, ".pack_mcmeta_tmp")
                tmp.outputStream().use { fos ->
                    zf.getInputStream(entry).use { it.copyTo(fos) }
                }
                return tmp
            }
        } catch (_: Exception) {
            return null
        }
    }

    private fun readZipEntry(zip: File, entryName: String): String? {
        try {
            java.util.zip.ZipFile(zip).use { zf ->
                val entry = zf.getEntry(entryName) ?: return null
                return zf.getInputStream(entry).bufferedReader().use { it.readText() }
            }
        } catch (_: Exception) {
            return null
        }
    }

    private fun extractJsonInt(text: String, key: String): Int? {
        val pattern = Regex("\"$key\"\\s*:\\s*(\\d+)")
        return pattern.find(text)?.groupValues?.get(1)?.toIntOrNull()
    }

    private fun extractJsonString(text: String, key: String): String? {
        // Match simple JSON strings (no nested quotes for description).
        val pattern = Regex("\"$key\"\\s*:\\s*\"((?:[^\"\\\\]|\\\\.)*)\"")
        val match = pattern.find(text) ?: return null
        return match.groupValues[1].replace("\\\"", "\"").replace("\\\\", "\\")
    }

    private fun copyDir(src: File, dst: File) {
        dst.mkdirs()
        src.walkTopDown().forEach { f ->
            if (f == src) return@forEach
            val d = File(dst, f.relativeTo(src).path)
            if (f.isDirectory) {
                d.mkdirs()
            } else {
                f.copyTo(d, overwrite = true)
            }
        }
    }
}

/** Immutable UI state for [ResourcePackManagerViewModel] (task 27). */
data class ResourcePackUiState(
    val instanceId: String = "",
    val resourcePacks: List<ResourcePackData> = emptyList(),
    val resourcePacksDir: java.io.File? = null,
    val lastError: String? = null,
    val lastSuccess: String? = null,
)
