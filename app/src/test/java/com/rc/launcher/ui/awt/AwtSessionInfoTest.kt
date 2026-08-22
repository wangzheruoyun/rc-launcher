package com.rc.launcher.ui.awt

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Unit tests for parsing the core's AWT session snapshot (task 18).
 *
 * The JSON below is the exact document `AwtHost::to_json` +
 * `ffi::awt_info_json` produce, so this test is also the contract check between
 * the Rust and Kotlin halves of the bridge.
 */
class AwtSessionInfoTest {

    private val snapshot = """
        {"backend":"cacio17",
         "screen":{"width":1280,"height":720},
         "surface":{"width":2400,"height":1080},
         "scale_mode":"fit",
         "placement":{"x":240,"y":0,"width":1920,"height":1080},
         "focused":true,"modifiers":64,"pending_events":3,
         "rgba_len":3686400,"uptime_ms":1234,
         "canvas":{"width":1280,"height":720,"fps":58.5,"frames_submitted":10,
                   "frames_presented":9,"frames_dropped":1,"bytes_blitted":4096,
                   "resizes":0,"last_seq":9,"dirty":null},
         "session":{"frames_accepted":9,"frames_rejected":1,"events_queued":5,
                    "events_drained":4,"events_dropped":2,"screen_resizes":0,
                    "surface_resizes":1},
         "link":{"state":"attached","frames_accepted":9,"frames_rejected":1,
                 "events_written":4,"events_lost":0,"reason":null},
         "transport":{"protocol":"rcaf1","frames":"/data/awt/awt-frames.rcaf",
                      "events":"/data/awt/awt-events.rcae"},
         "open":true}
    """.trimIndent()

    @Test
    fun aFullSnapshotIsParsed() {
        val info = AwtSessionInfo.parse(snapshot)
        assertTrue(info.open)
        assertEquals("cacio17", info.backend)
        assertEquals(1280, info.screenWidth)
        assertEquals(720, info.screenHeight)
        assertEquals(2400, info.surfaceWidth)
        assertEquals(1080, info.surfaceHeight)
        assertEquals(AwtScaleMode.FIT, info.scaleMode)
        assertEquals(AwtPlacement(240, 0, 1920, 1080), info.placement)
        assertTrue(info.focused)
        assertEquals(64, info.modifiers)
        assertEquals(3, info.pendingEvents)
        assertEquals(1280 * 720 * 4, info.rgbaLen)
        assertEquals(1234L, info.uptimeMs)
        assertEquals(58.5f, info.fps, 1e-4f)
        assertEquals(9L, info.framesPresented)
        assertEquals(1L, info.framesDropped)
        assertEquals(9L, info.framesAccepted)
        assertEquals(1L, info.framesRejected)
        assertEquals(2L, info.eventsDropped)
        assertNull(info.error)
        assertEquals("/data/awt/awt-frames.rcaf", info.framesChannel)
        assertEquals("/data/awt/awt-events.rcae", info.eventsChannel)
        assertTrue(info.hasTransport)
        assertTrue(info.link.attached)
        assertEquals("已连接", info.link.label)
        assertEquals(4L, info.link.eventsWritten)
        assertNull(info.link.reason)
        // The locally recomputed viewport agrees with the core's placement.
        assertEquals(info.placement, info.viewport.placement())
        assertTrue(info.describe().contains("cacio17"))
    }

    @Test
    fun aClosedSessionIsReportedWithoutAnError() {
        val info = AwtSessionInfo.parse("{\"open\":false}")
        assertFalse(info.open)
        assertNull(info.error)
        assertFalse(info.hasTransport)
        assertEquals("未开启", info.describe())
    }

    @Test
    fun anErrorAnswerBecomesAClosedSessionCarryingTheMessage() {
        val info = AwtSessionInfo.parse("{\"error\":\"no AWT session is open\"}")
        assertFalse(info.open)
        assertEquals("no AWT session is open", info.error)
    }

    @Test
    fun malformedJsonNeverThrows() {
        for (bad in listOf(null, "", "not json", "[1,2,3]", "{\"screen\":")) {
            val info = AwtSessionInfo.parse(bad)
            assertFalse(info.open)
            assertNotNull("a parse failure must be reported", info.error)
        }
    }

    @Test
    fun aLinkThatEndedCarriesItsReason() {
        val info = AwtSessionInfo.parse(
            "{\"screen\":{\"width\":8,\"height\":4},\"surface\":{\"width\":8,\"height\":4}," +
                "\"link\":{\"state\":\"ended\",\"events_lost\":3," +
                "\"reason\":\"the game JVM closed the AWT frame channel\"}}",
        )
        assertTrue("a snapshot without an explicit flag is an open session", info.open)
        assertTrue(info.link.ended)
        assertEquals("已断开", info.link.label)
        assertEquals(3L, info.link.eventsLost)
        assertEquals("the game JVM closed the AWT frame channel", info.link.reason)
    }

    @Test
    fun frameUpdatesAndInputResultsAreParsed() {
        val changed = AwtFrameUpdate.parse(
            "{\"changed\":true,\"x\":1,\"y\":2,\"width\":3,\"height\":4,\"bytes\":48}",
        )
        assertTrue(changed.changed)
        assertEquals(AwtRect(1, 2, 3, 4), changed.damage)
        assertEquals(48, changed.bytes)
        assertFalse(AwtFrameUpdate.parse("{\"changed\":false}").changed)
        assertEquals("boom", AwtFrameUpdate.parse("{\"error\":\"boom\"}").error)
        assertNotNull(AwtFrameUpdate.parse("garbage").error)

        val input = AwtInputResult.parse(
            "{\"queued\":8,\"pending\":8,\"modifiers\":64,\"focused\":true," +
                "\"pointer\":{\"x\":2,\"y\":1},\"rejected\":[\"unknown AWT input event type\"]}",
        )
        assertEquals(8, input.queued)
        assertEquals(8, input.pending)
        assertEquals(64, input.modifiers)
        assertTrue(input.focused)
        assertEquals(AwtPoint(2, 1), input.pointer)
        assertEquals(1, input.rejected.size)
        assertNotNull(AwtInputResult.parse("nope").error)
    }
}
