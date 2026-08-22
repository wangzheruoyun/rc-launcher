package com.rc.launcher.ui.awt

import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.parseJson

/**
 * Kotlin view of the AWT session snapshot the Rust core returns (task 18).
 *
 * Every bridge call answers with the same JSON document (`awtOpen`, `awtInfo`,
 * `awtConfigure`), so one parser covers them all. Parsing is **fail-soft**: an
 * unexpected / truncated document yields a closed session carrying the error
 * message instead of throwing, because the AWT canvas must never be able to
 * crash the launcher UI (task 19).
 *
 * Pure Kotlin (no Android imports): unit-testable on the JVM.
 */
data class AwtSessionInfo(
    /** Whether a live session exists in the core. */
    val open: Boolean = false,
    /** `headless` / `cacio8` / `cacio17`. */
    val backend: String = "headless",
    val screenWidth: Int = 0,
    val screenHeight: Int = 0,
    val surfaceWidth: Int = 0,
    val surfaceHeight: Int = 0,
    val scaleMode: AwtScaleMode = AwtScaleMode.FIT,
    /** Where the desktop is drawn inside the surface (as computed by the core). */
    val placement: AwtPlacement = AwtPlacement(0, 0, 0, 0),
    val focused: Boolean = false,
    /** `getModifiersEx()` value of the held modifiers / buttons. */
    val modifiers: Int = 0,
    /** AWT records queued for the JVM. */
    val pendingEvents: Int = 0,
    /** Bytes an RGBA8888 copy of the whole desktop needs. */
    val rgbaLen: Int = 0,
    val uptimeMs: Long = 0,
    /** Presented frames per second, as measured by the core's canvas. */
    val fps: Float = 0f,
    val framesPresented: Long = 0,
    val framesDropped: Long = 0,
    val framesAccepted: Long = 0,
    val framesRejected: Long = 0,
    val eventsDropped: Long = 0,
    /** State of the link to the game JVM. */
    val link: AwtLinkInfo = AwtLinkInfo(),
    /** Frame channel path, when a named-pipe transport is attached. */
    val framesChannel: String? = null,
    /** Event channel path, when a named-pipe transport is attached. */
    val eventsChannel: String? = null,
    /** Error reported by the core (`{"error": …}`), if any. */
    val error: String? = null,
) {
    /** The viewport used to place the bitmap and map touches. */
    val viewport: AwtViewport
        get() = AwtViewport(screenWidth, screenHeight, surfaceWidth, surfaceHeight, scaleMode)

    /** `true` when a transport is attached (the JVM can reach us). */
    val hasTransport: Boolean get() = framesChannel != null

    /** One-line summary for the diagnostics card. */
    fun describe(): String = when {
        !open -> "未开启"
        else -> "$backend · ${screenWidth}x$screenHeight → ${surfaceWidth}x$surfaceHeight · " +
            "${link.label} · ${framesAccepted} 帧"
    }

    companion object {
        /** A closed session (no core session, no error). */
        val CLOSED = AwtSessionInfo()

        /** A closed session carrying [message] as its error. */
        fun failed(message: String): AwtSessionInfo = AwtSessionInfo(error = message)

        /**
         * Parse a snapshot. `{"error": …}` and malformed input both yield a closed
         * session with [error] set.
         */
        fun parse(json: String?): AwtSessionInfo {
            val root = parseJson(json.orEmpty()) as? JsonValue.Obj
                ?: return failed("无法解析核心返回的 AWT 会话信息")
            root.str("error")?.let { return failed(it) }
            // `awtInfo` on a closed session answers `{"open":false}`.
            val screen = root.obj("screen")
            val surface = root.obj("surface")
            val canvas = root.obj("canvas")
            val session = root.obj("session")
            val open = root.bool("open") ?: (screen != null)
            if (!open) return CLOSED
            val placement = root.obj("placement")
            return AwtSessionInfo(
                open = true,
                backend = root.str("backend") ?: "headless",
                screenWidth = screen?.int("width") ?: 0,
                screenHeight = screen?.int("height") ?: 0,
                surfaceWidth = surface?.int("width") ?: 0,
                surfaceHeight = surface?.int("height") ?: 0,
                scaleMode = AwtScaleMode.fromId(root.str("scale_mode")),
                placement = AwtPlacement(
                    x = placement?.int("x") ?: 0,
                    y = placement?.int("y") ?: 0,
                    width = placement?.int("width") ?: 0,
                    height = placement?.int("height") ?: 0,
                ),
                focused = root.bool("focused") ?: false,
                modifiers = root.int("modifiers") ?: 0,
                pendingEvents = root.int("pending_events") ?: 0,
                rgbaLen = root.int("rgba_len") ?: 0,
                uptimeMs = root.long("uptime_ms") ?: 0,
                fps = canvas?.float("fps") ?: 0f,
                framesPresented = canvas?.long("frames_presented") ?: 0,
                framesDropped = canvas?.long("frames_dropped") ?: 0,
                framesAccepted = session?.long("frames_accepted") ?: 0,
                framesRejected = session?.long("frames_rejected") ?: 0,
                eventsDropped = session?.long("events_dropped") ?: 0,
                link = AwtLinkInfo.parse(root.obj("link")),
                framesChannel = root.obj("transport")?.str("frames"),
                eventsChannel = root.obj("transport")?.str("events"),
            )
        }
    }
}

/** State of the transport between the launcher and the game JVM. */
data class AwtLinkInfo(
    /** `detached` (nothing feeds the canvas), `attached`, or `ended`. */
    val state: String = "detached",
    val framesAccepted: Long = 0,
    val framesRejected: Long = 0,
    val eventsWritten: Long = 0,
    val eventsLost: Long = 0,
    /** Why the link ended (`null` while healthy). */
    val reason: String? = null,
) {
    val attached: Boolean get() = state == "attached"
    val ended: Boolean get() = state == "ended"

    /** Localised label for the diagnostics card. */
    val label: String
        get() = when (state) {
            "attached" -> "已连接"
            "ended" -> "已断开"
            else -> "未连接"
        }

    companion object {
        fun parse(obj: JsonValue.Obj?): AwtLinkInfo {
            if (obj == null) return AwtLinkInfo()
            return AwtLinkInfo(
                state = obj.str("state") ?: "detached",
                framesAccepted = obj.long("frames_accepted") ?: 0,
                framesRejected = obj.long("frames_rejected") ?: 0,
                eventsWritten = obj.long("events_written") ?: 0,
                eventsLost = obj.long("events_lost") ?: 0,
                reason = obj.str("reason"),
            )
        }
    }
}

/**
 * Result of one `awtPollFrame`: the damaged region that was refreshed, or `null`
 * (see [AwtFrameUpdate.NONE]) when nothing changed since the previous poll — in
 * which case the UI skips both the bitmap upload and the recomposition.
 */
data class AwtFrameUpdate(
    val changed: Boolean,
    val damage: AwtRect = AwtRect(0, 0, 0, 0),
    val bytes: Int = 0,
    val error: String? = null,
) {
    companion object {
        /** Nothing changed. */
        val NONE = AwtFrameUpdate(changed = false)

        /** Parse the JSON `awtPollFrame` / `awtSubmitFrame` return value. */
        fun parse(json: String?): AwtFrameUpdate {
            val root = parseJson(json.orEmpty()) as? JsonValue.Obj
                ?: return AwtFrameUpdate(false, error = "无法解析核心返回的帧信息")
            root.str("error")?.let { return AwtFrameUpdate(false, error = it) }
            if (root.bool("changed") != true) return NONE
            return AwtFrameUpdate(
                changed = true,
                damage = AwtRect(
                    root.int("x") ?: 0,
                    root.int("y") ?: 0,
                    root.int("width") ?: 0,
                    root.int("height") ?: 0,
                ),
                bytes = root.int("bytes") ?: 0,
            )
        }
    }
}

/** Result of one `awtInput` batch. */
data class AwtInputResult(
    val queued: Int = 0,
    val pending: Int = 0,
    val modifiers: Int = 0,
    val focused: Boolean = true,
    val pointer: AwtPoint = AwtPoint(0, 0),
    /** Events the core could not understand (never fatal for the batch). */
    val rejected: List<String> = emptyList(),
    val error: String? = null,
) {
    companion object {
        val EMPTY = AwtInputResult()

        fun parse(json: String?): AwtInputResult {
            val root = parseJson(json.orEmpty()) as? JsonValue.Obj
                ?: return AwtInputResult(error = "无法解析核心返回的输入结果")
            root.str("error")?.let { return AwtInputResult(error = it) }
            val pointer = root.obj("pointer")
            return AwtInputResult(
                queued = root.int("queued") ?: 0,
                pending = root.int("pending") ?: 0,
                modifiers = root.int("modifiers") ?: 0,
                focused = root.bool("focused") ?: true,
                pointer = AwtPoint(pointer?.int("x") ?: 0, pointer?.int("y") ?: 0),
                rejected = (root.entries["rejected"] as? JsonValue.Arr)
                    ?.items
                    ?.mapNotNull { (it as? JsonValue.Str)?.value }
                    ?: emptyList(),
            )
        }
    }
}

// ---- small JSON accessors ---------------------------------------------------

internal fun JsonValue.Obj.obj(key: String): JsonValue.Obj? = entries[key] as? JsonValue.Obj
internal fun JsonValue.Obj.str(key: String): String? = (entries[key] as? JsonValue.Str)?.value
internal fun JsonValue.Obj.bool(key: String): Boolean? = (entries[key] as? JsonValue.Bool)?.value
internal fun JsonValue.Obj.double(key: String): Double? = (entries[key] as? JsonValue.Num)?.value
internal fun JsonValue.Obj.int(key: String): Int? = double(key)?.toInt()
internal fun JsonValue.Obj.long(key: String): Long? = double(key)?.toLong()
internal fun JsonValue.Obj.float(key: String): Float? = double(key)?.toFloat()
