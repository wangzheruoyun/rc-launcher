package com.rc.launcher.ui.awt

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Unit tests for the AWT **control plane** (task 18), Kotlin side.
 *
 * Two contracts are pinned here:
 *
 * * the `RCAC` wire layout and the chunked reply encoding, byte for byte, because
 *   the peer is the Rust core (`launch::awt`) and a silent field reorder would
 *   show up on a device as a cursor that never changes or a clipboard answer the
 *   JVM cannot parse;
 * * the JSON the core hands back, including its fail-soft behaviour: a truncated
 *   or hostile document must yield an empty batch, never an exception, because the
 *   AWT canvas may not be able to crash the launcher UI (task 19).
 */
class AwtControlTest {

    // ---- Wire ---------------------------------------------------------------

    @Test
    fun aControlMessageHasTheDocumentedHeaderLayout() {
        val bytes = AwtControlWire.encode(
            AwtControlKind.TITLE,
            seq = 9,
            a = 1,
            b = 2,
            c = 3,
            text = "abc",
        )
        assertEquals(AwtControlWire.CONTROL_HEADER_LEN + 3, bytes.size)
        // magic "RCAC" little-endian
        assertArrayEquals(
            byteArrayOf('C'.code.toByte(), 'A'.code.toByte(), 'C'.code.toByte(), 'R'.code.toByte()),
            bytes.copyOfRange(0, 4),
        )
        assertEquals(1, bytes[4].toInt()) // version
        assertEquals(AwtControlKind.TITLE.code, bytes[6].toInt())
        assertEquals(9, bytes[8].toInt()) // seq
        assertEquals(1, bytes[12].toInt()) // a
        assertEquals(2, bytes[16].toInt()) // b
        assertEquals(3, bytes[20].toInt()) // c
        assertEquals(3, bytes[24].toInt()) // payload length
        assertEquals("abc", String(bytes, 32, 3, Charsets.UTF_8))
    }

    @Test
    fun theControlHeaderIsTheSameShapeAsAFrameHeaderButADifferentMagic() {
        // This is what lets one stream reader demultiplex both record types: the
        // length is at the same offset, so an unknown record is still consumed
        // whole and the channel never loses alignment.
        assertEquals(AwtWire.FRAME_HEADER_LEN, AwtControlWire.CONTROL_HEADER_LEN)
        assertFalse(AwtWire.FRAME_MAGIC == AwtControlWire.CONTROL_MAGIC)
        val control = AwtControlWire.encodeCursor(AwtCursorKind.HAND)
        assertNull("a control message is not a frame", AwtWire.decodeFrameHeader(control))
    }

    @Test
    fun everyControlKindRoundTripsThroughTheWire() {
        val cases = listOf(
            AwtControlWire.encodeCursor(AwtCursorKind.TEXT) to AwtControlKind.CURSOR,
            AwtControlWire.encode(AwtControlKind.TITLE, text = "Forge 安装程序") to
                AwtControlKind.TITLE,
            AwtControlWire.encodeClipboardSet("copied") to AwtControlKind.CLIPBOARD_SET,
            AwtControlWire.encodeClipboardRequest(4) to AwtControlKind.CLIPBOARD_REQUEST,
            AwtControlWire.encode(AwtControlKind.BEEP) to AwtControlKind.BEEP,
            AwtControlWire.encodeScreenSize(1024, 768) to AwtControlKind.SCREEN_SIZE,
            AwtControlWire.encodeImeShow(10, 20, 16) to AwtControlKind.IME_SHOW,
            AwtControlWire.encode(AwtControlKind.IME_HIDE) to AwtControlKind.IME_HIDE,
            AwtControlWire.encode(AwtControlKind.WINDOW_OPENED, a = 3, text = "对话框") to
                AwtControlKind.WINDOW_OPENED,
            AwtControlWire.encode(AwtControlKind.WINDOW_CLOSED, a = 3) to
                AwtControlKind.WINDOW_CLOSED,
            AwtControlWire.encode(AwtControlKind.BYE, text = "JVM exited") to AwtControlKind.BYE,
        )
        for ((bytes, kind) in cases) {
            val parsed = AwtControlWire.decode(bytes)
            assertNotNull("$kind must decode", parsed)
            assertEquals(kind, parsed!!.kind)
        }
        assertEquals(AwtCursorKind.TEXT, AwtControlWire.decode(cases[0].first)!!.cursor)
        assertEquals("Forge 安装程序", AwtControlWire.decode(cases[1].first)!!.text)
        assertEquals(1024, AwtControlWire.decode(cases[5].first)!!.width)
        assertEquals(768, AwtControlWire.decode(cases[5].first)!!.height)
        assertEquals(AwtImeCaret(10, 20, 16), AwtControlWire.decode(cases[6].first)!!.caret)
        assertEquals(3, AwtControlWire.decode(cases[8].first)!!.window)
    }

    @Test
    fun decodeRejectsGarbageInsteadOfThrowing() {
        val good = AwtControlWire.encode(AwtControlKind.TITLE, text = "abc")
        assertNull(AwtControlWire.decode(ByteArray(0)))
        assertNull(AwtControlWire.decode(ByteArray(31)))
        assertNull(AwtControlWire.decode(good.copyOf().also { it[0] = 0 })) // magic
        assertNull(AwtControlWire.decode(good.copyOf().also { it[4] = 9 })) // version
        assertNull(AwtControlWire.decode(good.copyOf().also { it[6] = 99 })) // kind
        // Declared longer than delivered.
        assertNull(AwtControlWire.decode(good.copyOf().also { it[24] = 64 }))
        // Invalid UTF-8 must not be silently replaced with U+FFFD.
        val bad = AwtControlWire.encode(AwtControlKind.TITLE, text = "ab").copyOf()
        bad[bad.size - 2] = 0xFF.toByte()
        bad[bad.size - 1] = 0xFE.toByte()
        assertNull(AwtControlWire.decode(bad))
    }

    @Test
    fun cursorKindsMapBothWaysAndDegradeSafely() {
        for (kind in AwtCursorKind.entries) {
            assertEquals(kind, AwtCursorKind.fromId(kind.id))
            assertEquals(kind, AwtCursorKind.fromAwtType(kind.awtType))
            assertTrue(kind.label.isNotEmpty())
        }
        // A bitmap (`CUSTOM_CURSOR` = -1) or future cursor must not break the link.
        assertEquals(AwtCursorKind.DEFAULT, AwtCursorKind.fromAwtType(-1))
        assertEquals(AwtCursorKind.DEFAULT, AwtCursorKind.fromAwtType(99))
        assertEquals(AwtCursorKind.DEFAULT, AwtCursorKind.fromId("wobbly"))
        assertEquals(AwtCursorKind.DEFAULT, AwtCursorKind.fromId(null))
        assertTrue(AwtCursorKind.TEXT.isText)
        assertFalse(AwtCursorKind.HAND.isText)
        assertTrue(AwtCursorKind.SE_RESIZE.isGrip)
        assertTrue(AwtCursorKind.MOVE.isGrip)
        assertFalse(AwtCursorKind.DEFAULT.isGrip)
    }

    // ---- Chunked replies ----------------------------------------------------

    @Test
    fun repliesChunkAndReassemble() {
        for (text in listOf(
            "",
            "a",
            "12345678",
            "123456789",
            "seed: -4172144997902289642",
            "中文剪贴板内容 🙂",
        )) {
            val records = AwtControlWire.encodeReply(AwtReplyKind.CLIPBOARD, 7, text)
            assertTrue("an answer is always sent", records.isNotEmpty())
            assertTrue(records.all { AwtControlWire.isControlRecord(it) })
            val reply = AwtControlWire.decodeReply(records)
            assertEquals(AwtControlReply(AwtReplyKind.CLIPBOARD, 7, text), reply)
        }
    }

    @Test
    fun aReplyRecordCannotBeMistakenForAnAwtEvent() {
        val record = AwtControlWire.encodeReply(AwtReplyKind.PONG, 0, "").first()
        assertEquals(AwtControlWire.CONTROL_EVENT_ID, record.id)
        // Every AWT id we produce is a small positive number; the reserved id is not.
        for (id in listOf(
            AwtEventRecord.KEY_TYPED,
            AwtEventRecord.KEY_PRESSED,
            AwtEventRecord.MOUSE_PRESSED,
            AwtEventRecord.MOUSE_WHEEL,
            AwtEventRecord.COMPONENT_RESIZED,
            AwtEventRecord.FOCUS_LOST,
        )) {
            assertTrue(id < 3000)
            assertFalse(AwtControlWire.isControlRecord(AwtEventRecord(id, 0, 0, 0, 0, 0, 0, 0)))
        }
        // …and it survives the byte round trip through the event wire.
        val bytes = AwtWire.encodeEventRecords(listOf(record))
        val back = AwtWire.decodeEventRecords(bytes)!!
        assertEquals(record, back.single())
        assertEquals(
            AwtControlReply(AwtReplyKind.PONG, 0, ""),
            AwtControlWire.decodeReply(back),
        )
    }

    @Test
    fun reassemblyRejectsADamagedRun() {
        val records = AwtControlWire.encodeReply(AwtReplyKind.CLIPBOARD, 3, "hello world!")
        assertTrue(records.size >= 2)
        assertNull(AwtControlWire.decodeReply(records.dropLast(1)))
        assertNull(AwtControlWire.decodeReply(listOf(records[1], records[0])))
        assertNull(AwtControlWire.decodeReply(emptyList()))
        assertNull(AwtControlWire.decodeReply(listOf(AwtEventRecord(0, 0, 0, 0, 0, 0, 0, 0))))
        // A bogus per-chunk length.
        assertNull(AwtControlWire.decodeReply(listOf(records[0].copy(keyChar = 99))))
        // A record from another reply spliced in.
        assertNull(AwtControlWire.decodeReply(listOf(records[0], records[1].copy(y = 4))))
    }

    // ---- JSON ---------------------------------------------------------------

    @Test
    fun aBatchIsParsedWithItsProjection() {
        val batch = AwtControlBatch.parse(
            """
            {"control":[
               {"kind":"cursor","seq":0,"cursor":"text","awt_type":2},
               {"kind":"clipboard_set","seq":0,"text":"copied"},
               {"kind":"clipboard_request","seq":12},
               {"kind":"beep","seq":0},
               {"kind":"ime_show","seq":0,"x":5,"y":6,"line_height":7}
             ],
             "count":5,
             "state":{"cursor":"text","cursor_awt_type":2,"title":"Forge",
                      "ime":{"x":5,"y":6,"line_height":7},"wants_keyboard":true,
                      "clipboard_out":"copied","clipboard_requests":1,
                      "windows":[{"id":1,"title":"对话框"}],"window_count":1,
                      "beeps":2,"bye":null},
             "clipboard_requests":1}
            """.trimIndent(),
        )
        assertNull(batch.error)
        assertEquals(5, batch.messages.size)
        assertEquals(AwtControlKind.CURSOR, batch.messages[0].kind)
        assertEquals(AwtCursorKind.TEXT, batch.messages[0].cursor)
        assertEquals("copied", batch.clipboardSet)
        assertEquals(12, batch.clipboardRequestSeq)
        assertEquals(1, batch.beeps)
        assertEquals(AwtImeCaret(5, 6, 7), batch.messages[4].caret)

        val state = batch.state
        assertEquals(AwtCursorKind.TEXT, state.cursor)
        assertEquals("Forge", state.title)
        assertTrue(state.wantsKeyboard)
        assertEquals(AwtImeCaret(5, 6, 7), state.caret)
        assertEquals("copied", state.clipboardOut)
        assertEquals(1, state.clipboardRequests)
        assertEquals(listOf(AwtWindowInfo(1, "对话框")), state.windows)
        assertEquals(2L, state.beeps)
        assertNull(state.bye)
        assertTrue(state.describe().contains("文本"))
    }

    @Test
    fun anEmptyOrHostileDocumentNeverThrows() {
        assertTrue(AwtControlBatch.parse("""{"control":[],"count":0}""").isEmpty)
        assertNotNull(AwtControlBatch.parse(null).error)
        assertNotNull(AwtControlBatch.parse("").error)
        assertNotNull(AwtControlBatch.parse("not json").error)
        assertEquals("boom", AwtControlBatch.parse("""{"error":"boom"}""").error)
        // Unknown kinds and wrong types are skipped, not fatal.
        val batch = AwtControlBatch.parse(
            """{"control":[{"kind":"teleport"},{"kind":"beep"},7,"x"],"state":{"cursor":42}}""",
        )
        assertEquals(1, batch.messages.size)
        assertEquals(AwtControlKind.BEEP, batch.messages.single().kind)
        assertEquals(AwtCursorKind.DEFAULT, batch.state.cursor)
        assertFalse(batch.state.wantsKeyboard)
    }

    @Test
    fun theStateFallsBackToTheImePresenceWhenTheFlagIsAbsent() {
        val batch = AwtControlBatch.parse(
            """{"control":[],"state":{"ime":{"x":1,"y":2,"line_height":3}}}""",
        )
        assertTrue("an IME caret implies a keyboard", batch.state.wantsKeyboard)
    }

    @Test
    fun requestsSerialiseToTheJsonTheCoreExpects() {
        assertEquals("""{"clipboard":"seed 42"}""", AwtControlRequest(clipboard = "seed 42").toJson())
        assertEquals(
            """{"clipboard_empty":true}""",
            AwtControlRequest(clipboard = null, clipboardEmpty = true).toJson(),
        )
        assertEquals(
            """{"clipboard":"x","clipboard_seq":3}""",
            AwtControlRequest(clipboard = "x", clipboardSeq = 3).toJson(),
        )
        assertEquals("""{"pong":9}""", AwtControlRequest(pong = 9).toJson())
        assertEquals("""{"reset":true}""", AwtControlRequest(reset = true).toJson())
        // An empty answer wins over the text, so "no clipboard" is unambiguous.
        assertEquals(
            """{"clipboard_empty":true}""",
            AwtControlRequest(clipboard = "ignored", clipboardEmpty = true).toJson(),
        )
    }

    @Test
    fun aResultIsParsedFailSoft() {
        val ok = AwtControlResult.parse("""{"queued":3,"clipboard_requests":0,"state":{"cursor":"hand"}}""")
        assertEquals(3, ok.queued)
        assertEquals(AwtCursorKind.HAND, ok.state.cursor)
        assertNotNull(AwtControlResult.parse("}{").error)
        assertEquals("nope", AwtControlResult.parse("""{"error":"nope"}""").error)
    }

    // ---- The fake bridge ----------------------------------------------------

    @Test
    fun theFakeBridgeMirrorsTheCoreProjection() {
        val bridge = FakeAwtCanvasBridge()
        bridge.open(AwtSessionConfig(screenWidth = 32, screenHeight = 16))
        assertTrue(bridge.submitControl(AwtControlWire.encodeCursor(AwtCursorKind.HAND)))
        assertTrue(bridge.submitControl(AwtControlWire.encode(AwtControlKind.TITLE, text = "T")))
        assertTrue(bridge.submitControl(AwtControlWire.encodeImeShow(1, 2, 3)))
        assertTrue(bridge.submitControl(AwtControlWire.encodeClipboardSet("copied")))
        assertTrue(bridge.submitControl(AwtControlWire.encodeClipboardRequest(1)))
        assertFalse("garbage is refused", bridge.submitControl(ByteArray(8)))

        val batch = bridge.drainControl()
        assertEquals(5, batch.messages.size)
        assertEquals(AwtCursorKind.HAND, batch.state.cursor)
        assertEquals("T", batch.state.title)
        assertTrue(batch.state.wantsKeyboard)
        assertEquals("copied", batch.state.clipboardOut)
        assertEquals(1, batch.state.clipboardRequests)
        assertTrue("draining is destructive", bridge.drainControl().isEmpty)

        val result = bridge.control(AwtControlRequest(clipboard = "answer"))
        assertTrue(result.queued >= 1)
        assertEquals(0, result.clipboardRequests)
        assertEquals(listOf("answer"), bridge.clipboardAnswers)

        // An announced managed screen resizes the fake canvas, exactly as in the
        // core, so the UI test path sees the same geometry change.
        assertTrue(bridge.submitControl(AwtControlWire.encodeScreenSize(8, 4)))
        assertEquals(8, bridge.info().screenWidth)
        assertEquals(4, bridge.info().screenHeight)

        bridge.submitControl(AwtControlWire.encode(AwtControlKind.BYE, text = "done"))
        assertEquals("done", bridge.drainControl().state.bye)
    }

    @Test
    fun theFakeBridgeClampsTheCaretWhenTheDesktopShrinks() {
        val bridge = FakeAwtCanvasBridge()
        bridge.open(AwtSessionConfig(screenWidth = 320, screenHeight = 240))
        bridge.submitControl(AwtControlWire.encodeImeShow(300, 230, 40))
        assertEquals(AwtImeCaret(300, 230, 40), bridge.drainControl().state.caret)

        bridge.configure(AwtConfigureRequest(screenWidth = 64, screenHeight = 48))
        assertEquals(AwtImeCaret(63, 47, 40), bridge.drainControl().state.caret)

        // …and on the JVM-announced path, as in the core.
        bridge.submitControl(AwtControlWire.encodeScreenSize(16, 8))
        val caret = bridge.drainControl().state.caret
        assertEquals(AwtImeCaret(15, 7, 8), caret)
    }

    @Test
    fun theFakeBridgeRefusesTheControlPlaneWithoutASession() {
        val bridge = FakeAwtCanvasBridge()
        assertNotNull(bridge.drainControl().error)
        assertNotNull(bridge.control(AwtControlRequest(clipboard = "x")).error)
        assertFalse(bridge.submitControl(AwtControlWire.encode(AwtControlKind.BEEP)))
    }
}
