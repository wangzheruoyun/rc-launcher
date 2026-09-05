package com.rc.launcher.core

import com.rc.launcher.ui.model.ControlLayoutDownloader
import com.rc.launcher.ui.model.ControlLayoutPackage
import com.rc.launcher.ui.model.DownloadFailure
import com.rc.launcher.ui.model.JvmControlLayoutDownloader
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * Adapter that forwards control-layout fetches to the Rust core's DownloadManager
 * (task 15). The model layer defines [ControlLayoutDownloader] as a pure JVM
 * interface; this wrapper is the Android-side implementation that turns it into
 * the spec JSON [RustBridge.downloadAsync] expects (chunked, resumable, mirror
 * fallback + checksum verification, task 2).
 *
 * The wrapper is intentionally minimal: it doesn't track progress (the UI
 * subscribes to [RcEventBus] for that) and doesn't do SHA-1 verification
 * itself (the Rust side already does it; on success the destination file's
 * length matches, on failure the manager emits an error event and the file
 * is removed).
 *
 * Lives in the `app` module (not the `core` module) because it depends on the
 * model-package interface that lives in `app`.
 */
class RustCoreLayoutDownloader(
    private val bridge: RustBridge,
) : ControlLayoutDownloader {

    override suspend fun download(pkg: ControlLayoutPackage, dest: File): Result<File> {
        val urls = (listOfNotNull(pkg.downloadUrl) + pkg.mirrors).distinct()
        return downloadUrl(urls.first(), dest, pkg.sha1, urls.drop(1))
    }

    override suspend fun downloadUrl(
        url: String,
        dest: File,
        sha1: String?,
        mirrors: List<String>,
    ): Result<File> = withContext(Dispatchers.IO) {
        val mirrorsJson = JSONArray()
        for (m in mirrors) mirrorsJson.put(m)
        val task = JSONObject().apply {
            put("url", url)
            put("dest", dest.absolutePath)
            if (dest.exists()) put("size", dest.length()) else put("size", -1L)
            if (sha1 != null) put("sha1", sha1)
            put("mirrors", mirrorsJson)
        }
        val spec = JSONObject().apply {
            put("scope", "ctrl_layout_${dest.nameWithoutExtension}")
            put("label", "control layout ${dest.name}")
            put("concurrency", 4)
            put("tasks", JSONArray().apply { put(task) })
        }
        val handle = try {
            bridge.runDownloadAsync(spec.toString())
        } catch (e: Throwable) {
            return@withContext Result.failure(DownloadFailure(
                reason = "Rust download bridge unavailable: ${e.message}",
                lastUrl = url,
                attempts = 1,
            ))
        }
        if (!handle.ok) {
            return@withContext Result.failure(DownloadFailure(
                reason = "Rust download manager rejected the spec",
                lastUrl = url,
                attempts = 1,
            ))
        }
        // The native job is fire-and-forget; progress / completion events flow
        // through [RcEventBus] so the UI can react. We block here (with a sane
        // timeout) to keep the `suspend fun download(...): Result<File>`
        // contract ergonomic for the ViewModel layer.
        val deadline = System.currentTimeMillis() + MAX_WAIT_MS
        while (System.currentTimeMillis() < deadline) {
            if (dest.exists() && dest.length() > 0) {
                return@withContext Result.success(dest)
            }
            try {
                Thread.sleep(POLL_INTERVAL_MS)
            } catch (_: InterruptedException) {
                return@withContext Result.failure(DownloadFailure("interrupted", url, 1))
            }
        }
        Result.failure(DownloadFailure("download did not complete in time", url, 1))
    }

    companion object {
        private const val MAX_WAIT_MS = 5L * 60_000L
        private const val POLL_INTERVAL_MS = 500L
    }
}

/**
 * Factory used by [com.rc.launcher.ui.model.ControlLayoutDownloaders] so the
 * model package stays decoupled from the Android-specific downloader wiring.
 *
 * Returns a [RustCoreLayoutDownloader] when the native library is reachable;
 * otherwise returns [JvmControlLayoutDownloader] so the UI keeps working
 * (Compose previews, unit tests, on devices where the Rust core is not bundled).
 */
object ControlLayoutDownloaderBridge {
    fun get(): ControlLayoutDownloader = try {
        // `RustBridge` may not be loadable when the native library is missing;
        // any exception (UnsatisfiedLinkError, ClassNotFound, ...) drops us to
        // the JVM fallback.
        RustCoreLayoutDownloader(RustBridge)
    } catch (_: Throwable) {
        JvmControlLayoutDownloader()
    }
}
