package com.rc.launcher.ui.viewmodel

import android.content.Context
import androidx.lifecycle.ViewModel
import com.rc.launcher.ui.model.ConflictResolution
import com.rc.launcher.ui.model.ControlLayoutImporter
import com.rc.launcher.ui.model.ControlLayoutDownloader
import com.rc.launcher.ui.model.ControlLayoutDownloaders
import com.rc.launcher.ui.model.ControlLayoutLibraryState
import com.rc.launcher.ui.model.ControlLayoutPackage
import com.rc.launcher.ui.model.ControlLayoutRepository
import com.rc.launcher.ui.model.ControlLayoutRepositories
import com.rc.launcher.ui.model.ControlLayoutSource
import com.rc.launcher.ui.model.ImportConflict
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

/**
 * State container for the "control layout library" screen (task 15).
 *
 * Owns three things the Compose layer should never have to think about:
 *  1. **The library catalogue** - a list of [ControlLayoutSource] entries
 *     (built-in + user-added URLs), plus per-source download progress.
 *  2. **The download pipeline** - delegates to a [ControlLayoutDownloader]
 *     (Rust core when the bridge is present, pure-JVM otherwise) so the same
 *     chunked, mirror-aware, checksum-verifying engine the rest of the
 *     launcher uses handles the actual fetch (task 2 / task 30).
 *  3. **The import pipeline** - decodes the downloaded payload (`.json` or
 *     `.zip`) via [ControlLayoutImporter], detects conflicts with the
 *     existing [ControlLayoutRepository] and persists the result.
 *
 * The model is plain (no Android-only types beyond [Context]) and exposes a
 * single immutable [ControlLayoutLibraryState] [StateFlow]; tests pass in
 * fake repository / downloader instances and assert against the state.
 */
class ControlLayoutLibraryViewModel(
    private val context: Context? = null,
    private val repository: ControlLayoutRepository = ControlLayoutRepositories.default,
    /**
     * Active downloader. Production wires the Rust-core bridge via
     * [com.rc.launcher.core.ControlLayoutDownloaderBridge.get]; tests pass a
     * fake to assert behaviour without the network.
     */
    private val downloader: ControlLayoutDownloader = ControlLayoutDownloaders.fromBridge(
        runCatching { com.rc.launcher.core.ControlLayoutDownloaderBridge.get() }.getOrNull(),
    ),
) : ViewModel() {

    private val _state = MutableStateFlow(ControlLayoutLibraryState.Empty)
    // Mirror Android's `viewModelScope` so the ViewModel compiles on a plain
    // JVM without `androidx.lifecycle:lifecycle-viewmodel-ktx`. Real Android
    // builds can replace this with `viewModelScope` from the standard
    // AndroidX dependency.
    private val scope: kotlinx.coroutines.CoroutineScope = kotlinx.coroutines.MainScope()
    val state: StateFlow<ControlLayoutLibraryState> = _state.asStateFlow()

    // ============================================================================
    // Catalogue mutation
    // ============================================================================

    /**
     * Add an externally supplied URL to the catalogue. The user can paste a
     * mirror's JSON layout URL and trigger a download immediately.
     */
    fun addExternalUrl(url: String) {
        if (url.isBlank()) return
        val cur = _state.value
        val id = "ext_${url.hashCode().toUInt().toString(16)}"
        val src = ControlLayoutSource(
            id = id,
            name = url.substringAfterLast('/').substringBefore('?').ifBlank { "\u5916\u90e8\u5e03\u5c40" },
            author = "\u5916\u90e8",
            version = "",
            description = url,
            tags = emptyList(),
            url = url,
            mirrors = emptyList(),
            format = if (url.endsWith(".zip", ignoreCase = true)) "zip" else "json",
            sha1 = null,
            size = null,
        )
        if (cur.sources.any { it.url == url }) return
        _state.value = cur.copy(sources = cur.sources + src)
    }

    fun removeSource(id: String) {
        val cur = _state.value
        _state.value = cur.copy(
            sources = cur.sources.filter { it.id != id },
            downloaded = cur.downloaded.filter { it.id != id },
            progress = cur.progress - id,
            status = cur.status - id,
        )
    }

    // ============================================================================
    // Fetch + import pipeline
    // ============================================================================

    /**
     * Download [source] into the launcher's cache dir and decode it into a
     * package. On success the source moves into `state.downloaded` and the
     * decoded package into `state.imported`. Conflicts with the existing
     * [ControlLayoutRepository] are stored on the source so the UI can prompt
     * the user.
     */
    fun fetchAndImport(source: ControlLayoutSource) {
        scope.launch {
            _state.value = _state.value.copy(
                progress = _state.value.progress + (source.id to 0f),
                status = _state.value.status + (source.id to "\u4e0b\u8f7d\u4e2d"),
                busy = true,
            )
            val dest = withContext(Dispatchers.IO) { cacheFileFor(source) }
            val fetched = downloader.download(source.toPackageRef(), dest)
            fetched.fold(
                onSuccess = { file ->
                    val pkg = withContext(Dispatchers.IO) {
                        ControlLayoutImporter.decode(file.readBytes(), dest.name)
                    }
                    val decodedNullable = pkg.pkg
                    if (decodedNullable == null) {
                        _state.value = _state.value.copy(
                            progress = _state.value.progress + (source.id to 0f),
                            status = _state.value.status + (source.id to "\u89e3\u6790\u5931\u8d25"),
                            busy = _state.value.sources.any { (it.id != source.id) && ((_state.value.progress[it.id] ?: 0f) < 1f) },
                            error = pkg.issues.firstOrNull()?.message ?: "\u4e0d\u53ef\u8bfb\u7684\u6587\u4ef6",
                        )
                        return@launch
                    }
                    val decoded: ControlLayoutPackage = decodedNullable
                    val conflicts = withContext(Dispatchers.Default) {
                        ControlLayoutImporter.detectConflicts(decoded, repository)
                    }
                    val cur = _state.value
                    _state.value = cur.copy(
                        downloaded = (cur.downloaded.filter { it.id != source.id } + source),
                        imported = (cur.imported.filter { it.id != decoded.id } + decoded),
                        progress = cur.progress + (source.id to 1f),
                        status = cur.status + (source.id to if (conflicts.isEmpty()) "\u5df2\u4e0b\u8f7d" else "\u5b58\u5728\u51b2\u7a81"),
                        busy = cur.sources.any { (cur.progress[it.id] ?: 0f) < 1f },
                    )
                },
                onFailure = { err ->
                    _state.value = _state.value.copy(
                        progress = _state.value.progress + (source.id to 0f),
                        status = _state.value.status + (source.id to "\u4e0b\u8f7d\u5931\u8d25"),
                        busy = _state.value.sources.any { (it.id != source.id) && ((_state.value.progress[it.id] ?: 0f) < 1f) },
                        error = err.message ?: err.javaClass.simpleName,
                    )
                },
            )
        }
    }

    /** Cancel an in-flight source by deleting its `.part` file. */
    fun cancelFetch(source: ControlLayoutSource) {
        scope.launch(Dispatchers.IO) {
            cacheFileFor(source).let { f ->
                File(f.parentFile, f.name + ".part").delete()
                File(f.parentFile, f.name + ".part.meta").delete()
            }
            _state.value = _state.value.copy(
                progress = _state.value.progress + (source.id to 0f),
                status = _state.value.status + (source.id to "\u5df2\u53d6\u6d88"),
                busy = _state.value.sources.any { (it.id != source.id) && ((_state.value.progress[it.id] ?: 0f) < 1f) },
            )
        }
    }

    /**
     * Apply [pkg] to the user repository, resolving any conflict with the
     * supplied [resolution]. The result is "saved" when the layout was
     * persisted; the UI then selects the new layout via the active-id in
     * settings.
     */
    fun applyPackage(
        pkg: ControlLayoutPackage,
        resolution: (ImportConflict) -> ConflictResolution,
    ) {
        scope.launch {
            val conflicts = withContext(Dispatchers.Default) {
                ControlLayoutImporter.detectConflicts(pkg, repository)
            }
            val resolved = ControlLayoutImporter.resolve(pkg, conflicts, resolution)
            withContext(Dispatchers.Default) {
                // Inline [ResolvedImport.applyTo] so we don't depend on the
                // extension-function import path staying stable across modules.
                if (!resolved.skipped) {
                    repository.save(resolved.pkg.layout.copy(name = resolved.pkg.name, editable = true))
                }
            }
        }
    }

    /**
     * Decode a local file (picked by the user via SAF / file manager) and
     * schedule an import. Returns the imported [ControlLayoutPackage] when
     * successful, or null on failure (the error is exposed on [state]).
     */
    suspend fun importLocal(bytes: ByteArray, fileName: String?): ControlLayoutPackage? {
        val result = withContext(Dispatchers.Default) {
            ControlLayoutImporter.decode(bytes, fileName)
        }
        val pkg = result.pkg ?: run {
            _state.value = _state.value.copy(error = result.message() ?: "\u5bfc\u5165\u5931\u8d25")
            return null
        }
        applyPackage(pkg) { ConflictResolution.RENAME }
        _state.value = _state.value.copy(imported = (_state.value.imported + pkg).distinctBy { it.id })
        return pkg
    }

    // ============================================================================
    // Internals
    // ============================================================================

    private fun cacheFileFor(source: ControlLayoutSource): File {
        // The Android side exposes `cacheDir` on the application context; for
        // unit tests / previews we fall back to a `java.io.tmpdir` subdirectory
        // so the same code runs on the JVM without an Android process.
        val dir = context?.cacheDir
            ?: File(System.getProperty("java.io.tmpdir"), "rc_ctrl_layouts").apply { mkdirs() }
        val safe = source.id.replace(Regex("[^a-zA-Z0-9._-]"), "_")
        return File(dir, "$safe.${source.format}")
    }

    /**
     * Convenience for the UI: snapshot the configured mirror as the fallback
     * for [ControlLayoutLibraryCatalog] URLs that miss it. Kept on the ViewModel
     * (not the model) so the unit tests don't have to mock `MirrorCatalog`.
     */
    fun withMirrorsFromCatalog(): ControlLayoutLibraryState {
        // No-op placeholder: the catalogue already ships mirror URLs.
        return _state.value
    }
}

/** Extract the failure message (or a sensible default) from an [com.rc.launcher.ui.model.ImportResult]. */
private fun com.rc.launcher.ui.model.ImportResult.message(): String? =
    issues.firstOrNull()?.message
