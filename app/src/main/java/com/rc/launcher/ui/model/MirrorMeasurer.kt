package com.rc.launcher.ui.model

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.net.HttpURLConnection
import java.net.URL

/**
 * Latency probe result for a single [MirrorSource] (task 14).
 *
 * `ms` is the round-trip time in milliseconds, or `null` when the mirror was
 * unreachable (the reason, if any, is surfaced in [error]).
 */
data class MirrorLatency(
    val mirrorId: String,
    val ms: Long?,
    val error: String?,
)

/**
 * Measures a mirror's reachability / latency. Kept as an interface so the
 * Settings Center can be unit-tested fully offline (task 14 / task 21): the
 * real screen injects [DefaultMirrorMeasurer], tests inject a fake.
 */
interface MirrorMeasurer {
    suspend fun probe(mirror: MirrorSource): MirrorLatency
}

/**
 * Default latency probe: a short, time-boxed HTTP HEAD against the mirror's
 * base URL. Pure JDK ([java.net]), no extra dependency; runs on
 * [Dispatchers.IO] so it never blocks the UI thread.
 */
object DefaultMirrorMeasurer : MirrorMeasurer {
    private const val TIMEOUT_MS = 4000

    override suspend fun probe(mirror: MirrorSource): MirrorLatency = withContext(Dispatchers.IO) {
        if (mirror.official) {
            // "Official / direct to Mojang" has no domestic rewrite target.
            return@withContext MirrorLatency(mirror.id, null, null)
        }
        val start = System.nanoTime()
        try {
            val conn = URL(mirror.baseUrl).openConnection() as HttpURLConnection
            conn.requestMethod = "HEAD"
            conn.connectTimeout = TIMEOUT_MS
            conn.readTimeout = TIMEOUT_MS
            conn.instanceFollowRedirects = true
            val code = conn.responseCode
            conn.disconnect()
            if (code in 200..399) {
                MirrorLatency(mirror.id, (System.nanoTime() - start) / 1_000_000, null)
            } else {
                MirrorLatency(mirror.id, null, "HTTP $code")
            }
        } catch (e: Exception) {
            MirrorLatency(mirror.id, null, e.message ?: e.javaClass.simpleName)
        }
    }
}

/**
 * Live state of a mirror speed test driven by
 * [SettingsViewModel.measureAndSelectFastestMirror] (task 14).
 */
sealed class MirrorProbeState {
    /** No test has been run yet. */
    object Idle : MirrorProbeState()

    /** Probing in progress: [done] of [total] mirrors measured. */
    data class Measuring(val done: Int, val total: Int) : MirrorProbeState()

    /** Finished. [bestId] is the fastest reachable mirror, or `null` if none. */
    data class Done(val bestId: String?, val latencies: List<MirrorLatency>) : MirrorProbeState()

    /** The whole operation failed before producing any result. */
    data class Error(val message: String?) : MirrorProbeState()
}
