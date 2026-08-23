package com.rc.launcher.ui.component

import android.graphics.Bitmap
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
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.nativeKeyCode
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.key.utf16CodePoint
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.Button as PointerButton
import androidx.compose.ui.input.pointer.PointerButtons
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.rc.launcher.ui.awt.AwtMouseButton
import com.rc.launcher.ui.awt.AwtPointerPhase
import com.rc.launcher.ui.awt.awtKeyNameForAndroidKeyCode
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
 */
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
                val name = awtKeyNameForAndroidKeyCode(event.key.nativeKeyCode)
                when (event.type) {
                    KeyEventType.KeyDown -> {
                        if (name != null) {
                            viewModel.onKeyNamed(true, name)
                        } else if (event.utf16CodePoint != 0) {
                            viewModel.onText(String(Character.toChars(event.utf16CodePoint)))
                        }
                        name != null || event.utf16CodePoint != 0
                    }
                    KeyEventType.KeyUp -> {
                        if (name != null) viewModel.onKeyNamed(false, name)
                        name != null
                    }
                    else -> false
                }
            }
            .pointerInput(state.open, touchButton) {
                if (!state.open) return@pointerInput
                awaitPointerEventScope {
                    while (true) {
                        val event = awaitPointerEvent()
                        val change = event.changes.firstOrNull() ?: continue
                        if (event.type == PointerEventType.Scroll) {
                            // Compose reports "scroll down" as +1; AWT wheel
                            // notches use the same sign convention.
                            viewModel.onScroll(
                                change.position.x,
                                change.position.y,
                                change.scrollDelta.y.toInt(),
                            )
                            change.consume()
                            continue
                        }
                        val button = when {
                            event.buttons.pressedButtons.contains(PointerButton.secondary) -> AwtMouseButton.RIGHT
                            event.buttons.pressedButtons.contains(PointerButton.tertiary) -> AwtMouseButton.MIDDLE
                            else -> touchButton
                        }
                        val phase = when {
                            change.pressed && !change.previousPressed -> AwtPointerPhase.DOWN
                            !change.pressed && change.previousPressed -> AwtPointerPhase.UP
                            change.pressed -> AwtPointerPhase.MOVE
                            else -> null
                        }
                        if (phase != null) {
                            viewModel.onPointer(phase, change.position.x, change.position.y, button)
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
            Canvas(modifier = Modifier.fillMaxSize()) {
                // Reading `frameTick` inside the draw lambda is what ties a
                // repaint to a freshly uploaded frame (the Bitmap is mutated in
                // place, so Compose has no other way to know).
                val tick = frameTick
                if (tick >= 0) {
                    drawAwtDesktop(image, placement.x, placement.y, placement.width, placement.height)
                }
            }
        }
    }

    LaunchedEffect(state.open) {
        if (state.open) runCatching { focusRequester.requestFocus() }
    }
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
