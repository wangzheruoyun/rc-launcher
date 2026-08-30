package com.rc.launcher.ui.viewmodel

import androidx.lifecycle.ViewModel
import com.rc.launcher.ui.awt.AwtBridges
import com.rc.launcher.ui.awt.AwtCanvasBridge
import com.rc.launcher.ui.awt.AwtConfigureRequest
import com.rc.launcher.ui.awt.AwtControlBatch
import com.rc.launcher.ui.awt.AwtControlRequest
import com.rc.launcher.ui.awt.AwtControlResult
import com.rc.launcher.ui.awt.AwtControlState
import com.rc.launcher.ui.awt.AwtCursorKind
import com.rc.launcher.ui.awt.AwtButtonEvent
import com.rc.launcher.ui.awt.AwtCaptureEvent
import com.rc.launcher.ui.awt.AwtFocusEvent
import com.rc.launcher.ui.awt.AwtGameInputState
import com.rc.launcher.ui.awt.AwtInputSettings
import com.rc.launcher.ui.awt.AwtKeyBindings
import com.rc.launcher.ui.awt.AwtMouseSensitivity
import com.rc.launcher.ui.awt.AwtPointerMode
import com.rc.launcher.ui.awt.AwtPointerSource
import com.rc.launcher.ui.awt.AwtPoint
import com.rc.launcher.ui.awt.AwtRelativePointerEvent
import com.rc.launcher.ui.awt.AwtScrollAtPointerEvent
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
import com.rc.launcher.ui.ScreenOrientation
import com.rc.launcher.ui.rotationFlips
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
        /** Physical keyboard / mouse settings to start with (task 12). */
        input: AwtInputSettings? = null,
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
            // A fresh session starts from the player's saved settings, so the
            // first mouse sample already has the right sensitivity.
            input = input ?: current.info.input,
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
        // A closed session has no cursor, no title and wants no keyboard.
        // Leaving the old projection up would keep the soft keyboard open over a
        // canvas that no longer exists.
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
     *
     * **Rotation safety (task 9).** Pointer samples are batched and flushed once
     * per frame, and the *core* maps them from surface pixels to desktop pixels
     * with whatever viewport it currently has. A rotation between the sample and
     * the flush would therefore map old coordinates through the new letterboxing
     * — a tap that visibly lands somewhere else. Two things prevent that:
     *
     * 1. queued samples are flushed **before** the new geometry is published, so
     *    every sample is mapped with the viewport it was taken in;
     * 2. when the resize is a real quarter turn ([rotationFlips]) the in-flight
     *    gesture is dropped afterwards ([releaseAll]) — the finger is physically
     *    somewhere else now, so continuing a drag/press would be wrong (and could
     *    leave a button stuck). Resizes *inside* one orientation (soft keyboard,
     *    split screen, foldable hinge) keep the gesture alive.
     */
    fun onSurfaceSizeChanged(width: Int, height: Int) {
        val w = width.coerceAtLeast(0)
        val h = height.coerceAtLeast(0)
        val current = _state.value
        if (current.surfaceWidth == w && current.surfaceHeight == h) return
        val rotated = rotationFlips(current.surfaceWidth, current.surfaceHeight, w, h)
        // (1) Hand over everything sampled against the old viewport first.
        if (current.info.open && pending.isNotEmpty()) flushInput()
        _state.value = _state.value.copy(
            surfaceWidth = w,
            surfaceHeight = h,
            rotations = _state.value.rotations + if (rotated) 1 else 0,
        )
        if (!current.info.open || w == 0 || h == 0) return
        configure(AwtConfigureRequest(surfaceWidth = w, surfaceHeight = h))
        // (2) A quarter turn invalidates the gesture that is in flight.
        if (rotated) releaseAll()
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
        /** Which device produced it (task 12): the core filters on this. */
        source: AwtPointerSource = AwtPointerSource.TOUCH,
    ) {
        enqueue(AwtPointerEvent(phase, x, y, button, source))
        // A press / release must not wait for the next frame.
        if (phase != AwtPointerPhase.MOVE) flushInput()
    }

    // ---- Physical keyboard & mouse (task 12) --------------------------------

    /**
     * Queue *relative* motion from a physical mouse.
     *
     * Batched like any other motion (one JNI call per frame): a 1000 Hz gaming
     * mouse would otherwise cross the JNI boundary 16 times per rendered frame.
     * The core accumulates the sub-pixel remainder, so batching loses nothing.
     */
    fun onMouseRelative(
        dx: Float,
        dy: Float,
        source: AwtPointerSource = AwtPointerSource.MOUSE,
    ) {
        if (dx == 0f && dy == 0f) return
        if (!dx.isFinite() || !dy.isFinite()) return
        enqueue(AwtRelativePointerEvent(dx, dy, source))
    }

    /** Hand a whole batch (what [com.rc.launcher.ui.awt.AwtMouseTracker] produced). */
    fun onInputEvents(events: List<AwtInputEvent>) {
        if (events.isEmpty()) return
        events.forEach { enqueue(it) }
        // Anything that is not pure motion is latency-sensitive (a click, a
        // capture change), so do not make it wait for the frame boundary.
        if (events.any { it !is AwtRelativePointerEvent && it !is AwtPointerEvent }) flushInput()
    }

    /**
     * Capture or release the pointer.
     *
     * Sent as an *input* event, not as a configure call, so it stays ordered with
     * the gesture stream: a click queued before the capture must be delivered
     * before the game is told it owns the cursor.
     */
    fun setPointerCapture(captured: Boolean) {
        if (_state.value.captured == captured) return
        sendNow(AwtCaptureEvent(captured))
    }

    /**
     * A button press / release from a **captured** mouse.
     *
     * Flushed immediately: a click that waits for the frame boundary is a click
     * that feels broken, and there is exactly one per press.
     */
    fun onCapturedButton(button: AwtMouseButton, down: Boolean) =
        sendNow(AwtButtonEvent(button, down))

    /**
     * A wheel scroll from a captured mouse, in notches (positive = toward the
     * user, as AWT and Compose count it).
     *
     * Fractional notches from a high-resolution wheel are rounded *away from
     * zero*, so a small flick still switches one hotbar slot instead of nothing.
     */
    fun onCapturedScroll(ticks: Float) {
        if (!ticks.isFinite() || ticks == 0f) return
        val whole = if (ticks > 0f) kotlin.math.ceil(ticks).toInt() else kotlin.math.floor(ticks).toInt()
        enqueue(AwtScrollAtPointerEvent(whole))
    }

    /** Toggle the capture (what the HUD button and a middle-click do). */
    fun togglePointerCapture() = setPointerCapture(!_state.value.captured)

    /** Switch pointer mode (absolute cursor ⇄ captured pointer). */
    fun setPointerMode(mode: AwtPointerMode) = setPointerCapture(mode.isCaptured)

    /** Replace the physical-input settings (sensitivity, hybrid touch, remaps). */
    fun setInputSettings(settings: AwtInputSettings) {
        configure(AwtConfigureRequest(input = settings.sanitized()))
    }

    /** Edit the settings in force, e.g. `updateInput { it.copy(hybridTouch = false) }`. */
    fun updateInput(edit: (AwtInputSettings) -> AwtInputSettings) {
        setInputSettings(edit(_state.value.inputSettings))
    }

    /** Set one sensitivity factor for both axes (the simple slider). */
    fun setSensitivity(factor: Float) =
        updateInput { it.copy(sensitivity = it.sensitivity.withUniform(factor)) }

    /** Invert the vertical axis. */
    fun setInvertY(invert: Boolean) =
        updateInput { it.copy(sensitivity = it.sensitivity.copy(invertY = invert)) }

    /** Accept touch while the pointer is captured (mixed touch + mouse). */
    fun setHybridTouch(enabled: Boolean) = updateInput { it.copy(hybridTouch = enabled) }

    /** Feed the game's own input queue as well as AWT's (normally on). */
    fun setNativeInput(enabled: Boolean) = updateInput { it.copy(nativeInput = enabled) }

    /** Wheel scaling, in per-mille (1000 = one notch per notch). */
    fun setScrollPermille(permille: Int) = updateInput { it.copy(scrollPermille = permille) }

    /** Remap one key onto another (both in the task-15 key vocabulary). */
    fun bindKey(from: String, to: String) =
        updateInput { it.copy(bindings = it.bindings.withKey(from, to)) }

    /** Drop one key remap. */
    fun unbindKey(from: String) =
        updateInput { it.copy(bindings = it.bindings.withoutKey(from)) }

    /** Remap one mouse button onto another (left-handed use). */
    fun bindButton(from: AwtMouseButton, to: AwtMouseButton) =
        updateInput { it.copy(bindings = it.bindings.withButton(from, to)) }

    /** Forget every remap. */
    fun clearBindings() = updateInput { it.copy(bindings = AwtKeyBindings.EMPTY) }

    /** Queue a scroll gesture in surface pixels. */
    fun onScroll(x: Float, y: Float, ticks: Int) {
        if (ticks == 0) return
        enqueue(AwtScrollEvent(x, y, ticks))
    }

    /** Press / release a key by `KeyEvent.VK_*` code (flushed immediately). */
    fun onKey(down: Boolean, code: Int) = sendNow(AwtKeyEvent(down = down, code = code))

    /**
     * Press / release a key by name (`"escape"`, `"key.keyboard.w"`, …).
     *
     * [scancode] is the *physical* code Android reported
     * (`KeyEvent.getScanCode()`, i.e. the Linux evdev number). Forwarding it is
     * what makes a non-US layout and the extra keys of a real keyboard work in
     * the game, because `GLFWKeyCallback` takes it and Minecraft falls back to it
     * for anything its key enumeration does not cover (task 12). `null` lets the
     * core fill it in from its table.
     */
    fun onKeyNamed(down: Boolean, name: String, scancode: Int? = null) {
        if (name.isBlank()) return
        sendNow(AwtKeyEvent(down = down, name = name, scancode = scancode))
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
                // Task 12: a `capture` event changes the mode inside the batch, so
                // the answer is the authority — without this the UI would keep
                // requesting Android pointer capture for a released pointer.
                input = current.info.input.copy(pointerMode = result.pointerMode),
                gameInput = current.info.gameInput.copy(
                    cursorX = result.gameCursor.x,
                    cursorY = result.gameCursor.y,
                    grabbed = result.captured,
                ),
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

    // ---- Control plane ------------------------------------------------------

    /**
     * Drain the control plane once per frame (right after [poll]).
     *
     * The *projection* (cursor shape, window title, whether a keyboard is wanted)
     * is folded into [AwtSurfaceUiState.control] for rendering; the returned batch
     * carries the side effects the composable has to perform with Android APIs
     * the ViewModel deliberately does not touch — pushing text onto the system
     * clipboard, answering a paste, showing the soft keyboard, a haptic tick.
     */
    fun pumpControl(): AwtControlBatch {
        if (!_state.value.info.open) return AwtControlBatch.EMPTY
        val batch = runCatching { bridge.drainControl() }
            .getOrElse { AwtControlBatch.failed(reason(it)) }
        val current = _state.value
        _state.value = current.copy(
            control = if (batch.error == null) batch.state else current.control,
            lastControl = batch,
            controlMessages = current.controlMessages + batch.messages.size,
            message = batch.error ?: current.message,
        )
        // Task 12: you cannot type into a Swing text field with the pointer
        // captured — there is no cursor and no soft keyboard. When the JVM says a
        // text component has focus, give the pointer back.
        if (batch.error == null && batch.state.wantsPointerReleased && _state.value.captured) {
            setPointerCapture(false)
        }
        return batch
    }

    /**
     * Answer a `Clipboard.getContents()` with the Android clipboard.
     *
     * `null` answers "there is no text" — which is still an *answer*: a Swing
     * thread blocked in `getContents()` has to be released either way.
     */
    fun answerClipboard(text: String?, seq: Int? = null): AwtControlResult {
        val request = AwtControlRequest(
            clipboard = text,
            clipboardEmpty = text == null,
            clipboardSeq = seq,
        )
        val result = runCatching { bridge.control(request) }
            .getOrElse { AwtControlResult(error = reason(it)) }
        val current = _state.value
        _state.value = current.copy(
            control = if (result.error == null) result.state else current.control,
            message = result.error ?: current.message,
        )
        return result
    }

    /** Forget the control projection (arrow cursor, no keyboard). */
    fun resetControl(): AwtControlResult {
        val result = runCatching { bridge.control(AwtControlRequest(reset = true)) }
            .getOrElse { AwtControlResult(error = reason(it)) }
        _state.value = _state.value.copy(
            control = if (result.error == null) result.state else AwtControlState.EMPTY,
        )
        return result
    }

    /** Inject one encoded `RCAC` control message (diagnostics self-test). */
    fun submitControl(message: ByteArray): Boolean =
        runCatching { bridge.submitControl(message) }.getOrDefault(false)

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
    /** Live control projection (cursor / title / IME / clipboard, task 18). */
    val control: AwtControlState = AwtControlState.EMPTY,
    /** Result of the most recent [AwtSurfaceViewModel.pumpControl]. */
    val lastControl: AwtControlBatch = AwtControlBatch.EMPTY,
    /** Control messages seen so far (diagnostics). */
    val controlMessages: Long = 0,
    /**
     * How often the surface really rotated (landscape ⇄ portrait, task 9). Each
     * of those released the in-flight gesture; mirrors the core's
     * `SessionStats::surface_rotations`.
     */
    val rotations: Long = 0,
    /** Transient error / notice for the diagnostics card. */
    val message: String? = null,
) {
    val open: Boolean get() = info.open

    /** Pointer shape the JVM asked for (what the overlay draws). */
    val cursor: AwtCursorKind get() = control.cursor

    /** Title of the active AWT window / dialog, if the bridge reported one. */
    val title: String? get() = control.title

    /** Whether a Swing text component is focused and wants the soft keyboard. */
    val wantsKeyboard: Boolean get() = open && control.wantsKeyboard

    /**
     * The Swing caret, mapped into *surface* pixels through the same viewport the
     * pixels use — so the IME anchor cannot drift onto the letterbox bars.
     */
    val caretOnSurface: Pair<Float, Float>?
        get() = control.caret?.let { viewport.mapToSurface(it.x, it.y) }

    /** Where the desktop is drawn inside the surface, recomputed locally. */
    val placement
        get() = AwtViewportHolder.viewport(info, surfaceWidth, surfaceHeight).placement()

    /** The viewport used to map touches (surface → desktop pixels). */
    val viewport get() = AwtViewportHolder.viewport(info, surfaceWidth, surfaceHeight)

    /** Orientation of the Compose surface (task 9 diagnostics). */
    val surfaceOrientation: ScreenOrientation
        get() = ScreenOrientation.of(surfaceWidth, surfaceHeight)

    /** Physical keyboard / mouse settings in force (task 12). */
    val inputSettings: AwtInputSettings get() = info.input

    /**
     * Whether the pointer is captured (task 12).
     *
     * Read from the *last input answer* when there is one and from the snapshot
     * otherwise: the answer is one frame fresher, and the composable uses this to
     * decide whether to hold Android's pointer capture and hide its own overlay.
     */
    val captured: Boolean
        get() = open && if (lastInput === AwtInputResult.EMPTY) info.captured else lastInput.captured

    /** Sensitivity in force (what the settings slider shows). */
    val sensitivity: AwtMouseSensitivity get() = info.input.sensitivity

    /** What the game's own input queue believes (diagnostics). */
    val gameInput: AwtGameInputState get() = info.gameInput

    /** The game's cursor, in game-window pixels (free-running while captured). */
    val gameCursor: AwtPoint
        get() = if (lastInput === AwtInputResult.EMPTY) {
            AwtPoint(info.gameInput.cursorX, info.gameInput.cursorY)
        } else {
            lastInput.gameCursor
        }
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
