package com.rc.launcher.core

import org.json.JSONObject
import java.util.concurrent.CopyOnWriteArrayList

/**
 * Receives raw JSON events from the Rust core (task 10). Pass an implementation
 * to [RustBridge.eventBusSubscribe]; the core invokes [onEvent] on a background
 * thread for every progress / log / lifecycle / error event.
 */
interface RcEventSink {
    fun onEvent(json: String)
}

/** Category of an [RcEvent], mirroring the Rust `EventKind`. */
enum class RcEventKind(val raw: String) {
    PROGRESS("progress"),
    LOG("log"),
    LIFECYCLE("lifecycle"),
    ERROR("error"),
    STATUS("status"),
    UNKNOWN("unknown");

    companion object {
        fun fromRaw(raw: String?): RcEventKind = entries.firstOrNull { it.raw == raw } ?: UNKNOWN
    }
}

/**
 * A single event that crossed the Rust->Kotlin boundary, decoded once from the
 * JSON string so Compose code never reparses.
 */
data class RcEvent(
    val seq: Long,
    val kind: RcEventKind,
    val message: String,
    val scope: String,
    val data: JSONObject?,
) {
    /** Downloaded bytes, when this is a progress event. */
    val progressDownloaded: Long get() = data?.optLong("downloaded") ?: 0L
    /** Total bytes, when known (null if the size is not yet known). */
    val progressTotal: Long? get() = data?.optLong("total")?.takeIf { it > 0 }
    /** Fraction in `[0, 1]`, or null when the total is unknown. */
    val progressFraction: Double?
        get() = data?.optDouble("fraction")?.takeIf { !it.isNaN() }
    /** Current download speed in bytes/second (task 30). 0 if unknown. */
    val progressSpeedBps: Long get() = data?.optLong("speed_bps") ?: 0L
    /** Download status string (task 30): "running", "paused", "completed",
     *  "failed", or null when not a download progress event. */
    val progressStatus: String? get() = data?.optString("status")?.takeIf { !it.isNullOrEmpty() }

    companion object {
        fun parse(json: String): RcEvent {
            val o = JSONObject(json)
            val data = if (o.has("data") && !o.isNull("data")) o.optJSONObject("data") else null
            return RcEvent(
                seq = o.optLong("seq", 0L),
                kind = RcEventKind.fromRaw(o.optString("kind")),
                message = o.optString("message", ""),
                scope = o.optString("scope", "global"),
                data = data,
            )
        }
    }
}

/** Listener for decoded [RcEvent]s. */
fun interface RcEventListener {
    fun onEvent(event: RcEvent)
}

/**
 * Kotlin-side event bus over the Rust [RustBridge] channel (task 10).
 *
 * Call [connect] once (e.g. in `Application.onCreate`): it registers [RcEventBus]
 * as the [RcEventSink] with the Rust core. Every event is decoded into an
 * [RcEvent] and dispatched to registered [RcEventListener]s on the calling
 * (Rust worker) thread. Listeners that need the main thread should post via
 * `Handler(Looper.getMainLooper())` / `Dispatchers.Main` themselves -- or wrap
 * this bus in `callbackFlow { ... }` once coroutines are on the classpath.
 *
 * Thread safety: dispatch uses a [CopyOnWriteArrayList], so listeners may be
 * added/removed while events are streaming. The Rust side already guarantees a
 * single global sink and marshals each event as one JSON [String] (no
 * field-by-field JNI overhead), matching the zero-copy goal of task 10.
 */
object RcEventBus : RcEventSink {
    private val listeners = CopyOnWriteArrayList<RcEventListener>()

    /** Attach this bus to the Rust core. Idempotent; returns true if a previous
     * sink was replaced. */
    fun connect(): Boolean = RustBridge.eventBusSubscribe(this)

    /** Detach from the Rust core and drop all listeners. */
    fun disconnect() {
        RustBridge.eventBusUnsubscribe()
        listeners.clear()
    }

    val isConnected: Boolean get() = RustBridge.eventBusHasSink()

    fun addListener(listener: RcEventListener): Boolean = listeners.add(listener)
    fun removeListener(listener: RcEventListener): Boolean = listeners.remove(listener)

    override fun onEvent(json: String) {
        val event = runCatching { RcEvent.parse(json) }.getOrElse {
            // Surface a malformed event instead of silently dropping it.
            RcEvent(0, RcEventKind.ERROR, json, "bus", null)
        }
        for (l in listeners) runCatching { l.onEvent(event) }
    }
}

/** Spec for [RustBridge.runAsync]. */
data class RcJobSpec(
    val scope: String = "job",
    val label: String = "job",
    val steps: Long = 1,
    val failAt: Long? = null,
    val delayMs: Long = 0,
) {
    fun toJson(): String = JSONObject().apply {
        put("scope", scope)
        put("label", label)
        put("steps", steps)
        if (failAt != null) put("fail_at", failAt)
        put("delay_ms", delayMs)
    }.toString()
}

/** Handle returned by [RustBridge.runAsync]. */
data class RcJobHandle(val ok: Boolean, val scope: String)

/** A single file download task for [RcDownloadJobSpec]. */
data class RcDownloadTask(
    val url: String,
    val dest: String,
    val size: Long? = null,
    val sha1: String? = null,
    val md5: String? = null,
    val mirrors: List<String> = emptyList(),
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("url", url)
        put("dest", dest)
        if (size != null) put("size", size)
        if (sha1 != null) put("sha1", sha1)
        if (md5 != null) put("md5", md5)
        if (mirrors.isNotEmpty()) put("mirrors", org.json.JSONArray(mirrors))
    }
}

/**
 * Spec for [RustBridge.downloadAsync] (task 2 + task 30 enhancements).
 *
 * Includes rate-limiting, sequential download, and checksum-retry options
 * for large modpack / resource-pack downloads on weak China-mainland networks.
 */
data class RcDownloadJobSpec(
    val scope: String = "download",
    val label: String = "download",
    val concurrency: Int? = null,
    val chunkConcurrency: Int? = null,
    val chunkSize: Long? = null,
    val maxRetries: Long? = null,
    val maxBytesPerSecond: Long? = null,
    val maxChecksumRetries: Long? = null,
    val sequential: Boolean? = null,
    val tasks: List<RcDownloadTask>,
) {
    fun toJson(): String = JSONObject().apply {
        put("scope", scope)
        put("label", label)
        if (concurrency != null) put("concurrency", concurrency)
        if (chunkConcurrency != null) put("chunk_concurrency", chunkConcurrency)
        if (chunkSize != null) put("chunk_size", chunkSize)
        if (maxRetries != null) put("max_retries", maxRetries)
        if (maxBytesPerSecond != null) put("max_bytes_per_second", maxBytesPerSecond)
        if (maxChecksumRetries != null) put("max_checksum_retries", maxChecksumRetries)
        if (sequential != null) put("sequential", sequential)
        val tasksArray = org.json.JSONArray()
        for (t in tasks) tasksArray.put(t.toJson())
        put("tasks", tasksArray)
    }.toString()
}

/** Handle returned by [RustBridge.downloadAsync]. */
data class RcDownloadJobHandle(val ok: Boolean, val scope: String)

/**
 * Start a download job using [RcDownloadJobSpec] (task 2 + task 30).
 * Returns the [RcDownloadJobHandle] so callers can [cancelAsync] the scope
 * or subscribe to progress events on the bus.
 */
fun startDownload(spec: RcDownloadJobSpec): RcDownloadJobHandle {
    val out = org.json.JSONObject(RustBridge.downloadAsync(spec.toJson()))
    return RcDownloadJobHandle(out.optBoolean("ok", false), out.optString("scope", ""))
}

/**
 * Cancel / pause a running download (or any async job) by scope (task 30).
 * Returns `true` if a matching job was found and the cancel flag was set.
 */
fun cancelDownload(scope: String): Boolean = RustBridge.cancelAsync(scope)
