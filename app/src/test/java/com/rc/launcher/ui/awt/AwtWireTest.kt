package com.rc.launcher.ui.awt

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Unit tests for the `RCAF` frame / AWT event wire formats (task 18).
 *
 * The layout is a contract with the Rust core (`launch::awt`), so these tests
 * assert the *bytes*, not just the round trip: a silent field reorder on either
 * side would otherwise show up as a garbled desktop on a device.
 */
class AwtWireTest {

    private val red = 0xFFFF0000.toInt()
    private val blue = 0xFF0000FF.toInt()

    @Test
    fun aFullFrameHasTheDocumentedHeaderLayout() {
        val bytes = AwtWire.encodeFrame(7, 4, 2, IntArray(8) { red })
        assertEquals(AwtWire.FRAME_HEADER_LEN + 8 * 4, bytes.size)
        // magic "RCAF" little-endian, version, format
        assertArrayEquals(byteArrayOf('F'.code.toByte(), 'A'.code.toByte(), 'C'.code.toByte(), 'R'.code.toByte()), bytes.copyOfRange(0, 4))
        assertEquals(1, bytes[4].toInt())
        assertEquals(0, bytes[6].toInt()) // FORMAT_ARGB
        assertEquals(7, bytes[8].toInt()) // seq
        assertEquals(4, bytes[12].toInt()) // width
        assertEquals(2, bytes[14].toInt()) // height
        assertEquals(32, bytes[24].toInt()) // payload bytes
        assertEquals(1, bytes[28].toInt()) // flags: full frame
        // First pixel, little-endian ARGB.
        assertArrayEquals(byteArrayOf(0, 0, 0xFF.toByte(), 0xFF.toByte()), bytes.copyOfRange(32, 36))
    }

    @Test
    fun headerAndPixelsRoundTrip() {
        val pixels = IntArray(8) { if (it % 2 == 0) red else blue }
        val header = AwtWire.decodeFrameHeader(AwtWire.encodeFrame(3, 4, 2, pixels))
        assertNotNull(header)
        assertEquals(3, header!!.seq)
        assertEquals(4, header.width)
        assertEquals(2, header.height)
        assertEquals(AwtRect(0, 0, 4, 2), header.damage)
        assertTrue(header.full)
        assertArrayEquals(pixels, AwtWire.decodeFramePixels(AwtWire.encodeFrame(3, 4, 2, pixels)))
    }

    @Test
    fun aPartialFrameCarriesOnlyItsDamage() {
        val damage = AwtRect(1, 0, 2, 2)
        val bytes = AwtWire.encodeFrame(1, 4, 2, IntArray(4) { blue }, damage)
        val header = AwtWire.decodeFrameHeader(bytes)!!
        assertEquals(damage, header.damage)
        assertFalse(header.full)
        assertEquals(16, header.payloadLen)
        assertEquals(4, AwtWire.decodeFramePixels(bytes)!!.size)
    }

    @Test
    fun encodingValidatesEverythingTheCoreWould() {
        // payload size must match the damage area
        assertThrows { AwtWire.encodeFrame(1, 4, 2, IntArray(7) { red }) }
        // empty damage
        assertThrows { AwtWire.encodeFrame(1, 4, 2, IntArray(0), AwtRect(0, 0, 0, 0)) }
        // damage outside the desktop
        assertThrows { AwtWire.encodeFrame(1, 4, 2, IntArray(4) { red }, AwtRect(3, 0, 2, 2)) }
        // negative origin
        assertThrows { AwtWire.encodeFrame(1, 4, 2, IntArray(4) { red }, AwtRect(-1, 0, 2, 2)) }
        // absurd dimensions
        assertThrows { AwtWire.encodeFrame(1, 0, 2, IntArray(0)) }
        assertThrows { AwtWire.encodeFrame(1, 99999, 2, IntArray(2)) }
    }

    @Test
    fun decodingRejectsCorruptFramesWithoutThrowing() {
        val good = AwtWire.encodeFrame(1, 4, 2, IntArray(8) { red })
        assertNull("truncated", AwtWire.decodeFrameHeader(good.copyOfRange(0, 16)))
        assertNull("empty", AwtWire.decodeFrameHeader(ByteArray(0)))

        val badMagic = good.copyOf()
        badMagic[0] = (badMagic[0].toInt() xor 0xFF).toByte()
        assertNull("bad magic", AwtWire.decodeFrameHeader(badMagic))

        val badVersion = good.copyOf()
        badVersion[4] = 9
        assertNull("bad version", AwtWire.decodeFrameHeader(badVersion))

        val badFormat = good.copyOf()
        badFormat[6] = 7
        assertNull("unknown pixel format", AwtWire.decodeFrameHeader(badFormat))

        val badWidth = good.copyOf()
        badWidth[12] = 0
        badWidth[13] = 0
        assertNull("zero width", AwtWire.decodeFrameHeader(badWidth))

        val shortPayload = good.copyOfRange(0, AwtWire.FRAME_HEADER_LEN + 8)
        assertNull("payload shorter than the header claims", AwtWire.decodeFrameHeader(shortPayload))
        assertNull(AwtWire.decodeFramePixels(shortPayload))
    }

    @Test
    fun eventRecordsRoundTripAsThirtyTwoByteRows() {
        val records = listOf(
            AwtEventRecord(AwtEventRecord.MOUSE_PRESSED, 12, 34, 1, 0, 0xFFFF, 1 shl 10, 0),
            AwtEventRecord(AwtEventRecord.KEY_PRESSED, 0, 0, 0, 27, 0xFFFF, 0, 0),
        )
        val bytes = AwtWire.encodeEventRecords(records)
        assertEquals(2 * AwtWire.EVENT_RECORD_LEN, bytes.size)
        assertEquals(records, AwtWire.decodeEventRecords(bytes))
        assertEquals(emptyList<AwtEventRecord>(), AwtWire.decodeEventRecords(ByteArray(0)))
        assertNull("not a record multiple", AwtWire.decodeEventRecords(ByteArray(20)))
        assertEquals("MOUSE_PRESSED", AwtEventRecord.nameOf(AwtEventRecord.MOUSE_PRESSED))
        assertEquals("EVENT_777", AwtEventRecord.nameOf(777))
    }

    @Test
    fun theSelfTestPatternIsOpaqueBorderedAndDeterministic() {
        val pattern = AwtWire.testPattern(8, 4)
        assertEquals(32, pattern.size)
        assertTrue("every pixel opaque", pattern.all { (it ushr 24) and 0xFF == 0xFF })
        assertEquals(0xFFFFFFFF.toInt(), pattern[0]) // top-left is border
        assertEquals(0xFFFFFFFF.toInt(), pattern[pattern.size - 1]) // bottom-right too
        assertArrayEquals(pattern, AwtWire.testPattern(8, 4))
        // The interior is not uniform (a wrong stride would be visible).
        assertTrue(pattern.toSet().size > 2)
        assertThrows { AwtWire.testPattern(0, 4) }
    }

    private fun assertThrows(block: () -> Unit) {
        try {
            block()
        } catch (expected: IllegalArgumentException) {
            return
        }
        throw AssertionError("expected an IllegalArgumentException")
    }
}
