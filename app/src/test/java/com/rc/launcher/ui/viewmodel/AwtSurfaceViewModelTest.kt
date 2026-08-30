package com.rc.launcher.ui.viewmodel

import com.rc.launcher.ui.awt.AwtButtonEvent
import com.rc.launcher.ui.awt.AwtCanvasBridge
import com.rc.launcher.ui.awt.AwtCaptureEvent
import com.rc.launcher.ui.awt.AwtConfigureRequest
import com.rc.launcher.ui.awt.AwtControlBatch
import com.rc.launcher.ui.awt.AwtControlKind
import com.rc.launcher.ui.awt.AwtControlRequest
import com.rc.launcher.ui.awt.AwtControlResult
import com.rc.launcher.ui.awt.AwtControlWire
import com.rc.launcher.ui.awt.AwtCursorKind
import com.rc.launcher.ui.awt.AwtFrameUpdate
import com.rc.launcher.ui.awt.AwtInputEvent
import com.rc.launcher.ui.awt.AwtInputResult
import com.rc.launcher.ui.awt.AwtMouseButton
import com.rc.launcher.ui.awt.AwtPlacement
import com.rc.launcher.ui.awt.AwtPointerEvent
import com.rc.launcher.ui.awt.AwtPointerMode
import com.rc.launcher.ui.awt.AwtPointerPhase
import com.rc.launcher.ui.awt.AwtPointerSource
import com.rc.launcher.ui.awt.AwtRelativePointerEvent
import com.rc.launcher.ui.awt.AwtRect
import com.rc.launcher.ui.awt.AwtScaleMode
import com.rc.launcher.ui.awt.AwtScrollAtPointerEvent
import com.rc.launcher.ui.awt.AwtSessionConfig
import com.rc.launcher.ui.awt.AwtSessionInfo
import com.rc.launcher.ui.awt.AwtReleaseAllEvent
import com.rc.launcher.ui.ScreenOrientation
import com.rc.launcher.ui.awt.FakeAwtCanvasBridge
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer

/**
 * Unit tests for the AWT canvas state container (task 18).
 *
 * The [FakeAwtCanvasBridge] re-implements the core's framebuffer + damage
 * tracking in Kotlin, so the *whole* UI path (open → poll → upload → input) is
 * exercised on the JVM without the native library.
 */
class AwtSurfaceViewModelTest {

    private fun vm(bridge: AwtCanvasBridge = FakeAwtCanvasBridge()) = AwtSurfaceViewModel(bridge)

    @Test
    fun aFreshViewModelIsClosed() {
        val v = vm()
        assertFalse(v.state.value.open)
        assertNull(v.frameBuffer)
        assertEquals(AwtFrameUpdate.NONE, v.poll())
    }

    @Test
    fun openAllocatesTheFramebufferAndReportsTheSession() {
        val v = vm()
        v.open(screenWidth = 64, screenHeight = 32, javaVersion = "jre17")
        val state = v.state.value
        assertTrue(state.open)
        assertEquals(64, state.info.screenWidth)
        assertEquals(32, state.info.screenHeight)
        assertEquals("cacio17", state.info.backend)
        assertEquals(64 * 32 * 4, state.info.rgbaLen)
        assertNotNull(v.frameBuffer)
        assertEquals(64 * 32 * 4, v.frameBuffer!!.capacity())
        assertTrue("the framebuffer must be direct for a zero-copy poll", v.frameBuffer!!.isDirect)
    }

    @Test
    fun aSurfaceResizeOnlyChangesTheLetterboxing() {
        val v = vm()
        v.open(screenWidth = 64, screenHeight = 32)
        val capacity = v.frameBuffer!!.capacity()
        v.onSurfaceSizeChanged(256, 256)
        val state = v.state.value
        assertEquals(256, state.surfaceWidth)
        assertEquals(256, state.surfaceHeight)
        assertEquals(64, state.info.screenWidth)
        assertEquals(capacity, v.frameBuffer!!.capacity())
        // 2:1 desktop on a square surface => letterboxed vertically.
        assertEquals(AwtPlacement(0, 64, 256, 128), state.placement)
        // Repeating the same size is a no-op.
        v.onSurfaceSizeChanged(256, 256)
        assertEquals(256, v.state.value.surfaceWidth)
    }

    @Test
    fun pollUploadsTheFirstFrameThenSkipsWhileIdle() {
        val v = vm()
        v.open(screenWidth = 8, screenHeight = 4)
        // A freshly opened desktop is fully damaged (opaque black).
        val first = v.poll()
        assertTrue(first.changed)
        assertEquals(AwtRect(0, 0, 8, 4), first.damage)
        assertEquals(1L, v.state.value.uploads)
        // Nothing changed since: no upload, no recomposition.
        assertFalse(v.poll().changed)
        assertEquals(1L, v.state.value.uploads)
        assertEquals(1L, v.state.value.skipped)
    }

    @Test
    fun testPatternTravelsAllTheWayIntoTheFramebufferAsRgba() {
        val v = vm()
        v.open(screenWidth = 8, screenHeight = 4)
        v.poll() // consume the initial black frame
        v.submitTestPattern()
        assertTrue(v.state.value.lastUpdate.changed)
        assertTrue(v.state.value.message!!.contains("自检帧"))
        assertTrue(v.poll().changed)
        val buffer = v.frameBuffer!!
        // The pattern's border is opaque white; RGBA8888 byte order.
        assertEquals(0xFF, buffer.get(0).toInt() and 0xFF)
        assertEquals(0xFF, buffer.get(1).toInt() and 0xFF)
        assertEquals(0xFF, buffer.get(2).toInt() and 0xFF)
        assertEquals(0xFF, buffer.get(3).toInt() and 0xFF)
    }

    @Test
    fun aTestPatternWithoutASessionIsRefusedPolitely() {
        val v = vm()
        v.submitTestPattern()
        assertEquals("请先开启 AWT 会话", v.state.value.message)
        v.clearMessage()
        assertNull(v.state.value.message)
    }

    @Test
    fun pointerMovesAreBatchedUntilTheNextPoll() {
        val fake = FakeAwtCanvasBridge()
        val v = vm(fake)
        v.open(screenWidth = 64, screenHeight = 32)
        v.onSurfaceSizeChanged(64, 32)
        v.onPointer(AwtPointerPhase.MOVE, 1f, 1f)
        v.onPointer(AwtPointerPhase.MOVE, 2f, 2f)
        assertTrue("moves must not cost a JNI call each", fake.received.isEmpty())
        v.poll()
        assertEquals(2, fake.received.size)
        assertEquals(
            AwtPointerEvent(AwtPointerPhase.MOVE, 1f, 1f, AwtMouseButton.LEFT),
            fake.received.first(),
        )
    }

    @Test
    fun pressesReleasesAndKeysAreFlushedImmediately() {
        val fake = FakeAwtCanvasBridge()
        val v = vm(fake)
        v.open(screenWidth = 64, screenHeight = 32)
        v.onSurfaceSizeChanged(64, 32)
        v.onPointer(AwtPointerPhase.DOWN, 10f, 10f)
        assertEquals(1, fake.received.size)
        v.onPointer(AwtPointerPhase.UP, 10f, 10f)
        assertEquals(2, fake.received.size)
        v.onKeyNamed(true, "escape")
        v.onKey(down = false, code = 27)
        v.onText("hi")
        v.onFocusChanged(false)
        v.releaseAll()
        assertEquals(7, fake.received.size)
        assertFalse("losing focus is reflected in the state", v.state.value.info.focused)
        // Blank / empty input is dropped before it reaches the core.
        v.onKeyNamed(true, "  ")
        v.onText("")
        v.onScroll(1f, 1f, 0)
        assertEquals(7, fake.received.size)
    }

    @Test
    fun aTapOnTheLetterboxBarQueuesNoAwtEvent() {
        val fake = FakeAwtCanvasBridge()
        val v = vm(fake)
        v.open(screenWidth = 64, screenHeight = 32) // 2:1 desktop
        v.onSurfaceSizeChanged(64, 64) // square surface => bars at the top/bottom
        val result = v.state.value
        assertEquals(AwtPlacement(0, 16, 64, 32), result.placement)
        v.onPointer(AwtPointerPhase.DOWN, 32f, 2f) // inside the top bar
        assertEquals(0, v.state.value.lastInput.queued)
        v.onPointer(AwtPointerPhase.DOWN, 32f, 32f) // on the picture
        assertEquals(1, v.state.value.lastInput.queued)
    }

    @Test
    fun theInputQueueIsBoundedEvenIfNobodyPolls() {
        val fake = FakeAwtCanvasBridge()
        val v = vm(fake)
        v.open(screenWidth = 64, screenHeight = 32)
        v.onSurfaceSizeChanged(64, 32)
        repeat(AwtSurfaceViewModel.MAX_PENDING_INPUT) {
            v.onPointer(AwtPointerPhase.MOVE, it.toFloat(), 1f)
        }
        assertEquals(
            "a full queue flushes itself",
            AwtSurfaceViewModel.MAX_PENDING_INPUT,
            fake.received.size,
        )
    }

    @Test
    fun scaleModeAndDesktopResizeGoThroughTheCore() {
        val v = vm()
        v.open(screenWidth = 64, screenHeight = 32)
        v.setScaleMode(AwtScaleMode.STRETCH)
        assertEquals(AwtScaleMode.STRETCH, v.state.value.info.scaleMode)
        v.resizeDesktop(128, 64)
        assertEquals(128, v.state.value.info.screenWidth)
        assertEquals(128 * 64 * 4, v.frameBuffer!!.capacity())
        // The clamp keeps a hostile size from allocating gigabytes.
        v.resizeDesktop(0, 999_999)
        assertEquals(1, v.state.value.info.screenWidth)
        assertEquals(8192, v.state.value.info.screenHeight)
    }

    @Test
    fun repaintDamagesEverythingAgain() {
        val v = vm()
        v.open(screenWidth = 8, screenHeight = 4)
        v.poll()
        assertFalse(v.poll().changed)
        v.repaint(0xFFFF0000.toInt())
        val update = v.poll()
        assertTrue(update.changed)
        assertEquals(AwtRect(0, 0, 8, 4), update.damage)
        val buffer = v.frameBuffer!!
        assertEquals(0xFF, buffer.get(0).toInt() and 0xFF) // R
        assertEquals(0x00, buffer.get(1).toInt() and 0xFF) // G
    }

    @Test
    fun attachingATransportSurfacesTheChannels() {
        val v = vm()
        v.open(screenWidth = 8, screenHeight = 4)
        v.attachTransport("/data/awt")
        val info = v.state.value.info
        assertTrue(info.hasTransport)
        assertEquals("/data/awt/awt-frames.rcaf", info.framesChannel)
        assertTrue(info.link.attached)
    }

    @Test
    fun closeReleasesTheFramebufferAndForgetsTheSession() {
        val v = vm()
        v.open(screenWidth = 8, screenHeight = 4)
        v.onSurfaceSizeChanged(16, 8)
        v.close()
        assertFalse(v.state.value.open)
        assertNull(v.frameBuffer)
        // The surface size survives, so reopening keeps the letterboxing.
        assertEquals(16, v.state.value.surfaceWidth)
        // Input after a close is dropped instead of crashing.
        v.onPointer(AwtPointerPhase.DOWN, 1f, 1f)
        assertEquals(AwtInputResult.EMPTY, v.flushInput())
    }

    // ---- Control plane ------------------------------------------------------

    @Test
    fun theControlPlaneIsProjectedIntoTheUiState() {
        val bridge = FakeAwtCanvasBridge()
        val v = vm(bridge)
        v.open(screenWidth = 64, screenHeight = 32)
        assertEquals(AwtCursorKind.DEFAULT, v.state.value.cursor)
        assertFalse(v.state.value.wantsKeyboard)

        bridge.submitControl(AwtControlWire.encodeCursor(AwtCursorKind.TEXT))
        bridge.submitControl(AwtControlWire.encode(AwtControlKind.TITLE, text = "Forge"))
        bridge.submitControl(AwtControlWire.encodeImeShow(32, 16, 8))
        val batch = v.pumpControl()

        assertEquals(3, batch.messages.size)
        assertEquals(AwtCursorKind.TEXT, v.state.value.cursor)
        assertEquals("Forge", v.state.value.title)
        assertTrue(v.state.value.wantsKeyboard)
        assertEquals(3L, v.state.value.controlMessages)
        // The caret is mapped through the very viewport the pixels use, so the IME
        // anchor cannot drift onto the letterbox bars.
        val caret = v.state.value.caretOnSurface
        assertNotNull(caret)
        assertEquals(v.state.value.viewport.mapToSurface(32, 16), caret)
        // Draining is destructive.
        assertTrue(v.pumpControl().isEmpty)
        assertEquals(3L, v.state.value.controlMessages)
    }

    @Test
    fun aClipboardRequestIsAnsweredEvenWhenAndroidHasNoText() {
        val bridge = FakeAwtCanvasBridge()
        val v = vm(bridge)
        v.open()
        bridge.submitControl(AwtControlWire.encodeClipboardRequest(7))
        val batch = v.pumpControl()
        assertEquals(7, batch.clipboardRequestSeq)

        v.answerClipboard(null, batch.clipboardRequestSeq)
        assertEquals(listOf<String?>(null), bridge.clipboardAnswers)
        v.answerClipboard("pasted")
        assertEquals(listOf(null, "pasted"), bridge.clipboardAnswers)
        assertNull(v.state.value.message)
    }

    @Test
    fun aClosedSessionHasNoCursorAndWantsNoKeyboard() {
        val bridge = FakeAwtCanvasBridge()
        val v = vm(bridge)
        v.open()
        bridge.submitControl(AwtControlWire.encodeCursor(AwtCursorKind.HAND))
        bridge.submitControl(AwtControlWire.encodeImeShow(1, 1, 1))
        v.pumpControl()
        assertTrue(v.state.value.wantsKeyboard)

        v.close()
        assertEquals(AwtCursorKind.DEFAULT, v.state.value.cursor)
        assertFalse("the soft keyboard must retract", v.state.value.wantsKeyboard)
        assertNull(v.state.value.title)
        // The control plane is inert without a session.
        assertTrue(v.pumpControl().isEmpty)
    }

    @Test
    fun resettingTheControlPlaneForgetsTheCursor() {
        val bridge = FakeAwtCanvasBridge()
        val v = vm(bridge)
        v.open()
        bridge.submitControl(AwtControlWire.encodeCursor(AwtCursorKind.WAIT))
        v.pumpControl()
        assertEquals(AwtCursorKind.WAIT, v.state.value.cursor)
        v.resetControl()
        assertEquals(AwtCursorKind.DEFAULT, v.state.value.cursor)
    }

    @Test
    fun aFailingControlPlaneBecomesAMessageNotACrash() {
        val v = vm(BoomBridge())
        v.open()
        v.clearMessage()
        // No session is open (the bridge blew up), so the control plane is a no-op…
        assertTrue(v.pumpControl().isEmpty)
        // …and an explicit answer still degrades to a visible message.
        val result = v.answerClipboard("x")
        assertNotNull(result.error)
        assertNotNull(v.state.value.message)
        assertFalse(v.submitControl(AwtControlWire.encode(AwtControlKind.BEEP)))
        assertNotNull(v.resetControl().error)
    }

    @Test
    fun aFailingBridgeBecomesAMessageNotACrash() {
        val v = vm(BoomBridge())
        assertFalse(v.state.value.open)
        v.open()
        val message = v.state.value.message
        assertNotNull(message)
        assertTrue(message!!, message.contains("boom"))
        // Every follow-up call degrades quietly.
        assertEquals(AwtFrameUpdate.NONE, v.poll())
        v.setScaleMode(AwtScaleMode.FIT)
        v.onPointer(AwtPointerPhase.DOWN, 1f, 1f)
        v.close()
        assertFalse(v.state.value.open)
    }

    /** A bridge whose every call fails (missing `librc_launcher.so`, …). */
    private class BoomBridge : AwtCanvasBridge {
        override fun open(config: AwtSessionConfig): AwtSessionInfo = throw IllegalStateException("boom")
        override fun close(): Boolean = throw IllegalStateException("boom")
        override fun info(): AwtSessionInfo = throw IllegalStateException("boom")
        override fun configure(request: AwtConfigureRequest): AwtSessionInfo = throw IllegalStateException("boom")
        override fun attachTransport(dir: String): AwtSessionInfo = throw IllegalStateException("boom")
        override fun input(events: List<AwtInputEvent>): AwtInputResult = throw IllegalStateException("boom")
        override fun submitFrame(frame: ByteArray): AwtFrameUpdate = throw IllegalStateException("boom")
        override fun poll(buffer: ByteBuffer): AwtFrameUpdate = throw IllegalStateException("boom")
        override fun drainEvents(): ByteArray = throw IllegalStateException("boom")
        override fun drainControl(): AwtControlBatch = throw IllegalStateException("boom")
        override fun control(request: AwtControlRequest): AwtControlResult =
            throw IllegalStateException("boom")
        override fun submitControl(message: ByteArray): Boolean =
            throw IllegalStateException("boom")
    }
    // ---- Rotation safety (task 9) ------------------------------------------

    @Test
    fun aRotationFlushesQueuedInputBeforeTheNewGeometry() {
        val fake = FakeAwtCanvasBridge()
        val v = vm(fake)
        // A portrait desktop, so the session starts portrait like the phone.
        v.open(screenWidth = 32, screenHeight = 64)
        v.onSurfaceSizeChanged(1080, 2400) // portrait phone
        val configuresBefore = fake.configures.size

        // A press is flushed at once; the following move is only *queued*.
        v.onPointer(AwtPointerPhase.DOWN, 540f, 1200f)
        v.onPointer(AwtPointerPhase.MOVE, 500f, 1100f)
        assertEquals(1, fake.received.size)

        v.onSurfaceSizeChanged(2400, 1080) // rotate

        // The queued sample was taken against the *old* viewport, so it must have
        // reached the core before the new surface size did — otherwise it would be
        // mapped through the new letterboxing and land in the wrong place.
        val mark = fake.configureInputMarks[configuresBefore]
        assertEquals("queued samples flush before the rotation is published", 2, mark)
        assertTrue(fake.received[1] is AwtPointerEvent)
    }

    @Test
    fun aRotationReleasesTheInFlightGesture() {
        val fake = FakeAwtCanvasBridge()
        val v = vm(fake)
        // A portrait desktop, so the session starts portrait like the phone.
        v.open(screenWidth = 32, screenHeight = 64)
        v.onSurfaceSizeChanged(1080, 2400)
        v.onPointer(AwtPointerPhase.DOWN, 540f, 1200f)

        v.onSurfaceSizeChanged(2400, 1080)

        assertEquals(AwtReleaseAllEvent, fake.received.last())
        assertEquals(1L, v.state.value.rotations)
        assertEquals(ScreenOrientation.LANDSCAPE, v.state.value.surfaceOrientation)
        assertEquals(2400, v.state.value.surfaceWidth)
        assertEquals(1080, v.state.value.surfaceHeight)
    }

    @Test
    fun resizingInsideOneOrientationKeepsTheGesture() {
        val fake = FakeAwtCanvasBridge()
        val v = vm(fake)
        // A portrait desktop, so the session starts portrait like the phone.
        v.open(screenWidth = 32, screenHeight = 64)
        v.onSurfaceSizeChanged(1080, 2400)
        v.onPointer(AwtPointerPhase.DOWN, 540f, 1200f)
        val received = fake.received.size

        // The soft keyboard shrinking the surface must not drop the drag.
        v.onSurfaceSizeChanged(1080, 1400)

        assertEquals(received, fake.received.size)
        assertEquals(0L, v.state.value.rotations)
        assertEquals(ScreenOrientation.PORTRAIT, v.state.value.surfaceOrientation)
    }

    @Test
    fun theFirstMeasurementIsNotARotation() {
        val fake = FakeAwtCanvasBridge()
        val v = vm(fake)
        // A portrait desktop, so the session starts portrait like the phone.
        v.open(screenWidth = 32, screenHeight = 64)
        // The surface starts at 0x0 (never laid out): adopting a real size is not
        // a rotation, so nothing may be released.
        v.onSurfaceSizeChanged(1080, 2400)
        assertEquals(0L, v.state.value.rotations)
        assertTrue(fake.received.isEmpty())
        // A repeated identical size is a no-op.
        val configures = fake.configures.size
        v.onSurfaceSizeChanged(1080, 2400)
        assertEquals(configures, fake.configures.size)
    }

    @Test
    fun rotationBookkeepingSurvivesAClosedSession() {
        val v = vm()
        // No session: the geometry is still tracked (the next `open` reuses it) and
        // nothing throws.
        v.onSurfaceSizeChanged(1080, 2400)
        v.onSurfaceSizeChanged(2400, 1080)
        assertEquals(1L, v.state.value.rotations)
        assertEquals(ScreenOrientation.LANDSCAPE, v.state.value.surfaceOrientation)
        // A portrait desktop, so the session starts portrait like the phone.
        v.open(screenWidth = 32, screenHeight = 64)
        assertEquals(2400, v.state.value.info.surfaceWidth)
    }

    // ---- Physical keyboard & mouse (task 12) -------------------------------

    @Test
    fun relativeMouseMotionIsBatchedLikeEveryOtherMotion() {
        val bridge = FakeAwtCanvasBridge()
        val v = vm(bridge)
        v.open(screenWidth = 320, screenHeight = 240)
        v.onSurfaceSizeChanged(640, 480)
        bridge.received.clear()
        v.onMouseRelative(3f, 4f)
        v.onMouseRelative(1f, 0f)
        assertTrue("a 1000 Hz mouse must not cross JNI per sample", bridge.received.isEmpty())
        v.flushInput()
        assertEquals(2, bridge.received.count { it is AwtRelativePointerEvent })
        // The pointer followed, scaled 1:1.
        assertEquals(4, v.state.value.lastInput.pointer.x)
        // Nothing and garbage never reach the core.
        bridge.received.clear()
        v.onMouseRelative(0f, 0f)
        v.onMouseRelative(Float.NaN, 1f)
        v.flushInput()
        assertTrue(bridge.received.isEmpty())
    }

    @Test
    fun capturingThePointerIsSentImmediatelyAndTracked() {
        val bridge = FakeAwtCanvasBridge()
        val v = vm(bridge)
        v.open(screenWidth = 320, screenHeight = 240)
        v.onSurfaceSizeChanged(640, 480)
        bridge.received.clear()
        v.setPointerCapture(true)
        // A capture must not wait for the frame boundary.
        assertEquals(listOf(AwtCaptureEvent(true)), bridge.received.toList())
        assertTrue(v.state.value.captured)
        assertEquals(AwtPointerMode.CAPTURED, v.state.value.lastInput.pointerMode)
        // Idempotent: asking twice is not two grabs.
        bridge.received.clear()
        v.setPointerCapture(true)
        assertTrue(bridge.received.isEmpty())
        v.togglePointerCapture()
        assertFalse(v.state.value.captured)
    }

    @Test
    fun aCapturedCursorKeepsTurningPastTheDesktopEdge() {
        val bridge = FakeAwtCanvasBridge()
        val v = vm(bridge)
        v.open(screenWidth = 320, screenHeight = 240)
        v.onSurfaceSizeChanged(640, 480)
        v.setPointerCapture(true)
        repeat(4) { v.onMouseRelative(200f, 0f) }
        v.flushInput()
        // The AWT pointer stops at the desktop edge…
        assertEquals(319, v.state.value.lastInput.pointer.x)
        // …while the game's cursor is free-running, which is the whole point.
        assertTrue(
            "the game cursor must leave the desktop: ${v.gameCursor}",
            v.gameCursor.x > 320,
        )
    }

    @Test
    fun aCapturedClickAndWheelHappenAtThePointer() {
        val bridge = FakeAwtCanvasBridge()
        val v = vm(bridge)
        v.open(screenWidth = 320, screenHeight = 240)
        bridge.received.clear()
        v.onCapturedButton(AwtMouseButton.RIGHT, down = true)
        v.onCapturedButton(AwtMouseButton.RIGHT, down = false)
        assertEquals(
            listOf(
                AwtButtonEvent(AwtMouseButton.RIGHT, true),
                AwtButtonEvent(AwtMouseButton.RIGHT, false),
            ),
            bridge.received.toList(),
        )
        bridge.received.clear()
        // Fractional notches round away from zero, so a small flick still counts.
        v.onCapturedScroll(0.3f)
        v.onCapturedScroll(-0.3f)
        v.onCapturedScroll(0f)
        v.onCapturedScroll(Float.NaN)
        v.flushInput()
        assertEquals(
            listOf(AwtScrollAtPointerEvent(1), AwtScrollAtPointerEvent(-1)),
            bridge.received.toList(),
        )
    }

    @Test
    fun aStrayTouchIsFilteredWhileTheMouseOwnsThePointer() {
        val bridge = FakeAwtCanvasBridge()
        val v = vm(bridge)
        v.open(screenWidth = 320, screenHeight = 240)
        v.onSurfaceSizeChanged(640, 480)
        v.setHybridTouch(false)
        v.setPointerCapture(true)
        v.onPointer(AwtPointerPhase.DOWN, 320f, 240f, AwtMouseButton.LEFT, AwtPointerSource.TOUCH)
        assertEquals(1, bridge.filteredSamples)
        // The mouse always gets through.
        v.onMouseRelative(5f, 0f)
        v.flushInput()
        assertEquals(1, bridge.filteredSamples)
    }

    @Test
    fun settingsAreEditedThroughConfigureAndReadBack() {
        val bridge = FakeAwtCanvasBridge()
        val v = vm(bridge)
        v.open(screenWidth = 320, screenHeight = 240)
        v.setSensitivity(2.5f)
        assertEquals(2500, v.state.value.sensitivity.xPermille)
        assertEquals(2500, v.state.value.sensitivity.yPermille)
        v.setInvertY(true)
        assertTrue(v.state.value.sensitivity.invertY)
        v.bindKey("key.keyboard.e", "key.keyboard.f")
        assertEquals("f", v.state.value.inputSettings.bindings.resolveKey("e"))
        v.bindButton(AwtMouseButton.LEFT, AwtMouseButton.RIGHT)
        assertEquals(
            AwtMouseButton.RIGHT,
            v.state.value.inputSettings.bindings.resolveButton(AwtMouseButton.LEFT),
        )
        v.clearBindings()
        assertTrue(v.state.value.inputSettings.bindings.isEmpty)
        // Out-of-range values are clamped, never sent as-is.
        v.setScrollPermille(1_000_000)
        assertEquals(10_000, v.state.value.inputSettings.scrollPermille)
        // Every edit went through `awtConfigure` with an `input` member.
        assertTrue(bridge.configures.count { it.input != null } >= 6)
    }

    @Test
    fun aKeyForwardsItsPhysicalScancode() {
        val bridge = FakeAwtCanvasBridge()
        val v = vm(bridge)
        v.open(screenWidth = 320, screenHeight = 240)
        bridge.received.clear()
        v.onKeyNamed(true, "key.keyboard.w", 17)
        val key = bridge.received.filterIsInstance<com.rc.launcher.ui.awt.AwtKeyEvent>().single()
        assertEquals(17, key.scancode)
        assertEquals("key.keyboard.w", key.name)
        assertTrue(key.toJson().toString().contains("17"))
    }

    @Test
    fun aFocusedTextFieldGivesThePointerBack() {
        val bridge = FakeAwtCanvasBridge()
        val v = vm(bridge)
        v.open(screenWidth = 320, screenHeight = 240)
        v.setPointerCapture(true)
        assertTrue(v.state.value.captured)
        // The JVM says a Swing text component wants the keyboard: typing is
        // impossible with the pointer captured, so the capture has to go.
        bridge.submitControl(AwtControlWire.encodeImeShow(x = 8, y = 8, lineHeight = 12))
        v.pumpControl()
        assertFalse("a text field must release the pointer", v.state.value.captured)
    }
}
