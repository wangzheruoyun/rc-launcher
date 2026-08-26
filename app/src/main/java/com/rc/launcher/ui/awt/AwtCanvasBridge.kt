package com.rc.launcher.ui.awt

import com.rc.launcher.core.RustBridge
import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.toJsonString
import java.nio.ByteBuffer

/**
 * The Compose ⇄ Rust contract of the AWT/Swing compatibility layer (task 18).
 *
 * The production implementation ([RustAwtCanvasBridge]) is a thin JSON wrapper
 * around [RustBridge]; [FakeAwtCanvasBridge] re-implements just enough of the
 * core (a framebuffer, damage tracking, an input log) to drive the whole UI —
 * including the pixel path — on a plain JVM, so the canvas is unit-testable and
 * usable in `@Preview` without the native library. Same repository split as
 * tasks 14–16.
 *
 * Threading: every method blocks for the duration of one JNI call (microseconds
 * for `poll`, which is a damage-limited memcpy). `poll` is meant to be called
 * from the frame callback; the rest from a coroutine.
 */
interface AwtCanvasBridge {
    /** Open (or replace) the live session. */
    fun open(config: AwtSessionConfig): AwtSessionInfo

    /** Close the session and stop its pump threads. `true` if one was open. */
    fun close(): Boolean

    /** Session + transport snapshot (`AwtSessionInfo.CLOSED` when none). */
    fun info(): AwtSessionInfo

    /** Apply geometry / focus / repaint changes. */
    fun configure(request: AwtConfigureRequest): AwtSessionInfo

    /** Create + pump the named-pipe channels in [dir] for the running session. */
    fun attachTransport(dir: String): AwtSessionInfo

    /** Hand a batch of input events over (one call per UI frame). */
    fun input(events: List<AwtInputEvent>): AwtInputResult

    /** Feed one encoded `RCAF` frame in (self-test / Kotlin-owned transport). */
    fun submitFrame(frame: ByteArray): AwtFrameUpdate

    /**
     * Refresh [buffer] — a **direct** `ByteBuffer` of `rgbaLen` bytes backing the
     * Compose bitmap — with whatever changed, and report the damaged region.
     */
    fun poll(buffer: ByteBuffer): AwtFrameUpdate

    /** Queued AWT records for a Kotlin-side transport (usually unused). */
    fun drainEvents(): ByteArray

    /**
     * Take the control messages the JVM sent (cursor shape, window title,
     * clipboard hand-off / request, IME, beep) plus the current projection.
     *
     * One call per UI frame, next to [poll]: the messages carry side effects that
     * must fire exactly once (push to the Android clipboard, buzz, pop the soft
     * keyboard), the projection is what the UI renders.
     */
    fun drainControl(): AwtControlBatch

    /** Answer the control plane (clipboard contents, liveness, reset). */
    fun control(request: AwtControlRequest): AwtControlResult

    /** Feed one encoded `RCAC` control message in (self-test / Kotlin transport). */
    fun submitControl(message: ByteArray): Boolean
}

/** Everything the UI can decide when opening a session. */
data class AwtSessionConfig(
    /** Virtual AWT desktop size — must match `-Dcacio.managed.screensize`. */
    val screenWidth: Int = 1280,
    val screenHeight: Int = 720,
    /** Size of the Compose surface the desktop is drawn on, in pixels. */
    val surfaceWidth: Int = 1280,
    val surfaceHeight: Int = 720,
    val scaleMode: AwtScaleMode = AwtScaleMode.FIT,
    /** Click tolerance in desktop pixels (a finger always jitters). */
    val clickSlop: Int = 8,
    /** Upper bound for the outbound event queue. */
    val maxPendingEvents: Int = 4096,
    /** `jre8` / `jre17` / … — selects the caciocavallo backend. */
    val javaVersion: String? = null,
    /** Directory for the named-pipe channels, or `null` for an off-line canvas. */
    val transportDir: String? = null,
) {
    /** JSON for `RustBridge.awtOpen`. */
    fun toJson(): String {
        val entries = linkedMapOf<String, JsonValue>(
            "screen" to size(screenWidth, screenHeight),
            "surface" to size(surfaceWidth, surfaceHeight),
            "scale_mode" to JsonValue.Str(scaleMode.id),
            "click_slop" to JsonValue.Num(clickSlop.toDouble()),
            "max_pending_events" to JsonValue.Num(maxPendingEvents.toDouble()),
        )
        javaVersion?.let { entries["java_version"] = JsonValue.Str(it) }
        transportDir?.let {
            entries["transport"] = JsonValue.Obj(linkedMapOf("dir" to JsonValue.Str(it)))
        }
        return JsonValue.Obj(entries).toJsonString()
    }
}

/** A partial update of the live session (only the set fields are applied). */
data class AwtConfigureRequest(
    val surfaceWidth: Int? = null,
    val surfaceHeight: Int? = null,
    val screenWidth: Int? = null,
    val screenHeight: Int? = null,
    val scaleMode: AwtScaleMode? = null,
    val focus: Boolean? = null,
    val releaseAll: Boolean = false,
    val resetInput: Boolean = false,
    val clear: Boolean = false,
    val fillArgb: Int? = null,
) {
    /** JSON for `RustBridge.awtConfigure`. */
    fun toJson(): String {
        val entries = linkedMapOf<String, JsonValue>()
        if (surfaceWidth != null && surfaceHeight != null) {
            entries["surface"] = size(surfaceWidth, surfaceHeight)
        }
        if (screenWidth != null && screenHeight != null) {
            entries["screen"] = size(screenWidth, screenHeight)
        }
        scaleMode?.let { entries["scale_mode"] = JsonValue.Str(it.id) }
        focus?.let { entries["focus"] = JsonValue.Bool(it) }
        if (releaseAll) entries["release_all"] = JsonValue.Bool(true)
        if (resetInput) entries["reset_input"] = JsonValue.Bool(true)
        if (clear) entries["clear"] = JsonValue.Bool(true)
        // `fill` is an unsigned 32-bit ARGB colour on the wire.
        fillArgb?.let { entries["fill"] = JsonValue.Num((it.toLong() and 0xFFFFFFFFL).toDouble()) }
        return JsonValue.Obj(entries).toJsonString()
    }

    /** `true` when the request would not change anything. */
    val isEmpty: Boolean
        get() = surfaceWidth == null && screenWidth == null && scaleMode == null &&
            focus == null && !releaseAll && !resetInput && !clear && fillArgb == null
}

private fun size(width: Int, height: Int): JsonValue = JsonValue.Obj(
    linkedMapOf(
        "width" to JsonValue.Num(width.toDouble()),
        "height" to JsonValue.Num(height.toDouble()),
    ),
)

/**
 * [RustBridge]-backed bridge (the real thing).
 *
 * Every call is wrapped in [runCatching]: a build without `librc_launcher.so`
 * (or a JNI failure) degrades to an error-carrying result instead of taking the
 * UI down with an `UnsatisfiedLinkError` (task 19).
 */
class RustAwtCanvasBridge : AwtCanvasBridge {
    override fun open(config: AwtSessionConfig): AwtSessionInfo =
        parseInfo { RustBridge.awtOpen(config.toJson()) }

    override fun close(): Boolean = runCatching {
        val json = RustBridge.awtClose()
        (com.rc.launcher.ui.model.json.parseJson(json) as? JsonValue.Obj)?.bool("closed") ?: false
    }.getOrDefault(false)

    override fun info(): AwtSessionInfo = parseInfo { RustBridge.awtInfo() }

    override fun configure(request: AwtConfigureRequest): AwtSessionInfo =
        if (request.isEmpty) info() else parseInfo { RustBridge.awtConfigure(request.toJson()) }

    override fun attachTransport(dir: String): AwtSessionInfo = parseInfo {
        RustBridge.awtAttachTransport(
            JsonValue.Obj(linkedMapOf("dir" to JsonValue.Str(dir))).toJsonString(),
        )
    }

    override fun input(events: List<AwtInputEvent>): AwtInputResult {
        if (events.isEmpty()) return AwtInputResult.EMPTY
        return runCatching { AwtInputResult.parse(RustBridge.awtInput(events.toBatchJson())) }
            .getOrElse { AwtInputResult(error = nativeError(it)) }
    }

    override fun submitFrame(frame: ByteArray): AwtFrameUpdate =
        runCatching { AwtFrameUpdate.parse(RustBridge.awtSubmitFrame(frame)) }
            .getOrElse { AwtFrameUpdate(false, error = nativeError(it)) }

    override fun poll(buffer: ByteBuffer): AwtFrameUpdate = runCatching {
        if (buffer.isDirect) {
            AwtFrameUpdate.parse(RustBridge.awtPollFrame(buffer))
        } else {
            // A heap buffer cannot be shared with native code: fall back to the
            // array path (one extra copy each way, still correct).
            val array = if (buffer.hasArray()) buffer.array() else ByteArray(buffer.capacity())
            val update = AwtFrameUpdate.parse(RustBridge.awtPollFrameArray(array))
            if (update.changed && !buffer.hasArray()) {
                buffer.duplicate().apply { position(0) }.put(array)
            }
            update
        }
    }.getOrElse { AwtFrameUpdate(false, error = nativeError(it)) }

    override fun drainEvents(): ByteArray =
        runCatching { RustBridge.awtDrainEvents() }.getOrDefault(ByteArray(0))

    override fun drainControl(): AwtControlBatch =
        runCatching { AwtControlBatch.parse(RustBridge.awtDrainControl()) }
            .getOrElse { AwtControlBatch.failed(nativeError(it)) }

    override fun control(request: AwtControlRequest): AwtControlResult =
        runCatching { AwtControlResult.parse(RustBridge.awtControl(request.toJson())) }
            .getOrElse { AwtControlResult(error = nativeError(it)) }

    override fun submitControl(message: ByteArray): Boolean = runCatching {
        val json = RustBridge.awtSubmitControl(message)
        (com.rc.launcher.ui.model.json.parseJson(json) as? JsonValue.Obj)
            ?.bool("accepted") ?: false
    }.getOrDefault(false)

    private fun parseInfo(call: () -> String): AwtSessionInfo =
        runCatching { AwtSessionInfo.parse(call()) }
            .getOrElse { AwtSessionInfo.failed(nativeError(it)) }

    private fun nativeError(t: Throwable): String =
        "Rust 核心不可用：" + (t.message ?: t.javaClass.simpleName)
}

/**
 * A pure-Kotlin stand-in for the core's session: a framebuffer with damage
 * tracking, an input log and the same fail-soft behaviour. Used by unit tests and
 * previews — it makes the *whole* Compose path (bitmap upload included) testable
 * on the JVM.
 */
class FakeAwtCanvasBridge(
    /** Frames the fake pretends the JVM sent (drives `link.state`). */
    var linkState: String = "detached",
) : AwtCanvasBridge {
    private var session: AwtSessionConfig? = null
    private var pixels: IntArray = IntArray(0)
    private var dirty: AwtRect? = null
    private var scaleMode: AwtScaleMode = AwtScaleMode.FIT
    private var screenW = 0
    private var screenH = 0
    private var surfaceW = 0
    private var surfaceH = 0
    private var focused = true
    private var accepted = 0L
    private var rejected = 0L
    private var transportDir: String? = null

    /** Every event handed to [input], in order (assert on this in tests). */
    val received = mutableListOf<AwtInputEvent>()

    /** Control messages waiting for [drainControl]. */
    private val pendingControl = mutableListOf<AwtControlMessage>()

    /** Projection the fake keeps in step with [submitControl]. */
    private var controlState: AwtControlState = AwtControlState.EMPTY

    /** Every clipboard answer the UI produced (assert on this in tests). */
    val clipboardAnswers = mutableListOf<String?>()

    /** How many times [poll] ran. */
    var polls = 0
        private set

    override fun open(config: AwtSessionConfig): AwtSessionInfo {
        session = config
        screenW = config.screenWidth.coerceIn(1, AwtWire.MAX_CANVAS_DIM)
        screenH = config.screenHeight.coerceIn(1, AwtWire.MAX_CANVAS_DIM)
        surfaceW = config.surfaceWidth.coerceAtLeast(1)
        surfaceH = config.surfaceHeight.coerceAtLeast(1)
        scaleMode = config.scaleMode
        transportDir = config.transportDir
        pixels = IntArray(screenW * screenH) { AwtWire.OPAQUE_BLACK }
        dirty = AwtRect.whole(screenW, screenH)
        focused = true
        accepted = 0
        rejected = 0
        received.clear()
        pendingControl.clear()
        clipboardAnswers.clear()
        controlState = AwtControlState.EMPTY
        polls = 0
        return info()
    }

    override fun close(): Boolean {
        val had = session != null
        session = null
        pixels = IntArray(0)
        dirty = null
        return had
    }

    override fun info(): AwtSessionInfo {
        if (session == null) return AwtSessionInfo.CLOSED
        return AwtSessionInfo(
            open = true,
            backend = if (session?.javaVersion == "jre8") "cacio8" else "cacio17",
            screenWidth = screenW,
            screenHeight = screenH,
            surfaceWidth = surfaceW,
            surfaceHeight = surfaceH,
            scaleMode = scaleMode,
            placement = AwtViewport(screenW, screenH, surfaceW, surfaceH, scaleMode).placement(),
            focused = focused,
            rgbaLen = screenW * screenH * 4,
            framesAccepted = accepted,
            framesRejected = rejected,
            framesPresented = accepted,
            link = AwtLinkInfo(state = linkState, framesAccepted = accepted),
            framesChannel = transportDir?.let { "$it/awt-frames.rcaf" },
            eventsChannel = transportDir?.let { "$it/awt-events.rcae" },
        )
    }

    override fun configure(request: AwtConfigureRequest): AwtSessionInfo {
        if (session == null) return AwtSessionInfo.failed("no AWT session is open")
        if (request.surfaceWidth != null && request.surfaceHeight != null) {
            surfaceW = request.surfaceWidth.coerceAtLeast(1)
            surfaceH = request.surfaceHeight.coerceAtLeast(1)
        }
        if (request.screenWidth != null && request.screenHeight != null) {
            screenW = request.screenWidth.coerceIn(1, AwtWire.MAX_CANVAS_DIM)
            screenH = request.screenHeight.coerceIn(1, AwtWire.MAX_CANVAS_DIM)
            pixels = IntArray(screenW * screenH) { AwtWire.OPAQUE_BLACK }
            dirty = AwtRect.whole(screenW, screenH)
        }
        request.scaleMode?.let { scaleMode = it }
        request.focus?.let { focused = it }
        clampCaretToDesktop()
        if (request.clear || request.fillArgb != null) {
            val colour = request.fillArgb ?: AwtWire.OPAQUE_BLACK
            pixels.fill(colour)
            dirty = AwtRect.whole(screenW, screenH)
        }
        return info()
    }

    override fun attachTransport(dir: String): AwtSessionInfo {
        if (session == null) return AwtSessionInfo.failed("no AWT session is open")
        transportDir = dir
        linkState = "attached"
        return info()
    }

    override fun input(events: List<AwtInputEvent>): AwtInputResult {
        if (session == null) return AwtInputResult(error = "no AWT session is open")
        val viewport = AwtViewport(screenW, screenH, surfaceW, surfaceH, scaleMode)
        var queued = 0
        for (event in events) {
            received.add(event)
            queued += when (event) {
                // A tap on the letterbox bars is not an AWT event (as in the core).
                is AwtPointerEvent -> if (viewport.mapPointer(event.x, event.y) != null) 1 else 0
                is AwtTextEvent -> event.text.length
                is AwtFocusEvent -> {
                    focused = event.gained
                    1
                }
                else -> 1
            }
        }
        return AwtInputResult(queued = queued, focused = focused)
    }

    override fun submitFrame(frame: ByteArray): AwtFrameUpdate {
        if (session == null) return AwtFrameUpdate(false, error = "no AWT session is open")
        val header = AwtWire.decodeFrameHeader(frame)
        val payload = AwtWire.decodeFramePixels(frame)
        if (header == null || payload == null || header.width != screenW || header.height != screenH) {
            rejected++
            return AwtFrameUpdate(false, error = "invalid AWT frame")
        }
        var i = 0
        for (y in header.damage.y until header.damage.y + header.damage.height) {
            for (x in header.damage.x until header.damage.x + header.damage.width) {
                pixels[y * screenW + x] = payload[i++]
            }
        }
        accepted++
        dirty = header.damage
        return AwtFrameUpdate(true, header.damage, payload.size * 4)
    }

    override fun poll(buffer: ByteBuffer): AwtFrameUpdate {
        polls++
        if (session == null) return AwtFrameUpdate(false, error = "no AWT session is open")
        val rect = dirty ?: return AwtFrameUpdate.NONE
        val needed = screenW * screenH * 4
        if (buffer.capacity() < needed) {
            return AwtFrameUpdate(false, error = "RGBA framebuffer too small")
        }
        for (y in rect.y until rect.y + rect.height) {
            for (x in rect.x until rect.x + rect.width) {
                val argb = pixels[y * screenW + x]
                val base = (y * screenW + x) * 4
                buffer.put(base, ((argb shr 16) and 0xFF).toByte())
                buffer.put(base + 1, ((argb shr 8) and 0xFF).toByte())
                buffer.put(base + 2, (argb and 0xFF).toByte())
                buffer.put(base + 3, ((argb shr 24) and 0xFF).toByte())
            }
        }
        dirty = null
        return AwtFrameUpdate(true, rect, rect.area.toInt() * 4)
    }

    override fun drainEvents(): ByteArray = ByteArray(0)

    override fun drainControl(): AwtControlBatch {
        if (session == null) return AwtControlBatch.failed("no AWT session is open")
        val messages = pendingControl.toList()
        pendingControl.clear()
        return AwtControlBatch(
            messages = messages,
            state = controlState,
            clipboardRequests = controlState.clipboardRequests,
        )
    }

    override fun control(request: AwtControlRequest): AwtControlResult {
        if (session == null) return AwtControlResult(error = "no AWT session is open")
        var queued = 0
        if (request.clipboard != null || request.clipboardEmpty) {
            val answered = if (request.clipboardSeq != null) 1 else controlState.clipboardRequests
            queued += AwtControlWire.encodeReply(
                if (request.clipboardEmpty) AwtReplyKind.CLIPBOARD_EMPTY else AwtReplyKind.CLIPBOARD,
                request.clipboardSeq ?: 0,
                request.clipboard.orEmpty(),
            ).size * maxOf(1, answered)
            controlState = controlState.copy(
                clipboardRequests = (controlState.clipboardRequests - maxOf(1, answered))
                    .coerceAtLeast(0),
            )
            clipboardAnswers.add(request.clipboard)
        }
        if (request.pong != null) queued += 1
        if (request.reset) {
            controlState = AwtControlState.EMPTY
            pendingControl.clear()
        }
        return AwtControlResult(
            queued = queued,
            clipboardRequests = controlState.clipboardRequests,
            state = controlState,
        )
    }

    /**
     * Keep the reported Swing caret inside the desktop, exactly as the core does.
     *
     * A caret is a *desktop* coordinate, so shrinking the desktop can leave it
     * outside the picture — and the UI maps it straight through the viewport,
     * which would anchor the IME on the letterbox bars.
     */
    private fun clampCaretToDesktop() {
        val caret = controlState.caret ?: return
        controlState = controlState.copy(
            caret = AwtImeCaret(
                x = caret.x.coerceIn(0, maxOf(0, screenW - 1)),
                y = caret.y.coerceIn(0, maxOf(0, screenH - 1)),
                lineHeight = caret.lineHeight.coerceIn(0, screenH),
            ),
        )
    }

    override fun submitControl(message: ByteArray): Boolean {
        if (session == null) return false
        val parsed = AwtControlWire.decode(message) ?: return false
        pendingControl.add(parsed)
        controlState = when (parsed.kind) {
            AwtControlKind.CURSOR -> controlState.copy(cursor = parsed.cursor)
            AwtControlKind.TITLE -> controlState.copy(title = parsed.text.ifEmpty { null })
            AwtControlKind.CLIPBOARD_SET -> controlState.copy(clipboardOut = parsed.text)
            AwtControlKind.CLIPBOARD_REQUEST ->
                controlState.copy(clipboardRequests = controlState.clipboardRequests + 1)
            AwtControlKind.BEEP -> controlState.copy(beeps = controlState.beeps + 1)
            AwtControlKind.IME_SHOW ->
                controlState.copy(caret = parsed.caret, wantsKeyboard = true)
            AwtControlKind.IME_HIDE -> controlState.copy(caret = null, wantsKeyboard = false)
            AwtControlKind.SCREEN_SIZE -> {
                screenW = parsed.width.coerceIn(1, AwtWire.MAX_CANVAS_DIM)
                screenH = parsed.height.coerceIn(1, AwtWire.MAX_CANVAS_DIM)
                pixels = IntArray(screenW * screenH) { AwtWire.OPAQUE_BLACK }
                dirty = AwtRect.whole(screenW, screenH)
                clampCaretToDesktop()
                controlState
            }
            AwtControlKind.WINDOW_OPENED -> controlState.copy(
                windows = controlState.windows + AwtWindowInfo(parsed.window, parsed.text),
                title = parsed.text.ifEmpty { controlState.title },
            )
            AwtControlKind.WINDOW_CLOSED -> controlState.copy(
                windows = controlState.windows.filterNot { it.id == parsed.window },
            )
            AwtControlKind.BYE -> controlState.copy(
                bye = parsed.text.ifEmpty { "the AWT bridge closed" },
                caret = null,
                wantsKeyboard = false,
            )
        }
        return true
    }
}

/**
 * Process-wide bridge holder, mirroring
 * [com.rc.launcher.ui.model.AccountRepositories]. The real implementation is
 * installed from `RcApplication.onCreate`; until then (previews / unit tests) a
 * [FakeAwtCanvasBridge] keeps the UI alive without the native core.
 */
object AwtBridges {
    @Volatile
    private var _default: AwtCanvasBridge? = null

    val default: AwtCanvasBridge
        get() = _default ?: FakeAwtCanvasBridge().also { _default = it }

    fun install(bridge: AwtCanvasBridge) {
        _default = bridge
    }
}
