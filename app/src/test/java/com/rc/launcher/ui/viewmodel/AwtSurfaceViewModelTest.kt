package com.rc.launcher.ui.viewmodel

import com.rc.launcher.ui.awt.AwtCanvasBridge
import com.rc.launcher.ui.awt.AwtConfigureRequest
import com.rc.launcher.ui.awt.AwtFrameUpdate
import com.rc.launcher.ui.awt.AwtInputEvent
import com.rc.launcher.ui.awt.AwtInputResult
import com.rc.launcher.ui.awt.AwtMouseButton
import com.rc.launcher.ui.awt.AwtPlacement
import com.rc.launcher.ui.awt.AwtPointerEvent
import com.rc.launcher.ui.awt.AwtPointerPhase
import com.rc.launcher.ui.awt.AwtRect
import com.rc.launcher.ui.awt.AwtScaleMode
import com.rc.launcher.ui.awt.AwtSessionConfig
import com.rc.launcher.ui.awt.AwtSessionInfo
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
    }
}
