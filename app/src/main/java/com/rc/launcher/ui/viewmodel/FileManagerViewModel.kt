package com.rc.launcher.ui.viewmodel

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.rc.launcher.core.RustBridge
import com.rc.launcher.core.RustBridge.FileManagerResult
import com.rc.launcher.ui.model.GameInstance
import com.rc.launcher.ui.model.InstanceRepository
import com.rc.launcher.ui.model.effectiveGameDir
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/**
 * State container for the in-app small file manager (task 19).
 *
 * The screen is deliberately *small*: it can only see / touch the
 * directories the caller hands it through [start]. The "allowed roots" list
 * is a defence-in-depth contract — the Rust core ([fs_ops::resolve_under_
 * roots]) refuses to read or write anything that does not canonicalise under
 * one of them, so a malicious UI cannot trick the bridge into escaping the
 * scope.
 *
 * Destructive operations go through a two-step flow:
 *   1. [requestDelete] / [requestMove] without `confirm` return a preview
 *      ([FileManagerUiState.pendingDelete] / [.pendingMove]) that the UI
 *      renders in a confirmation dialog.
 *   2. [confirmPendingDelete] / [confirmPendingMove] re-issue the request
 *      with `confirm = true` and only then does the core actually mutate
 *      the filesystem.
 */
class FileManagerViewModel : ViewModel() {

    private val _state = MutableStateFlow(FileManagerUiState())
    val state: StateFlow<FileManagerUiState> = _state.asStateFlow()

    /**
     * Open the manager on the directory that contains a Minecraft instance
     * (its effective game dir) so the user can browse `saves/`, `mods/`,
     * `resourcepacks/`, `shaderpacks/`, and the per-instance config.
     */
    fun startForInstance(
        instance: GameInstance,
        baseDir: String,
        initialSubdir: String? = null,
    ) {
        val root = instance.effectiveGameDir(baseDir)
        val start = if (initialSubdir.isNullOrBlank()) root else "$root/$initialSubdir"
        _state.value = FileManagerUiState(
            roots = listOf(root),
            currentPath = start,
            title = "${instance.name} · 文件",
            initialSubdir = initialSubdir,
        )
        refresh()
    }

    /**
     * Open the manager on an arbitrary allowed root (e.g. the game root or
     * a per-instance directory). Multiple roots are accepted; the user can
     * navigate into any of them and back out.
     */
    fun start(roots: List<String>, startPath: String, title: String) {
        require(roots.isNotEmpty()) { "file manager needs at least one allowed root" }
        _state.value = FileManagerUiState(
            roots = roots,
            currentPath = startPath,
            title = title,
            initialSubdir = null,
        )
        refresh()
    }

    /** Re-list the current directory (e.g. after an external change). */
    fun refresh() {
        val s = _state.value
        if (s.currentPath.isBlank()) return
        _state.update { it.copy(busy = true, errorMessage = null) }
        viewModelScope.launch {
            val result = withContext(Dispatchers.IO) {
                RustBridge.fsListDirTyped(s.currentPath, s.roots)
            }
            when (result) {
                is FileManagerResult.Success -> {
                    val listing = parseListing(result.json)
                    _state.update {
                        it.copy(
                            busy = false,
                            currentPath = listing.path,
                            parent = listing.parent,
                            entries = listing.entries,
                            errorMessage = null,
                        )
                    }
                }
                is FileManagerResult.Error -> {
                    _state.update { it.copy(busy = false, errorMessage = result.message) }
                }
                is FileManagerResult.NeedsConfirmation -> {
                    // Should not happen for list; surface the raw envelope as
                    // a banner so a future reply-shape drift is obvious.
                    _state.update {
                        it.copy(
                            busy = false,
                            errorMessage = "意外的确认请求: ${result.json}",
                        )
                    }
                }
            }
        }
    }

    /** Navigate into one of the listed entries (must be a directory). */
    fun open(entry: FsEntry) {
        if (entry.kind != "dir") return
        _state.update { it.copy(currentPath = entry.path) }
        refresh()
    }

    /**
     * Step one level up. Stops at the first allowed root so the user can
     * never navigate above the configured sandbox.
     */
    fun goUp() {
        val s = _state.value
        val parent = s.parent.ifBlank { return }
        if (s.roots.any { canonicalRoot(it) == parent }) return
        _state.update { it.copy(currentPath = parent) }
        refresh()
    }

    /**
     * Open the "new folder" dialog. The actual create goes through the
     * core on confirm; see [createDirectory].
     */
    fun requestCreateDialog() {
        _state.update { it.copy(dialog = FileManagerDialogState.Create) }
    }

    /** Open the "rename" dialog for [entry]. */
    fun requestRenameDialog(entry: FsEntry) {
        _state.update { it.copy(dialog = FileManagerDialogState.Rename(entry.path)) }
    }

    /** Open the "copy to..." dialog for [entry]. */
    fun requestCopyDialog(entry: FsEntry) {
        _state.update { it.copy(dialog = FileManagerDialogState.Copy(entry.path)) }
    }

    /** Dismiss any open name-input dialog. */
    fun dismissDialog() {
        _state.update { it.copy(dialog = null) }
    }

    /**
     * Step one: ask the core to delete `paths`. Without `confirm: true` the
     * core returns a preview; the UI is expected to render it and, if the
     * user accepts, call [confirmPendingDelete].
     */
    fun requestDelete(paths: List<String>) {
        if (paths.isEmpty()) return
        _state.update { it.copy(busy = true, errorMessage = null) }
        viewModelScope.launch {
            val r = withContext(Dispatchers.IO) {
                RustBridge.fsDeleteTyped(paths, _state.value.roots, confirm = false)
            }
            when (r) {
                is FileManagerResult.NeedsConfirmation -> {
                    _state.update {
                        it.copy(
                            busy = false,
                            pendingDelete = PendingDelete(paths, r.json),
                        )
                    }
                }
                is FileManagerResult.Error ->
                    _state.update { it.copy(busy = false, errorMessage = r.message) }
                is FileManagerResult.Success ->
                    _state.update { it.copy(busy = false) }
            }
        }
    }

    /** Step two: actually delete the paths queued by [requestDelete]. */
    fun confirmPendingDelete() {
        val pending = _state.value.pendingDelete ?: return
        _state.update { it.copy(busy = true, errorMessage = null) }
        viewModelScope.launch {
            val r = withContext(Dispatchers.IO) {
                RustBridge.fsDeleteTyped(pending.paths, _state.value.roots, confirm = true)
            }
            when (r) {
                is FileManagerResult.Success -> {
                    _state.update { it.copy(busy = false, pendingDelete = null) }
                    refresh()
                }
                is FileManagerResult.Error ->
                    _state.update { it.copy(busy = false, errorMessage = r.message, pendingDelete = null) }
                is FileManagerResult.NeedsConfirmation ->
                    _state.update {
                        it.copy(
                            busy = false,
                            errorMessage = "删除仍需确认: ${r.json}",
                            pendingDelete = null,
                        )
                    }
            }
        }
    }

    /** Cancel a queued delete preview. */
    fun cancelPendingDelete() {
        _state.update { it.copy(pendingDelete = null) }
    }

    /**
     * Create a new directory under the current path. `name` is a single
     * leaf; the core rejects `..` / absolute / drive-prefix components.
     */
    fun createDirectory(name: String) {
        if (name.isBlank()) return
        _state.update { it.copy(busy = true, errorMessage = null) }
        viewModelScope.launch {
            val r = withContext(Dispatchers.IO) {
                RustBridge.fsMkdirTyped(_state.value.currentPath, name, _state.value.roots)
            }
            when (r) {
                is FileManagerResult.Success -> {
                    _state.update { it.copy(busy = false) }
                    refresh()
                }
                is FileManagerResult.Error ->
                    _state.update { it.copy(busy = false, errorMessage = r.message) }
                is FileManagerResult.NeedsConfirmation ->
                    _state.update { it.copy(busy = false, errorMessage = "意外的确认请求") }
            }
        }
    }

    /**
     * Rename `path` to `newName`. Returns silently on success; on error the
     * message is surfaced through [FileManagerUiState.errorMessage].
     */
    fun rename(path: String, newName: String) {
        if (newName.isBlank()) return
        _state.update { it.copy(busy = true, errorMessage = null) }
        viewModelScope.launch {
            val r = withContext(Dispatchers.IO) {
                RustBridge.fsRenameTyped(path, newName, _state.value.roots)
            }
            when (r) {
                is FileManagerResult.Success -> {
                    _state.update { it.copy(busy = false) }
                    refresh()
                }
                is FileManagerResult.Error ->
                    _state.update { it.copy(busy = false, errorMessage = r.message) }
                is FileManagerResult.NeedsConfirmation ->
                    _state.update { it.copy(busy = false, errorMessage = "意外的确认请求") }
            }
        }
    }

    /**
     * Step one: ask the core to move `source` to `destination`. Without
     * `confirm: true` the core returns a preview when the destination
     * already exists; the UI is expected to render it and, if the user
     * accepts, call [confirmPendingMove].
     */
    fun requestMove(source: String, destination: String) {
        _state.update { it.copy(busy = true, errorMessage = null) }
        viewModelScope.launch {
            val r = withContext(Dispatchers.IO) {
                RustBridge.fsMoveTyped(source, destination, _state.value.roots, confirm = false)
            }
            when (r) {
                is FileManagerResult.NeedsConfirmation -> {
                    _state.update {
                        it.copy(
                            busy = false,
                            pendingMove = PendingMove(source, destination, r.json),
                        )
                    }
                }
                is FileManagerResult.Success -> {
                    _state.update { it.copy(busy = false) }
                    refresh()
                }
                is FileManagerResult.Error ->
                    _state.update { it.copy(busy = false, errorMessage = r.message) }
            }
        }
    }

    /** Step two: actually move. */
    fun confirmPendingMove() {
        val pending = _state.value.pendingMove ?: return
        _state.update { it.copy(busy = true, errorMessage = null) }
        viewModelScope.launch {
            val r = withContext(Dispatchers.IO) {
                RustBridge.fsMoveTyped(
                    pending.source,
                    pending.destination,
                    _state.value.roots,
                    confirm = true,
                )
            }
            when (r) {
                is FileManagerResult.Success -> {
                    _state.update { it.copy(busy = false, pendingMove = null) }
                    refresh()
                }
                is FileManagerResult.Error ->
                    _state.update { it.copy(busy = false, errorMessage = r.message, pendingMove = null) }
                is FileManagerResult.NeedsConfirmation ->
                    _state.update {
                        it.copy(
                            busy = false,
                            errorMessage = "移动仍需确认: ${r.json}",
                            pendingMove = null,
                        )
                    }
            }
        }
    }

    fun cancelPendingMove() {
        _state.update { it.copy(pendingMove = null) }
    }

    /**
     * Copy a single file (the typical "duplicate this save / mod" use case).
     * `destination` is a full path; the core refuses to overwrite an
     * existing entry.
     */
    fun copyFile(source: String, destination: String) {
        _state.update { it.copy(busy = true, errorMessage = null) }
        viewModelScope.launch {
            val r = withContext(Dispatchers.IO) {
                RustBridge.fsCopyTyped(source, destination, _state.value.roots)
            }
            when (r) {
                is FileManagerResult.Success -> {
                    _state.update { it.copy(busy = false) }
                    refresh()
                }
                is FileManagerResult.Error ->
                    _state.update { it.copy(busy = false, errorMessage = r.message) }
                is FileManagerResult.NeedsConfirmation ->
                    _state.update { it.copy(busy = false, errorMessage = "意外的确认请求") }
            }
        }
    }

    /**
     * Extract a `.zip` archive at `archive` into `destination`. Used by the
     * mod / resource-pack / shader-pack import buttons so the user can pick
     * an archive from the file manager and unpack it into the right place.
     */
    fun extractZip(archive: String, destination: String) {
        _state.update { it.copy(busy = true, errorMessage = null) }
        viewModelScope.launch {
            val r = withContext(Dispatchers.IO) {
                RustBridge.fsExtractZipTyped(archive, destination, _state.value.roots)
            }
            when (r) {
                is FileManagerResult.Success -> {
                    _state.update { it.copy(busy = false) }
                    refresh()
                }
                is FileManagerResult.Error ->
                    _state.update { it.copy(busy = false, errorMessage = r.message) }
                is FileManagerResult.NeedsConfirmation ->
                    _state.update { it.copy(busy = false, errorMessage = "意外的确认请求") }
            }
        }
    }

    /**
     * Write `bytes` (typically decoded from a `content://` URI) to
     * `destination`. The destination is the full file path; the parent
     * directory is created on demand.
     */
    fun importBytes(destination: String, bytes: ByteArray) {
        _state.update { it.copy(busy = true, errorMessage = null) }
        viewModelScope.launch {
            val r = withContext(Dispatchers.IO) {
                RustBridge.fsImportBytesTyped(destination, bytes, _state.value.roots)
            }
            when (r) {
                is FileManagerResult.Success -> {
                    _state.update { it.copy(busy = false) }
                    refresh()
                }
                is FileManagerResult.Error ->
                    _state.update { it.copy(busy = false, errorMessage = r.message) }
                is FileManagerResult.NeedsConfirmation ->
                    _state.update { it.copy(busy = false, errorMessage = "意外的确认请求") }
            }
        }
    }

    fun consumeError() {
        _state.update { it.copy(errorMessage = null) }
    }

    private fun canonicalRoot(path: String): String {
        val f = java.io.File(path)
        return try {
            f.canonicalPath
        } catch (_: Throwable) {
            f.absolutePath
        }
    }

    private fun parseListing(json: JSONObject): ParsedListing {
        val entriesJson = json.optJSONArray("entries") ?: JSONArray()
        val out = ArrayList<FsEntry>(entriesJson.length())
        for (i in 0 until entriesJson.length()) {
            val e = entriesJson.getJSONObject(i)
            out.add(
                FsEntry(
                    name = e.optString("name"),
                    kind = e.optString("kind"),
                    size = e.optLong("size", 0L),
                    mtimeMs = e.optLong("mtime_ms", 0L),
                    path = e.optString("path"),
                    hidden = e.optBoolean("hidden", false),
                ),
            )
        }
        return ParsedListing(
            path = json.optString("path"),
            parent = json.optString("parent"),
            entries = out,
        )
    }

    private data class ParsedListing(
        val path: String,
        val parent: String,
        val entries: List<FsEntry>,
    )
}

/** One row in the file manager list. Mirrors `fs_ops::FsEntry` on the Rust side. */
data class FsEntry(
    val name: String,
    val kind: String,
    val size: Long,
    val mtimeMs: Long,
    val path: String,
    val hidden: Boolean,
) {
    val isDirectory: Boolean get() = kind == "dir"
    val isFile: Boolean get() = kind == "file"
    val isSymlink: Boolean get() = kind == "link"

    /**
     * UI-side selection state. Not part of the Rust payload; the file
     * manager holds one mutable list of [FsEntry]s (the most-recent
     * listing) and the Compose tree mutates this flag for the multi-
     * select affordance (delete / copy). Toggling it does **not** reach
     * the core; it only changes the highlighted row in the UI.
     */
    var isSelected: Boolean = false
}

/** Snapshot of one file-manager screen. */
data class FileManagerUiState(
    val roots: List<String> = emptyList(),
    val currentPath: String = "",
    val parent: String = "",
    val entries: List<FsEntry> = emptyList(),
    val title: String = "",
    val initialSubdir: String? = null,
    val busy: Boolean = false,
    val errorMessage: String? = null,
    val pendingDelete: PendingDelete? = null,
    val pendingMove: PendingMove? = null,
    val dialog: FileManagerDialogState? = null,
)

/** Modal dialog driven by the file manager (create / rename / copy). */
sealed class FileManagerDialogState {
    /** "New folder" dialog. */
    data object Create : FileManagerDialogState()

    /** "Rename" dialog. */
    data class Rename(val path: String) : FileManagerDialogState()

    /** "Copy to..." dialog. */
    data class Copy(val source: String) : FileManagerDialogState()
}

/** A queued delete: the targets and the core's preview JSON. */
data class PendingDelete(
    val paths: List<String>,
    val preview: JSONObject,
) {
    val totalBytes: Long get() = preview.optLong("total_bytes", 0L)
    val hasDirectories: Boolean get() = preview.optBoolean("has_directories", false)
}

/** A queued move: source / destination and the core's preview JSON. */
data class PendingMove(
    val source: String,
    val destination: String,
    val preview: JSONObject,
) {
    val totalBytes: Long get() = preview.optLong("total_bytes", 0L)
}
