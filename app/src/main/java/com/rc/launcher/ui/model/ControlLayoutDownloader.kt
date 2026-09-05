package com.rc.launcher.ui.model

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException
import java.io.RandomAccessFile
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * Resumable, mirror-aware HTTP download for [ControlLayoutPackage] payloads
 * (task 15).
 *
 * The launcher has two parallel paths:
 *  1. **Rust core** - when the native bridge is reachable the actual download
 *     is forwarded to `com.rc.launcher.core.RustBridge.downloadAsync` so it
 *     uses the chunked, resumable, checksum-verifying engine the rest of the
 *     launcher uses (task 2). The bridge wrapper lives in the `core` module
 *     (`com.rc.launcher.core.ControlLayoutDownloaderBridge`) so the model
 *     layer stays Android-independent.
 *  2. **Pure-JVM fallback** - a small [HttpURLConnection] based client that
 *     supports `Range` resume (via a `.part` side-car), per-mirror fallback,
 *     SHA-1 verification and exponential backoff. This is what unit tests
 *     exercise and what runs when the native library is not loaded (Compose
 *     previews / `viewModel()` without an Android process).
 *
 * Callers obtain a [ControlLayoutDownloader] via [ControlLayoutDownloaders.fromBridge];
 * they should not depend on which one they get.
 */
interface ControlLayoutDownloader {
    /**
     * Download [pkg] into [dest], honouring [pkg.mirrors] as fallback targets.
     * Returns the resulting [File] (== `dest`) on success or the final
     * [DownloadFailure] (cause + last URL tried) on permanent failure.
     */
    suspend fun download(pkg: ControlLayoutPackage, dest: File): Result<File>

    /**
     * Variant exposed for the "free-form URL" case (the user pastes an external
     * URL). Mirrors in [pkg.mirrors] are still appended so the same resilience
     * strategy applies.
     */
    suspend fun downloadUrl(
        url: String,
        dest: File,
        sha1: String? = null,
        mirrors: List<String> = emptyList(),
    ): Result<File>
}

/** Why a download failed (after every mirror / retry was exhausted). */
data class DownloadFailure(val reason: String, val lastUrl: String?, val attempts: Int) :
    Exception(reason) {
    override fun toString(): String = "DownloadFailure(reason=$reason, lastUrl=$lastUrl, attempts=$attempts)"
}

/**
 * Build the right downloader for the runtime:
 *  * If a `com.rc.launcher.core.ControlLayoutDownloaderBridge` is reachable,
 *    use it so we go through the Rust download manager (with chunked parallel
 *    range downloads, checksum, mirror fallback, event bus progress).
 *  * Otherwise fall back to the pure-JVM client (Compose previews / unit tests).
 */
object ControlLayoutDownloaders {
    /**
     * Build the right [ControlLayoutDownloader] for the runtime.
     *
     * Production callers wire the Rust-core bridge explicitly by passing the
     * `com.rc.launcher.core.ControlLayoutDownloaderBridge.get()` instance; when
     * omitted, the model layer falls back to the JVM client (Compose previews
     * / unit tests). The bridge lookup is intentionally lazy so the model
     * package doesn't compile-depend on the `app.core` subpackage.
     */
    fun fromBridge(@Suppress("UNUSED_PARAMETER") rustBridge: Any? = null): ControlLayoutDownloader =
        if (rustBridge != null && rustBridge is ControlLayoutDownloader) {
            rustBridge
        } else {
            JvmControlLayoutDownloader()
        }
}

/**
 * Pure-JVM downloader used as the fallback when the native bridge is not
 * available. Implements just enough of the FCL/Zalith "control layout"
 * downloader for the import pipeline to work in unit tests and Compose
 * previews:
 *  * `Range`-aware resume through a `.part` side-car (the same scheme the Rust
 *    download manager uses, see task 2).
 *  * Per-mirror fallback.
 *  * Exponential backoff with jitter.
 *  * Optional SHA-1 verification of the assembled payload.
 *
 * Deliberately *not* a general-purpose HTTP client: only what the library UI
 * needs (small JSON payloads, no streaming, no concurrency).
 */
class JvmControlLayoutDownloader(
    private val timeoutMs: Int = 8000,
    private val maxAttemptsPerUrl: Int = 3,
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
        val allUrls = (listOf(url) + mirrors).distinct()
        var lastError: Throwable? = null
        var attempts = 0
        for (current in allUrls) {
            for (attempt in 1..maxAttemptsPerUrl) {
                attempts++
                val result = tryDownloadOnce(current, dest)
                if (result.isSuccess) {
                    val file = result.getOrThrow()
                    if (sha1 != null) {
                        val actual = sha1Of(file)
                        if (!actual.equals(sha1, ignoreCase = true)) {
                            // Checksum mismatch: throw away the bad file and try
                            // the next mirror (the file may have been truncated
                            // by a hostile proxy).
                            file.delete()
                            lastError = DownloadFailure(
                                reason = "SHA-1 mismatch: expected=$sha1 actual=$actual",
                                lastUrl = current,
                                attempts = attempts,
                            )
                            break
                        }
                    }
                    return@withContext Result.success(file)
                }
                lastError = result.exceptionOrNull()
                // Backoff: 200ms, 400ms, 800ms (+/- 25% jitter).
                val sleep = (200L shl (attempt - 1))
                try {
                    Thread.sleep(sleep + (sleep / 4 * (Math.random() - 0.5).toLong()))
                } catch (_: InterruptedException) {
                    return@withContext Result.failure(DownloadFailure("interrupted", current, attempts))
                }
            }
        }
        Result.failure(lastError ?: DownloadFailure(reason = "no URLs", lastUrl = null, attempts = attempts))
    }

    private fun tryDownloadOnce(url: String, dest: File): kotlin.Result<File> {
        val partFile = File(dest.parentFile, dest.name + ".part")
        val existing = if (partFile.exists()) partFile.length() else 0L
        return try {
            val conn = URL(url).openConnection() as HttpURLConnection
            conn.connectTimeout = timeoutMs
            conn.readTimeout = timeoutMs
            conn.instanceFollowRedirects = true
            conn.setRequestProperty("User-Agent", "rc-launcher/1.0 (control-layout)")
            if (existing > 0) {
                conn.setRequestProperty("Range", "bytes=$existing-")
            }
            val code = conn.responseCode
            when {
                code in 200..299 || code == HttpURLConnection.HTTP_PARTIAL -> {
                    val total = conn.contentLengthLong.let {
                        if (code == HttpURLConnection.HTTP_PARTIAL) it + existing else it
                    }
                    conn.inputStream.use { input ->
                        RandomAccessFile(partFile, "rw").use { raf ->
                            if (code == HttpURLConnection.HTTP_PARTIAL) {
                                raf.seek(existing)
                            } else {
                                raf.setLength(0)
                                raf.seek(0)
                            }
                            val buf = ByteArray(64 * 1024)
                            while (true) {
                                val n = input.read(buf)
                                if (n <= 0) break
                                raf.write(buf, 0, n)
                            }
                        }
                    }
                    // Move the assembled file into place and drop the partial.
                    if (!partFile.renameTo(dest)) {
                        partFile.copyTo(dest, overwrite = true)
                        partFile.delete()
                    }
                    if (total > 0 && dest.length() != total) {
                        return kotlin.Result.failure(IOException("truncated download: got ${dest.length()} of $total"))
                    }
                    kotlin.Result.success(dest)
                }
                code == HttpURLConnection.HTTP_NOT_FOUND || code == HttpURLConnection.HTTP_GONE ->
                    kotlin.Result.failure(IOException("HTTP $code"))
                else -> kotlin.Result.failure(IOException("unexpected HTTP $code"))
            }
        } catch (e: Exception) {
            // Preserve partial progress: leave .part in place so a future resume
            // continues from where we left off.
            kotlin.Result.failure(DownloadFailure(reason = e.message ?: e.javaClass.simpleName, lastUrl = url, attempts = 1))
        }
    }

    companion object {
        /** SHA-1 hex digest of [file]'s bytes. */
        fun sha1Of(file: File): String {
            val md = MessageDigest.getInstance("SHA-1")
            file.inputStream().use { input ->
                val buf = ByteArray(64 * 1024)
                while (true) {
                    val n = input.read(buf)
                    if (n <= 0) break
                    md.update(buf, 0, n)
                }
            }
            return md.digest().joinToString("") { "%02x".format(it) }
        }
    }
}
