package com.rc.launcher.ui.viewmodel

import androidx.lifecycle.ViewModel
import com.rc.launcher.ui.awt.AwtBridges
import com.rc.launcher.ui.awt.AwtCanvasBridge
import com.rc.launcher.ui.awt.AwtConfigureRequest
import com.rc.launcher.ui.awt.AwtFocusEvent
import com.rc.launcher.ui.awt.AwtFrameUpdate
import com.rc.launcher.ui.awt.AwtInputEvent
import com.rc.launcher.ui.awt.AwtInputResult
import com.rc.launcher.ui.awt.AwtKeyEvent
import com.rc.launcher.ui.awt.AwtMouseButton
import com.rc.launcher.ui.awt.AwtPointerEvent
import com.rc.launcher.ui.awt.AwtPointerPhase
import com.rc.launcher.ui.awt.AwtReleaseAllEvent
import com.rc.launcher.ui.awt.AwtScaleMode
import com.rc.launcher.ui.awt.AwtScrollEvent
import com.rc.launcher.ui.awt.AwtSessionConfig
import com.rc.launcher.ui.awt.AwtSessionInfo
import com.rc.launcher.ui.awt.AwtTextEvent
import com.rc.launcher.ui.awt.AwtWire
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * State container of the AWT/Swing canvas (task 18).
 *
 * It owns the three things the Compose layer must not have to think about:
 *
 * 1. **The framebuffer** — one direct [ByteBuffer] of `width * height * 4` RGBA
 *    bytes, reallocated only when the virtual desktop is resized. The Rust core
 *    writes the damaged rows straight into it ([poll]); the composable uploads it
 *    into its `Bitmap`. No pixel is ever copied through a Java array.
 * 2. **Input batching** — pointer / scroll samples are queued and flushed once
 *    per frame (a drag then costs *one* JNI call), while keys, text and focus go
 *    through immediately because they are rare and latency-sensitive.
 * 3. **A fail-soft state machine** — every bridge answer is folded into
 *    [AwtSurfaceUiState]; an error becomes a visible message instead of an
 *    exception, so a missing native library or a misbehaving JVM-side bridge can
 *    never take the UI down (task 19).
 *
 * The bridge is injected with a default so `viewModel()` can instantiate it and
 * tests can pass a [com.rc.launcher.ui.awt.FakeAwtCanvasBridge] (mirrors
 * [SettingsViewModel] / [ControlLayoutViewModel]).
 */
class AwtSurfaceViewModel(
    private val bridge: AwtCanvasBridge = AwtBridges.default,
) : ViewModel() {

    private val _state = MutableStateFlow(AwtSurfaceUiState(info = runCatching { bridge.info() }.getOrDefault(AwtSessionInfo.CLOSED)))
    val state: StateFlow<AwtSurfaceUiState> = _state.asStateFlow()

    private val pending = ArrayList<AwtInputEvent>()
    private var buffer: ByteBuffer? = null

    /** The RGBA framebuffer the composable uploads (null until a session is open). */
    val frameBuffer: ByteBuffer? get() = buffer

    // ---- Session lifecycle --------------------------------------------------

    /**
     * Open (or replace) the session for a `screenWidth x screenHeight` desktop.
     *
     * [transportDir] names the directory for the named-pipe channels; pass the
     * same path to the launch engine (`LaunchOptions.awt_transport_dir`) so the
     * game JVM finds them. `null` keeps the canvas off-line (self-test only).
     */
    fun open(
        screenWidth: Int = DEFAULT_SCREEN_WIDTH,
        screenHeight: Int = DEFAULT_SCREEN_HEIGHT,
        javaVersion: String? = null,
        transportDir: String? = null,
    ) {
        val current = _state.value
        val config = AwtSessionConfig(
            screenWidth = screenWidth.coerceIn(1, AwtWire.MAX_CANVAS_DIM),
            screenHeight = screenHeight.coerceIn(1, AwtWire.MAX_CANVAS_DIM),
            surfaceWidth = if (current.surfaceWidth > 0) current.surfaceWidth else screenWidth,
            surfaceHeight = if (current.surfaceHeight > 0) current.surfaceHeight else screenHeight,
            scaleMode = current.info.scaleMode,
            javaVersion = javaVersion,
            transportDir = transportDir,
        )
        pending.clear()
        applyInfo(runCatching { bridge.open(config) }.getOrElse { AwtSessionInfo.failed(reason(it)) })
    }

    /** Close the session, release the framebuffer and stop the pumps. */
    fun close() {
        pending.clear()
        runCatching { bridge.close() }
        buffer = null
        _state.value = AwtSurfaceUiState(
            surfaceWidth = _state.value.surfaceWidth,
            surfaceHeight = _state.value.surfaceHeight,
            message = null,
        )
    }

    /** Refresh the snapshot (diagnostics card, link state). */
    fun refresh() {
        applyInfo(runCatching { bridge.info() }.getOrElse { AwtSessionInfo.failed(reason(it)) })
    }

    /** Create + pump the named-pipe channels of the running session. */
    fun attachTransport(dir: String) {
        applyInfo(runCatching { bridge.attachTransport(dir) }.getOrElse { AwtSessionInfo.failed(reason(it)) })
    }

    // ---- Geometry ----------------------------------------------------------

    /**
     * The Compose surface changed size (rotation, split screen, …). The desktop
     * keeps its own resolution: only the letterboxing changes, so no frame is
     * lost on a rotation.
     */
    fun onSurfaceSizeChanged(width: Int, height: Int) {
        val w = width.coerceAtLeast(0)
        val h = height.coerceAtLeast(0)
        val current = _state.value
        if (current.surfaceWidth == w && current.surfaceHeight == h) return
        _state.value = current.copy(surfaceWidth = w, surfaceHeight = h)
        if (!current.info.open || w == 0 || h == 0) return
        configure(AwtConfigureRequest(surfaceWidth = w, surfaceHeight = h))
    }

    /** Change the fitting policy (stretch / fit / crop / 1:1). */
    fun setScaleMode(mode: AwtScaleMode) {
        configure(AwtConfigureRequest(scaleMode = mode))
    }

    /** Resize the *virtual desktop* (reallocates the framebuffer). */
    fun resizeDesktop(width: Int, height: Int) {
        configure(
            AwtConfigureRequest(
                screenWidth = width.coerceIn(1, AwtWire.MAX_CANVAS_DIM),
                screenHeight = height.coerceIn(1, AwtWire.MAX_CANVAS_DIM),
            ),
        )
    }

    /** Repaint the whole desktop (`argb == null` → opaque black). */
    fun repaint(argb: Int? = null) {
        configure(AwtConfigureRequest(clear = argb == null, fillArgb = argb))
    }

    private fun configure(request: AwtConfigureRequest) {
        if (!_state.value.info.open) return
        applyInfo(runCatching { bridge.configure(request) }.getOrElse { AwtSessionInfo.failed(reason(it)) })
    }

    // ---- Pixels ------------------------------------------------------------

    /**
     * Refresh the framebuffer with whatever changed. Call once per frame.
     *
     * Returns [AwtFrameUpdate.NONE] when nothing changed, so the caller skips the
     * bitmap upload *and* the recomposition — which is what keeps a blinking
     * Swing caret from costing a full-screen blit at 60 fps.
     */
    fun poll(): AwtFrameUpdate {
        flushInput()
        val target = buffer ?: return AwtFrameUpdate.NONE
        val update = runCatching { bridge.poll(target) }
            .getOrElse { AwtFrameUpdate(false, error = reason(it)) }
        val current = _state.value
        _state.value = when {
            update.error != null -> current.copy(
                lastUpdate = update,
                skipped = current.skipped + 1,
                message = update.error,
            )
            update.changed -> current.copy(
                lastUpdate = update,
                uploads = current.uploads + 1,
                generation = current.generation + 1,
            )
            else -> current.copy(lastUpdate = update, skipped = current.skipped + 1)
        }
        return update
    }

    /** Push a locally generated test pattern through the whole pipeline. */
    fun submitTestPattern() {
        val info = _state.value.info
        if (!info.open || info.screenWidth <= 0) {
            _state.value = _state.value.copy(message = "请先开启 AWT 会话")
            return
        }
        val update = runCatching {
            val pattern = AwtWire.testPattern(info.screenWidth, info.screenHeight)
            bridge.submitFrame(
                AwtWire.encodeFrame(
                    seq = (_state.value.uploads + 1).toInt(),
                    width = info.screenWidth,
                    height = info.screenHeight,
                    pixels = pattern,
                ),
            )
        }.getOrElse { AwtFrameUpdate(false, error = reason(it)) }
        _state.value = _state.value.copy(
            lastUpdate = update,
            message = update.error ?: "已提交自检帧（${info.screenWidth}x${info.screenHeight}）",
        )
    }

    // ---- Input -------------------------------------------------------------

    /** Queue a pointer sample in **surface** pixels. */
    fun onPointer(
        phase: AwtPointerPhase,
        x: Float,
        y: Float,
        button: AwtMouseButton = AwtMouseButton.LEFT,
    ) {
        enqueue(AwtPointerEvent(phase, x, y, button))
        // A press / release must not wait for the next frame.
        if (phase != AwtPointerPhase.MOVE) flushInput()
    }

    /** Queue a scroll gesture in surface pixels. */
    fun onScroll(x: Float, y: Float, ticks: Int) {
        if (ticks == 0) return
        enqueue(AwtScrollEvent(x, y, ticks))
    }

    /** Press / release a key by `KeyEvent.VK_*` code (flushed immediately). */
    fun onKey(down: Boolean, code: Int) = sendNow(AwtKeyEvent(down = down, code = code))

    /** Press / release a key by name (`"escape"`, `"key.keyboard.w"`, …). */
    fun onKeyNamed(down: Boolean, name: String) {
        if (name.isBlank()) return
        sendNow(AwtKeyEvent(down = down, name = name))
    }

    /** Commit text from the soft keyboard / IME. */
    fun onText(text: String) {
        if (text.isEmpty()) return
        sendNow(AwtTextEvent(text))
    }

    /** The canvas gained / lost focus (losing it releases everything held). */
    fun onFocusChanged(gained: Boolean) = sendNow(AwtFocusEvent(gained))

    /** Release every held button / modifier (app went to the background). */
    fun releaseAll() = sendNow(AwtReleaseAllEvent)

    /** Hand the queued input to the core (called once per frame by [poll]). */
    fun flushInput(): AwtInputResult {
        if (pending.isEmpty()) return AwtInputResult.EMPTY
        val batch = ArrayList<AwtInputEvent>(pending)
        pending.clear()
        val result = runCatching { bridge.input(batch) }
            .getOrElse { AwtInputResult(error = reason(it)) }
        val current = _state.value
        // The core answers with the state that *matters* for the next frame
        // (focus, queue depth, held modifiers), so fold it in instead of paying
        // for a separate `awtInfo` round trip.
        val info = if (result.error == null && current.info.open) {
            current.info.copy(
                focused = result.focused,
                pendingEvents = result.pending,
                modifiers = result.modifiers,
            )
        } else {
            current.info
        }
        _state.value = current.copy(
            info = info,
            lastInput = result,
            message = result.error ?: current.message,
        )
        return result
    }

    private fun enqueue(event: AwtInputEvent) {
        if (!_state.value.info.open) return
        pending.add(event)
        // Never let a stalled poll loop grow the queue without bound.
        if (pending.size >= MAX_PENDING_INPUT) flushInput()
    }

    private fun sendNow(event: AwtInputEvent) {
        enqueue(event)
        flushInput()
    }

    /** Clear the transient message shown in the diagnostics card. */
    fun clearMessage() {
        _state.value = _state.value.copy(message = null)
    }

    // ---- Internals ---------------------------------------------------------

    private fun applyInfo(info: AwtSessionInfo) {
        val current = _state.value
        if (info.open && info.rgbaLen > 0 && (buffer?.capacity() ?: -1) != info.rgbaLen) {
            buffer = ByteBuffer.allocateDirect(info.rgbaLen).order(ByteOrder.nativeOrder())
        }
        if (!info.open) buffer = null
        _state.value = current.copy(
            info = info,
            surfaceWidth = if (info.surfaceWidth > 0) info.surfaceWidth else current.surfaceWidth,
            surfaceHeight = if (info.surfaceHeight > 0) info.surfaceHeight else current.surfaceHeight,
            message = info.error ?: current.message,
            // A new / resized desktop invalidates whatever the bitmap held.
            generation = current.generation + 1,
        )
    }

    private fun reason(t: Throwable): String =
        "AWT 桥接失败：" + (t.message ?: t.javaClass.simpleName)

    companion object {
        /** Default virtual desktop: 720p, like `-Dcacio.managed.screensize`. */
        const val DEFAULT_SCREEN_WIDTH = 1280
        const val DEFAULT_SCREEN_HEIGHT = 720

        /** Hard cap for un-flushed input samples (one flush per frame is normal). */
        const val MAX_PENDING_INPUT = 64
    }
}

/** Everything the AWT canvas UI renders from. */
data class AwtSurfaceUiState(
    val info: AwtSessionInfo = AwtSessionInfo.CLOSED,
    /** Size of the Compose surface, in pixels (0 until the first layout). */
    val surfaceWidth: Int = 0,
    val surfaceHeight: Int = 0,
    /** Result of the most recent [AwtSurfaceViewModel.poll]. */
    val lastUpdate: AwtFrameUpdate = AwtFrameUpdate.NONE,
    /** Bumped whenever the framebuffer changed (drives the bitmap upload). */
    val generation: Long = 0,
    /** Frames uploaded into the bitmap. */
    val uploads: Long = 0,
    /** Polls that found nothing to upload (the cheap, common case). */
    val skipped: Long = 0,
    val lastInput: AwtInputResult = AwtInputResult.EMPTY,
    /** Transient error / notice for the diagnostics card. */
    val message: String? = null,
) {
    val open: Boolean get() = info.open

    /** Where the desktop is drawn inside the surface, recomputed locally. */
    val placement
        get() = AwtViewportHolder.viewport(info, surfaceWidth, surfaceHeight).placement()

    /** The viewport used to map touches (surface → desktop pixels). */
    val viewport get() = AwtViewportHolder.viewport(info, surfaceWidth, surfaceHeight)
}

/**
 * Builds the viewport from the session snapshot and the *current* surface size.
 *
 * The core reports the placement it knows about, but the surface can change a
 * frame before the core hears about it (a rotation), so the UI recomputes it from
 * the size it is actually drawing with — identical integer math, no drift.
 */
private object AwtViewportHolder {
    fun viewport(info: AwtSessionInfo, surfaceWidth: Int, surfaceHeight: Int) =
        com.rc.launcher.ui.awt.AwtViewport(
            screenWidth = info.screenWidth,
            screenHeight = info.screenHeight,
            surfaceWidth = if (surfaceWidth > 0) surfaceWidth else info.surfaceWidth,
            surfaceHeight = if (surfaceHeight > 0) surfaceHeight else info.surfaceHeight,
            mode = info.scaleMode,
        )
}
