package com.rc.launcher.ui.viewmodel

import android.content.Context
import androidx.lifecycle.ViewModel
import com.rc.launcher.ui.model.GameInstance
import com.rc.launcher.ui.model.InstanceRepository
import com.rc.launcher.ui.model.ShaderPackData
import com.rc.launcher.ui.model.effectiveGameDir
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import java.io.File

/**
 * State container for the shader-pack management screen (task 27).
 *
 * Scans the per-instance `shaderpacks/` directory and validates each entry
 * (folder or `.zip`) by checking for a `shaders/` tree — mirroring the Rust
 * core's [`crate::mods::shader::ShaderPackManager::scan`]. Enable/disable is
 * done via the `.disabled` suffix rename convention so state is durable and
 * the two layers never disagree.
 */
class ShaderPackManagerViewModel(
    private val context: Context? = null,
    private val repository: InstanceRepository = InstanceRepository,
) : ViewModel() {

    private val scope: kotlinx.coroutines.CoroutineScope = kotlinx.coroutines.MainScope()

    private val _state = MutableStateFlow(ShaderPackUiState())
    val state: StateFlow<ShaderPackUiState> = _state.asStateFlow()

    /**
     * Load shader packs for the instance identified by [instanceId].
     */
    fun load(instanceId: String) {
        val inst = repository.getById(instanceId)
        val gameDir = inst?.let { it.effectiveGameDir(resolveBaseDir()) }
            ?: File(resolveBaseDir(), ".minecraft").absolutePath
        val spDir = File(gameDir, "shaderpacks")

        scope.launch {
            val packs = scanShaderPacks(spDir)
            _state.value = _state.value.copy(
                instanceId = instanceId,
                shaderPacks = packs,
                shaderPacksDir = spDir,
            )
        }
    }

    /** Toggle a pack's enabled state by renaming with/without `.disabled`. */
    fun setEnabled(pack: ShaderPackData, enabled: Boolean) {
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

    /** Import a shader pack from a local `.zip` or folder. */
    fun importPack(source: File) {
        scope.launch {
            val spDir = _state.value.shaderPacksDir ?: return@launch
            try {
                val dest = File(spDir, source.name)
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

    /** Delete a shader pack. */
    fun delete(pack: ShaderPackData, confirm: Boolean) {
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

    /** Download a shader pack from a mirror / online source URL. */
    fun downloadFromUrl(url: String) {
        // In production this goes through download::Manager (task 2 / task 30)
        // with mirror + range + checksum support.
        scope.launch {
            _state.value = _state.value.copy(
                lastError = "Download from URL not yet wired to the Rust core",
                lastSuccess = null,
            )
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

    private fun scanShaderPacks(dir: File): List<ShaderPackData> {
        if (!dir.exists() || !dir.isDirectory) return emptyList()
        val out = ArrayList<ShaderPackData>()
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
            val valid = hasShaderTree(child)

            out.add(ShaderPackData(
                name = baseName,
                path = child,
                enabled = enabled,
                valid = valid,
            ))
        }
        return out.sortedBy { it.name.lowercase() }
    }

    private fun hasShaderTree(path: File): Boolean {
        if (path.isDirectory) {
            val shadersDir = File(path, "shaders")
            if (shadersDir.isDirectory) return true
            return File(path, "shaders.properties").isFile
        }
        // zip: check for shaders/ entry or shaders.properties
        return readZipAny(path, listOf("shaders/", "shaders.properties"))
    }

    private fun readZipAny(path: File, needles: List<String>): Boolean {
        try {
            java.util.zip.ZipFile(path).use { zf ->
                zf.entries().asSequence().forEach { entry ->
                    val lower = entry.name.lowercase()
                    for (n in needles) {
                        if (lower == n || lower.startsWith(n)) return true
                    }
                }
            }
            return false
        } catch (_: Exception) {
            return false
        }
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

/** Immutable UI state for [ShaderPackManagerViewModel] (task 27). */
data class ShaderPackUiState(
    val instanceId: String = "",
    val shaderPacks: List<ShaderPackData> = emptyList(),
    val shaderPacksDir: java.io.File? = null,
    val lastError: String? = null,
    val lastSuccess: String? = null,
)
