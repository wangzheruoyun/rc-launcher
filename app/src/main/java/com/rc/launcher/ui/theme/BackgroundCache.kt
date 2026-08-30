package com.rc.launcher.ui.theme

import android.content.Context
import android.net.Uri
import java.io.File

/**
 * Caches the chosen background image inside the app's own `filesDir/background/`
 * directory so the launcher keeps showing it even if the original `content://`
 * grant is later revoked (e.g. the gallery app is uninstalled). The cached file
 * lives under a single app-owned root; [BackgroundValidator.isSafePath] is used
 * to guarantee we only ever *load* from there — this is the "背景资源经安全路径
 * 校验后缓存" (cache the background asset after a safe-path check) requirement of
 * task 11.
 */
object BackgroundCache {
    private const val DIR_NAME = "background"
    private const val FILE_NAME = "bg.png"

    /** The app-owned directory that holds the cached background. */
    fun cacheDir(context: Context): File =
        File(context.filesDir, DIR_NAME).also { if (!it.exists()) it.mkdirs() }

    /** The single cached file (may not exist yet). */
    fun currentFile(context: Context): File = File(cacheDir(context), FILE_NAME)

    /** Absolute, scheme-less path of the cached image, or null when absent. */
    fun cachedPath(context: Context): String? =
        currentFile(context).takeIf { it.exists() }?.absolutePath

    /**
     * Copy a picked [uri] into the cache and return its `file://` path, or null
     * on any IO failure (the caller then keeps the original `content://` URI).
     */
    fun cacheUri(context: Context, uri: Uri): String? = runCatching {
        val out = currentFile(context)
        context.contentResolver.openInputStream(uri)?.use { input ->
            out.outputStream().use { output -> input.copyTo(output) }
        } ?: return null
        "file://" + out.absolutePath
    }.getOrNull()

    /** Remove the cached image (used when the user clears the background). */
    fun clear(context: Context) {
        runCatching { currentFile(context).delete() }
    }
}
