package com.rc.launcher.ui.viewmodel

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.rc.launcher.core.RcEvent
import com.rc.launcher.core.RcEventBus
import com.rc.launcher.core.RcEventKind
import com.rc.launcher.core.RcEventListener
import com.rc.launcher.core.RcDownloadJobSpec
import com.rc.launcher.core.startDownload
import com.rc.launcher.core.cancelDownload
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * Active download state shown on the downloads screen (task 30).
 *
 * Each row maps to one `scope` on the Rust core's event bus. Progress,
 * speed (`speed_bps`) and `status` (`"running"` / `"paused"` / `"completed"` /
 * `"failed"`) are updated as `progress` events arrive.
 */
data class DownloadEntry(
    val scope: String,
    val label: String,
    val downloaded: Long,
    val total: Long?,
    val speedBps: Long,
    val status: String,
    val errorMessage: String? = null,
) {
    val fraction: Float get() =
        total?.takeIf { it > 0 }?.let { downloaded.toFloat() / it } ?: 0f

    val isFinished: Boolean get() = status == "completed" || status == "failed"

    val bytesRemaining: Long? get() = total?.minus(downloaded)
}

/**
 * Downloads-screen state container (task 30).
 *
 * Subscribes to the global Rust event bus, correlating `progress` events by
 * `scope` so the UI gets live speed / status updates. Pause / cancel is wired
 * through `cancelAsync` — the Rust core preserves the `.part` / `.part.meta`
 * side-cars so a later resume picks up where it left off (auto-recovery after
 * disconnect).
 */
class DownloadsViewModel : ViewModel() {

    private val _entries = MutableStateFlow<List<DownloadEntry>>(emptyList())
    val entries: StateFlow<List<DownloadEntry>> = _entries.asStateFlow()

    private var listener: RcEventListener? = null

    init {
        subscribeToBus()
    }

    /**
     * Start a download job with the given spec. The job's `scope` is used to
     * correlate progress events on the bus.
     */
    fun startDownload(spec: RcDownloadJobSpec) {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                val handle = startDownload(spec)
                if (!handle.ok) {
                    _entries.update { list ->
                        list + DownloadEntry(
                            scope = spec.scope,
                            label = spec.label,
                            downloaded = 0,
                            total = null,
                            speedBps = 0,
                            status = "failed",
                            errorMessage = "Failed to start download job",
                        )
                    }
                } else {
                    _entries.update { list ->
                        list + DownloadEntry(
                            scope = handle.scope,
                            label = spec.label,
                            downloaded = 0,
                            total = null,
                            speedBps = 0,
                            status = "running",
                        )
                    }
                }
            } catch (t: Throwable) {
                _entries.update { list ->
                    list + DownloadEntry(
                        scope = spec.scope,
                        label = spec.label,
                        downloaded = 0,
                        total = null,
                        speedBps = 0,
                        status = "failed",
                        errorMessage = t.message ?: "unknown error",
                    )
                }
            }
        }
    }

    /**
     * Pause / cancel a running download by scope (task 30). The Rust core
     * preserves the `.part` / `.part.meta` side-cars so a later call to
     * [startDownload] resumes from the last checkpoint.
     */
    fun pauseDownload(scope: String) {
        viewModelScope.launch(Dispatchers.IO) {
            cancelDownload(scope)
        }
    }

    private fun subscribeToBus() {
        listener?.let { RcEventBus.removeListener(it) }
        if (!RcEventBus.isConnected) RcEventBus.connect()
        val l = RcEventListener { event ->
            when (event.kind) {
                RcEventKind.PROGRESS -> handleProgress(event)
                RcEventKind.LIFECYCLE -> handleLifecycle(event)
                RcEventKind.ERROR -> handleError(event)
                else -> {}
            }
        }
        listener = l
        RcEventBus.addListener(l)
    }

    private fun handleProgress(event: RcEvent) {
        val speed = event.progressSpeedBps
        val status = event.progressStatus ?: "running"
        val downloaded = event.progressDownloaded
        val total = event.progressTotal
        _entries.update { list ->
            val idx = list.indexOfFirst { it.scope == event.scope }
            if (idx >= 0) {
                val existing = list[idx]
                val updated = existing.copy(
                    downloaded = downloaded,
                    total = total,
                    speedBps = speed,
                    status = status,
                    errorMessage = null,
                )
                list.toMutableList().apply { set(idx, updated) }
            } else {
                list + DownloadEntry(
                    scope = event.scope,
                    label = event.message,
                    downloaded = downloaded,
                    total = total,
                    speedBps = speed,
                    status = status,
                )
            }
        }
    }

    private fun handleLifecycle(event: RcEvent) {
        val phase = event.data?.optString("phase") ?: ""
        when (phase) {
            "completed", "finished" -> {
                _entries.update { list ->
                    list.map { e ->
                        if (e.scope == event.scope && !e.isFinished) {
                            e.copy(status = "completed")
                        } else e
                    }
                }
            }
            "cancelled" -> {
                _entries.update { list ->
                    list.map { e ->
                        if (e.scope == event.scope) e.copy(status = "paused")
                        else e
                    }
                }
            }
        }
    }

    private fun handleError(event: RcEvent) {
        _entries.update { list ->
            list.map { e ->
                if (e.scope == event.scope) {
                    e.copy(status = "failed", errorMessage = event.message)
                } else e
            }
        }
    }

    override fun onCleared() {
        super.onCleared()
        listener?.let { RcEventBus.removeListener(it) }
        listener = null
    }
}
