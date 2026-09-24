package com.rc.launcher.ui.viewmodel

import android.app.Application
import android.util.Base64
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.rc.launcher.core.RcEventBus
import com.rc.launcher.core.RcEventKind
import com.rc.launcher.core.RcEventListener
import com.rc.launcher.core.RcEvent
import com.rc.launcher.core.RustBridge
import com.rc.launcher.ui.model.GameDirectoryType
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/**
 * The modpack-import flow state container (task 17).
 *
 * Drives the Rust-side import pipeline through [RustBridge]:
 *   * `importFromUrl`     -> `RustBridge.modpackImport` over a remote manifest
 *   * `importFromText`    -> `RustBridge.modpackInspect` then `modpackImport`
 *   * `importLocalArchive` -> `modpackInspectArchive` then `modpackImport`
 *
 * Subscribes to the event bus so progress / per-file outcomes are surfaced
 * back into the UI.
 *
 * Adaptive layout is delegated to the Compose tree (`rcWindowInfo()`); this
 * ViewModel stays a pure, framework-friendly state holder.
 */
class ModpackImportViewModel(application: Application) : AndroidViewModel(application) {

    private val _state = MutableStateFlow(ModpackImportState())
    val state: StateFlow<ModpackImportState> = _state.asStateFlow()

    private var subscriptionJob: Job? = null
    private var runningScope: String? = null

    fun setUrlInput(value: String) {
        _state.update { it.copy(urlInput = value) }
    }

    fun setManifestTextInput(value: String) {
        _state.update { it.copy(manifestTextInput = value) }
    }

    fun setInstanceId(value: String) {
        _state.update { it.copy(instanceId = value.replace("/", "_").replace("\\", "_")) }
    }

    fun setAllowOverwrite(value: Boolean) {
        _state.update { it.copy(allowOverwrite = value) }
    }

    fun setGameDirectoryType(value: GameDirectoryType) {
        _state.update { it.copy(gameDirectoryType = value) }
    }

    fun consumeError() {
        _state.update { it.copy(errorMessage = null) }
    }

    /**
     * Inspect a manifest text. The reply (a `Manifest` JSON envelope) is
     * cached on the state so the confirmation card can render before we
     * actually start downloading.
     */
    fun importFromText() {
        val text = _state.value.manifestTextInput
        viewModelScope.launch(Dispatchers.IO) {
            try {
                val reply = RustBridge.modpackInspectManifest(text, origin = "pasted")
                if (reply.has("error") && !reply.has("spec")) {
                    _state.update {
                        it.copy(errorMessage = "\u89e3\u6790\u5931\u8d25: ${reply.optString("error")}")
                    }
                    return@launch
                }
                _state.update { it.copy(parsedManifest = reply) }
            } catch (t: Throwable) {
                _state.update { it.copy(errorMessage = "\u89e3\u6792\u9519\u8bef: ${t.message ?: "unknown"}") }
            }
        }
    }

    /**
     * Inspect + import a remote URL.
     */
    fun importFromUrl() {
        val url = _state.value.urlInput.trim()
        if (url.isEmpty()) return
        viewModelScope.launch(Dispatchers.IO) {
            try {
                // Try the inspect path first to give the user a confirmation
                // card. Fall back to a one-shot import when the URL is a
                // raw `.zip` / `.mrpack` (not a bare manifest text).
                val manifestText = fetchText(url)
                if (manifestText != null) {
                    val reply = RustBridge.modpackInspectManifest(manifestText, origin = url)
                    if (!reply.has("error")) {
                        _state.update { it.copy(parsedManifest = reply) }
                        return@launch
                    }
                }
                // Fallback: let the native side handle the URL via the
                // importer's `import_url` path. We expose it through a thin
                // Kotlin helper that mimics the same JSON envelope.
                startImportFromArchive(uri = url)
            } catch (t: Throwable) {
                _state.update { it.copy(errorMessage = t.message ?: "unknown") }
            }
        }
    }

    /**
     * Import a local archive (already in memory).
     */
    fun importLocalArchive(bytes: ByteArray, origin: String) {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                val b64 = Base64.encodeToString(bytes, Base64.NO_WRAP)
                val reply = RustBridge.modpackInspectArchive(b64, origin = origin)
                if (reply.has("error") && !reply.has("spec")) {
                    _state.update {
                        it.copy(errorMessage = "\u89e3\u6790\u5931\u8d25: ${reply.optString("error")}")
                    }
                    return@launch
                }
                _state.update {
                    it.copy(
                        parsedManifest = reply,
                        pendingArchiveBase64 = b64,
                        pendingArchiveOrigin = origin,
                    )
                }
            } catch (t: Throwable) {
                _state.update { it.copy(errorMessage = t.message ?: "unknown") }
            }
        }
    }

    /** Start the actual import (after the user has confirmed). */
    fun startImport() {
        val s = _state.value
        val manifest = s.parsedManifest ?: return
        val instanceId = s.instanceId.takeIf { it.isNotBlank() } ?: run {
            _state.update { it.copy(errorMessage = "\u5b9e\u4f8b\u76ee\u5f55\u540d\u4e0d\u80fd\u4e3a\u7a7a") }
            return
        }
        viewModelScope.launch(Dispatchers.IO) {
            try {
                val instancesRoot = instancesRoot()
                val handle = RustBridge.runModpackImport(
                    instancesRoot = instancesRoot,
                    manifestJson = manifest,
                    instanceId = instanceId,
                    allowOverwrite = s.allowOverwrite,
                )
                if (!handle.ok) {
                    _state.update { it.copy(errorMessage = "\u542f\u52a8\u5bfc\u5165\u4efb\u52a1\u5931\u8d25") }
                    return@launch
                }
                runningScope = handle.scope
                subscribeToBus(handle.scope)
                _state.update {
                    it.copy(
                        busy = true,
                        progressLabel = "\u51c6\u5907\u4e2d...",
                        progressFraction = 0f,
                        report = null,
                    )
                }
            } catch (t: Throwable) {
                _state.update { it.copy(errorMessage = t.message ?: "unknown") }
            }
        }
    }

    private fun startImportFromArchive(uri: String) {
        // Best-effort: try to fetch the bytes through OkHttp-like path.
        // On Android we let the JVM URL.openStream do the heavy lifting.
        viewModelScope.launch(Dispatchers.IO) {
            try {
                val bytes = withContext(Dispatchers.IO) {
                    java.net.URL(uri).openStream().use { it.readBytes() }
                }
                importLocalArchive(bytes, origin = uri)
            } catch (t: Throwable) {
                _state.update { it.copy(errorMessage = t.message ?: "unknown") }
            }
        }
    }

    private suspend fun fetchText(url: String): String? = withContext(Dispatchers.IO) {
        runCatching {
            java.net.URL(url).openStream().bufferedReader().use { it.readText() }
        }.getOrNull()
    }

    private var listener: RcEventListener? = null

    private fun subscribeToBus(scope: String) {
        listener?.let { RcEventBus.removeListener(it) }
        if (!RcEventBus.isConnected) RcEventBus.connect()
        val l = RcEventListener { event ->
            if (event.scope != scope) return@RcEventListener
            when (event.kind) {
                RcEventKind.LIFECYCLE -> handleLifecycle(event)
                RcEventKind.ERROR -> _state.update {
                    it.copy(errorMessage = event.message)
                }
                else -> {}
            }
        }
        listener = l
        RcEventBus.addListener(l)
    }

    private fun handleLifecycle(event: RcEvent) {
        val phase = event.data?.optString("phase") ?: ""
        when (phase) {
            "started" -> _state.update { it.copy(progressLabel = "\u5f00\u59cb\u4e0b\u8f7d...") }
            "succeeded" -> finishWithReport(event, ok = true)
            "completed_with_errors" -> finishWithReport(event, ok = false)
            "finished" -> _state.update { it.copy(busy = false) }
        }
    }

    private fun finishWithReport(event: RcEvent, ok: Boolean) {
        // The Rust side puts the report payload under data.result for the
        // lifecycle_with_result helper. Fallback to data itself.
        val data = event.data?.optJSONObject("result") ?: event.data
        _state.update {
            it.copy(
                busy = false,
                report = data,
                progressLabel = if (ok) "\u5bfc\u5165\u5b8c\u6210" else "\u5bfc\u5165\u5b8c\u6210\uff08\u6709\u95ee\u9898\uff09",
            )
        }
    }

    private fun instancesRoot(): String {
        // Android scoped storage: `/data/data/<pkg>/files/instances`. The
        // launch engine (task 7) reads from the same root. We reach
        //  through reflection so the ViewModel compiles without a
        // hard dependency on  (and stays unit-test
        // friendly).
        val ctx: Application = getApplication()
        val dir = ctx::class.java.getMethod("getFilesDir").invoke(ctx) as java.io.File
        return java.io.File(dir, "instances").absolutePath
    }

    protected fun cleanup() {
        subscriptionJob?.cancel()
        listener?.let { RcEventBus.removeListener(it) }
        listener = null
    }
}

/** Modpack import screen state. */
data class ModpackImportState(
    val urlInput: String = "",
    val manifestTextInput: String = "",
    val instanceId: String = "",
    val allowOverwrite: Boolean = false,
    val gameDirectoryType: GameDirectoryType = GameDirectoryType.ISOLATED,
    val parsedManifest: JSONObject? = null,
    val pendingArchiveBase64: String? = null,
    val pendingArchiveOrigin: String? = null,
    val busy: Boolean = false,
    val progressLabel: String = "",
    val progressFraction: Float = 0f,
    val report: JSONObject? = null,
    val errorMessage: String? = null,
)
