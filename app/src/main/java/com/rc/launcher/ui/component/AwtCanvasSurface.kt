package com.rc.launcher.ui.component

import android.graphics.Bitmap
import android.os.Build
import android.view.MotionEvent
import android.view.View
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.focusable
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.nativeKeyCode
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.key.utf16CodePoint
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.rc.launcher.ui.awt.AwtCursorKind
import com.rc.launcher.ui.awt.AwtMouseButton
import com.rc.launcher.ui.awt.AwtMouseTracker
import com.rc.launcher.ui.awt.AwtPointerPhase
import com.rc.launcher.ui.awt.AwtPointerSource
import com.rc.launcher.ui.awt.awtScancodeForAndroidKeyCode
import com.rc.launcher.ui.awt.gameKeyNameForAndroidKeyCode
import com.rc.launcher.ui.viewmodel.AwtSurfaceViewModel

/**
 * The Compose surface that shows Minecraft's embedded AWT/Swing UI (task 18).
 *
 * ```text
 *   game JVM (caciocavallo peers)  ──frames──▶  Rust core (canvas + damage)
 *                                                   │ awtPollFrame(direct buffer)
 *                                                   ▼
 *                        this composable:  ByteBuffer ─▶ Bitmap ─▶ drawImage
 *                                                   ▲
 *   AWT event queue  ◀──records──  Rust core  ◀── touches / keys / IME text
 * ```
 *
 * What it does, and why:
 *
 * * **One upload per changed frame.** A `withFrameNanos` loop polls the core once
 *   per vsync; the core answers "nothing changed" for an idle desktop, in which
 *   case neither the bitmap nor the canvas is touched (a blinking Swing caret
 *   must not cost a full-screen blit).
 * * **Letterboxing.** The desktop keeps its aspect ratio through
 *   [AwtSurfaceViewModel.state]'s viewport, and the same integer math maps a
 *   touch back to a desktop pixel — so a tap always lands on the Swing button
 *   under the finger, and a tap on the black bars is not forwarded at all.
 * * **Focus & keys.** The surface is focusable: gaining focus tells AWT, losing it
 *   releases every held button/modifier (nothing stays stuck when the app goes to
 *   the background). Hardware keys are forwarded by name, and anything without an
 *   AWT code degrades to typed text (IME included).
 * * **The control plane.** Everything that crosses the bridge but is not a pixel
 *   is drained on the same frame loop, because most of it repaints *nothing*: the
 *   cursor shape the JVM asked for is drawn as a pointer overlay, a
 *   `Clipboard.setContents` lands on the Android clipboard, a
 *   `Clipboard.getContents` is **answered** with it (a Swing thread may be
 *   blocked on that call), a focused text field pops the soft keyboard, and
 *   `Toolkit.beep()` becomes a haptic tick.
 */
@OptIn(ExperimentalComposeUiApi::class)
@Composable
fun AwtCanvasSurface(
    viewModel: AwtSurfaceViewModel,
    modifier: Modifier = Modifier,
    /** Button a touch acts as (a UI toggle can offer "right click" mode). */
    touchButton: AwtMouseButton = AwtMouseButton.LEFT,
    /** Draw a hint while no session is open. */
    placeholder: String = "AWT 会话未开启",
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val focusRequester = remember { FocusRequester() }
    // Android-side services the ViewModel deliberately does not know about.
    val clipboard = LocalClipboardManager.current
    val haptics = LocalHapticFeedback.current
    val keyboard = LocalSoftwareKeyboardController.current

    val screenWidth = state.info.screenWidth
    val screenHeight = state.info.screenHeight

    // One bitmap per desktop size; recreated (not resized) when cacio changes it.
    val bitmap: Bitmap? = remember(screenWidth, screenHeight, state.open) {
        if (state.open && screenWidth > 0 && screenHeight > 0) {
            Bitmap.createBitmap(screenWidth, screenHeight, Bitmap.Config.ARGB_8888)
        } else {
            null
        }
    }
    val image: ImageBitmap? = remember(bitmap) { bitmap?.asImageBitmap() }
    // Bumped after every successful upload; read inside the draw lambda so the
    // canvas repaints exactly when the pixels changed.
    var frameTick by remember(bitmap) { mutableStateOf(0L) }

    // The frame pump: poll -> upload -> redraw. Restarted whenever the session or
    // the desktop size changes; cancelled with the composition.
    LaunchedEffect(state.open, bitmap) {
        if (!state.open || bitmap == null) return@LaunchedEffect
        while (true) {
            withFrameNanos { }
            val update = viewModel.poll()
            // *Before* the `continue` below: a cursor change, a clipboard
            // hand-off or a paste request repaints nothing at all, so tying the
            // control plane to a changed frame would stall it on an idle desktop
            // (and hang a JVM thread blocked in `getContents()`).
            val control = viewModel.pumpControl()
            if (control.messages.isNotEmpty()) {
                control.clipboardSet?.let { text ->
                    runCatching { clipboard.setText(AnnotatedString(text)) }
                }
                control.clipboardRequestSeq?.let { seq ->
                    // `null` is still an answer: never leave the JVM waiting.
                    val text = runCatching { clipboard.getText()?.text }.getOrNull()
                    viewModel.answerClipboard(text, seq)
                }
                if (control.beeps > 0) {
                    runCatching { haptics.performHapticFeedback(HapticFeedbackType.LongPress) }
                }
            }
            if (!update.changed) continue
            val buffer = viewModel.frameBuffer ?: continue
            // `copyPixelsFromBuffer` reads from the buffer's position; the core
            // wrote the whole framebuffer in place, so rewind before each upload.
            buffer.rewind()
            runCatching { bitmap.copyPixelsFromBuffer(buffer) }
            frameTick++
        }
    }

    // Losing the composition must not leave a button held down inside AWT.
    DisposableEffect(Unit) {
        onDispose { viewModel.releaseAll() }
    }

    Box(
        modifier = modifier
            .background(Color.Black)
            .onSizeChanged { viewModel.onSurfaceSizeChanged(it.width, it.height) }
            .focusRequester(focusRequester)
            .focusable()
            .onFocusChanged { viewModel.onFocusChanged(it.isFocused) }
            .onKeyEvent { event ->
                if (!state.open) return@onKeyEvent false
                val keyCode = event.key.nativeKeyCode
                // Task 12: the *side-aware* name (GLFW tells left from right shift,
                // AWT cannot) and the physical scancode Android reported. The
                // scancode is what `GLFWKeyCallback` needs and what Minecraft falls
                // back to for every key its enumeration does not cover, which is
                // what makes a non-US layout work; `scanCode` is 0 for a synthetic
                // key, and the core then fills it in from its own table.
                val name = gameKeyNameForAndroidKeyCode(keyCode)
                val scancode = event.nativeKeyEvent.scanCode
                    .takeIf { it > 0 }
                    ?: awtScancodeForAndroidKeyCode(keyCode).takeIf { it > 0 }
                when (event.type) {
                    KeyEventType.KeyDown -> {
                        if (name != null) {
                            viewModel.onKeyNamed(true, name, scancode)
                            // A printable key also has to *type*: the game's chat
                            // and every Swing text field react to the character,
                            // not to the key. The core keeps the two apart.
                            val ch = event.utf16CodePoint
                            if (ch != 0 && !Character.isISOControl(ch)) {
                                viewModel.onText(String(Character.toChars(ch)))
                            }
                        } else if (event.utf16CodePoint != 0) {
                            viewModel.onText(String(Character.toChars(event.utf16CodePoint)))
                        }
                        name != null || event.utf16CodePoint != 0
                    }
                    KeyEventType.KeyUp -> {
                        if (name != null) viewModel.onKeyNamed(false, name, scancode)
                        name != null
                    }
                    else -> false
                }
            }
            // Task 9: keying the gesture detector on the surface orientation makes
            // Compose restart it after a rotation, so a drag that started before
            // the quarter turn cannot continue with stale coordinates (the core
            // released the held buttons for the same reason).
            .pointerInput(state.open, touchButton, state.surfaceOrientation) {
                if (!state.open) return@pointerInput
                awaitPointerEventScope {
                    while (true) {
                        val event = awaitPointerEvent()
                        val change = event.changes.firstOrNull() ?: continue
                        val source = awtSourceOf(change.type)
                        if (event.type == PointerEventType.Scroll) {
                            // Compose reports "scroll down" as +1, the sign AWT
                            // uses; the core flips it for GLFW.
                            val ticks = change.scrollDelta.y
                            viewModel.onScroll(
                                change.position.x,
                                change.position.y,
                                if (ticks > 0f) {
                                    kotlin.math.ceil(ticks).toInt()
                                } else {
                                    kotlin.math.floor(ticks).toInt()
                                },
                            )
                            change.consume()
                            continue
                        }
                        // Task 12: which physical button is held. A mouse reports
                        // primary / secondary / tertiary; a finger acts as
                        // [touchButton] (which a UI toggle can set to "right").
                        val button = when {
                            source == AwtPointerSource.TOUCH -> touchButton
                            event.buttons.isSecondaryPressed -> AwtMouseButton.RIGHT
                            event.buttons.isTertiaryPressed -> AwtMouseButton.MIDDLE
                            event.buttons.isPrimaryPressed -> AwtMouseButton.LEFT
                            else -> touchButton
                        }
                        val phase = when {
                            change.pressed && !change.previousPressed -> AwtPointerPhase.DOWN
                            !change.pressed && change.previousPressed -> AwtPointerPhase.UP
                            // A mouse hovers: report the motion even with no
                            // button held, or Swing would never see a `mouseMoved`
                            // (no tool-tips, no hover highlight, no I-beam).
                            change.pressed || source != AwtPointerSource.TOUCH ->
                                AwtPointerPhase.MOVE
                            else -> null
                        }
                        if (phase != null) {
                            viewModel.onPointer(
                                phase,
                                change.position.x,
                                change.position.y,
                                button,
                                source,
                            )
                            change.consume()
                        }
                    }
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        if (image == null) {
            Text(placeholder, color = Color.White, style = MaterialTheme.typography.bodyMedium)
        } else {
            val placement = state.placement
            val cursor = state.cursor
            val captured = state.captured
            val pointer = state.lastInput.pointer
            val viewport = state.viewport
            Canvas(modifier = Modifier.fillMaxSize()) {
                // Reading `frameTick` inside the draw lambda is what ties a
                // repaint to a freshly uploaded frame (the Bitmap is mutated in
                // place, so Compose has no other way to know).
                val tick = frameTick
                if (tick >= 0) {
                    drawAwtDesktop(image, placement.x, placement.y, placement.width, placement.height)
                }
                // The pointer overlay: Android has no cursor to hand to a window
                // manager, so the shape the JVM asked for is drawn here. It is the
                // only cue that the thing under the finger is a text field (I-beam)
                // or a link (hand).
                // While the pointer is captured the *game* owns the cursor (it
                // draws its own crosshair), so a second pointer on top would only
                // be confusing — and it would sit at a stale position anyway.
                if (cursor != AwtCursorKind.DEFAULT && !captured) {
                    val (sx, sy) = viewport.mapToSurface(pointer.x, pointer.y)
                    drawAwtCursor(cursor, Offset(sx, sy))
                }
            }
        }
    }

    LaunchedEffect(state.open) {
        if (state.open) runCatching { focusRequester.requestFocus() }
    }

    // Task 12: hold Android's pointer capture while the game owns the cursor, and
    // feed the relative motion it then delivers.
    AwtPointerCaptureEffect(viewModel, enabled = state.open && state.captured)

    // A Swing text component gained / lost focus inside the JVM: offer or retract
    // the soft keyboard. Without this the user would have to guess that the
    // invisible `JTextField` under the finger wants input.
    LaunchedEffect(state.wantsKeyboard) {
        runCatching {
            if (state.wantsKeyboard) keyboard?.show() else keyboard?.hide()
        }
    }
}

/**
 * Draw the AWT cursor shape at [at] (surface pixels).
 *
 * Deliberately vector-drawn instead of bitmap assets: it stays crisp at every
 * density, costs nothing to ship, and cannot fail to load. Every shape is drawn
 * twice (a black outline under a white body) so it stays visible over both a
 * white `JOptionPane` and a dark game frame.
 */
private fun DrawScope.drawAwtCursor(kind: AwtCursorKind, at: Offset) {
    val s = 12f
    val body = Color.White
    val edge = Color.Black
    fun stroke(from: Offset, to: Offset) {
        drawLine(edge, from, to, strokeWidth = 4f, cap = StrokeCap.Round)
        drawLine(body, from, to, strokeWidth = 2f, cap = StrokeCap.Round)
    }
    when (kind) {
        AwtCursorKind.TEXT -> {
            // I-beam.
            stroke(Offset(at.x, at.y - s), Offset(at.x, at.y + s))
            stroke(Offset(at.x - s / 3, at.y - s), Offset(at.x + s / 3, at.y - s))
            stroke(Offset(at.x - s / 3, at.y + s), Offset(at.x + s / 3, at.y + s))
        }
        AwtCursorKind.CROSSHAIR -> {
            stroke(Offset(at.x - s, at.y), Offset(at.x + s, at.y))
            stroke(Offset(at.x, at.y - s), Offset(at.x, at.y + s))
        }
        AwtCursorKind.WAIT -> {
            // Hourglass-ish: a ring is the clearest "busy" hint at this size.
            drawCircle(edge, radius = s * 0.75f, center = at, style = Stroke(width = 4f))
            drawCircle(body, radius = s * 0.75f, center = at, style = Stroke(width = 2f))
            stroke(at, Offset(at.x, at.y - s * 0.75f))
        }
        AwtCursorKind.HAND -> {
            // A pointing finger, reduced to its silhouette.
            drawRoundedCursorBody(at, Size(s * 0.9f, s * 1.3f), edge, body)
            stroke(Offset(at.x, at.y), Offset(at.x, at.y - s))
        }
        AwtCursorKind.MOVE -> {
            stroke(Offset(at.x - s, at.y), Offset(at.x + s, at.y))
            stroke(Offset(at.x, at.y - s), Offset(at.x, at.y + s))
            drawCircle(body, radius = 3f, center = at)
        }
        else -> {
            // Resize grips: a double-headed arrow along the grip direction.
            val (dx, dy) = when (kind) {
                AwtCursorKind.N_RESIZE, AwtCursorKind.S_RESIZE -> 0f to 1f
                AwtCursorKind.W_RESIZE, AwtCursorKind.E_RESIZE -> 1f to 0f
                AwtCursorKind.NE_RESIZE, AwtCursorKind.SW_RESIZE -> 1f to -1f
                else -> 1f to 1f
            }
            stroke(Offset(at.x - s * dx, at.y - s * dy), Offset(at.x + s * dx, at.y + s * dy))
        }
    }
}

private fun DrawScope.drawRoundedCursorBody(at: Offset, size: Size, edge: Color, body: Color) {
    val path = Path().apply {
        moveTo(at.x - size.width / 2, at.y)
        lineTo(at.x + size.width / 2, at.y)
        lineTo(at.x + size.width / 2, at.y + size.height)
        lineTo(at.x - size.width / 2, at.y + size.height)
        close()
    }
    drawPath(path, edge, style = Stroke(width = 4f))
    drawPath(path, body, style = Stroke(width = 2f))
}

/** Blit the whole desktop into the (letterboxed) destination rectangle. */
private fun DrawScope.drawAwtDesktop(
    image: ImageBitmap,
    x: Int,
    y: Int,
    width: Int,
    height: Int,
) {
    if (width <= 0 || height <= 0) return
    drawImage(
        image = image,
        srcOffset = IntOffset.Zero,
        srcSize = IntSize(image.width, image.height),
        dstOffset = IntOffset(x, y),
        dstSize = IntSize(width, height),
    )
}

/** Which device a Compose [PointerType] describes (task 12). */
private fun awtSourceOf(type: PointerType): AwtPointerSource = when (type) {
    PointerType.Mouse -> AwtPointerSource.MOUSE
    PointerType.Stylus, PointerType.Eraser -> AwtPointerSource.STYLUS
    else -> AwtPointerSource.TOUCH
}

/**
 * Holds Android's **pointer capture** while the game owns the cursor (task 12).
 *
 * This is the piece that makes a mouse feel like the desktop game. Without
 * capture Android keeps delivering absolute positions and stops at the edge of
 * the screen, so the view cannot turn further than one swipe; with capture the
 * events carry *relative* motion (`AXIS_RELATIVE_X/Y`) and there is no edge at
 * all. Buttons and the wheel keep arriving on the same channel, so they are
 * forwarded here too — a captured pointer sends nothing through `pointerInput`.
 *
 * Robustness (task 19):
 *
 * * `requestPointerCapture()` needs API 26 and a focused window; below that, or
 *   when the request is refused, the canvas simply keeps working in absolute mode
 *   — the player loses \"unlimited turning\", not the mouse;
 * * every framework call is wrapped, because a manufacturer's `View` may throw;
 * * the capture is released on dispose *and* whenever the flag goes false, so the
 *   pointer can never stay trapped in a screen the player has left.
 */
@Composable
private fun AwtPointerCaptureEffect(viewModel: AwtSurfaceViewModel, enabled: Boolean) {
    val view = LocalView.current
    // Android reports which buttons are held, never "this one went down": the
    // tracker turns consecutive states into presses and releases.
    val tracker = remember { AwtMouseTracker() }
    // API 26 is where Android learned to hand an app relative mouse motion; below
    // that the canvas simply stays in absolute mode.
    val supported = Build.VERSION.SDK_INT >= Build.VERSION_CODES.O
    DisposableEffect(view, enabled, supported) {
        if (!enabled || !supported) {
            return@DisposableEffect onDispose { }
        }
        tracker.setCaptured(true)
        val listener = View.OnCapturedPointerListener { _, event ->
            handleCapturedPointer(viewModel, tracker, event)
        }
        runCatching {
            view.setOnCapturedPointerListener(listener)
            // The window must have focus, and it may not have it yet on the frame
            // the session opens — `requestFocus` first, then capture.
            view.requestFocus()
            view.requestPointerCapture()
        }
        onDispose {
            runCatching {
                view.releasePointerCapture()
                view.setOnCapturedPointerListener(null)
            }
            // Leaving the screen with a button held would keep the game mining.
            viewModel.onInputEvents(tracker.releaseHeld())
            viewModel.flushInput()
        }
    }
}

/**
 * One captured `MotionEvent` → input events (task 12).
 *
 * A captured mouse reports movement in `AXIS_RELATIVE_X/Y`; some devices only
 * fill `getX()/getY()`, which are then already relative, so both are read and the
 * axis wins when it carries anything. Buttons come as a *state* bitmask with no
 * transition, which is what [AwtMouseTracker] exists for — and its captured
 * dialect carries no coordinates, because the click has to land wherever the
 * core's virtual pointer is.
 */
private fun handleCapturedPointer(
    viewModel: AwtSurfaceViewModel,
    tracker: AwtMouseTracker,
    event: MotionEvent,
): Boolean {
    var handled = false
    when (event.actionMasked) {
        MotionEvent.ACTION_MOVE, MotionEvent.ACTION_HOVER_MOVE -> {
            val dx = relativeAxis(event, MotionEvent.AXIS_RELATIVE_X, event.x)
            val dy = relativeAxis(event, MotionEvent.AXIS_RELATIVE_Y, event.y)
            if (dx != 0f || dy != 0f) {
                viewModel.onMouseRelative(dx, dy, AwtPointerSource.MOUSE)
                handled = true
            }
        }
        MotionEvent.ACTION_DOWN,
        MotionEvent.ACTION_UP,
        MotionEvent.ACTION_BUTTON_PRESS,
        MotionEvent.ACTION_BUTTON_RELEASE,
        -> {
            // The bitmask is the truth; `getActionButton()` is 0 on some ROMs, so
            // diffing the state is both simpler and more portable.
            val events = tracker.onCapturedButtonState(event.buttonState)
            if (events.isNotEmpty()) {
                viewModel.onInputEvents(events)
                handled = true
            }
        }
        MotionEvent.ACTION_SCROLL -> {
            val ticks = event.getAxisValue(MotionEvent.AXIS_VSCROLL)
            if (ticks != 0f) {
                // Android's VSCROLL is positive *away* from the user; AWT (and the
                // core) count positive toward the user, hence the negation.
                viewModel.onCapturedScroll(-ticks)
                handled = true
            }
        }
        else -> Unit
    }
    return handled
}

private fun relativeAxis(event: MotionEvent, axis: Int, fallback: Float): Float {
    val value = runCatching { event.getAxisValue(axis) }.getOrDefault(0f)
    return if (value != 0f) value else fallback
}

