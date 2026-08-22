package com.rc.launcher.ui.awt

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Unit tests for the Compose → AWT input encoding (task 18). */
class AwtInputTest {

    @Test
    fun aPointerBatchIsEncodedForTheCore() {
        val json = listOf<AwtInputEvent>(
            AwtPointerEvent(AwtPointerPhase.DOWN, 10f, 20f),
            AwtPointerEvent(AwtPointerPhase.MOVE, 10.5f, 20f, AwtMouseButton.RIGHT),
            AwtPointerEvent(AwtPointerPhase.UP, 11f, 21f, AwtMouseButton.MIDDLE),
        ).toBatchJson()
        assertEquals(
            "{\"events\":[" +
                "{\"type\":\"pointer\",\"phase\":\"down\",\"x\":10,\"y\":20,\"button\":\"left\"}," +
                "{\"type\":\"pointer\",\"phase\":\"move\",\"x\":10.5,\"y\":20,\"button\":\"right\"}," +
                "{\"type\":\"pointer\",\"phase\":\"up\",\"x\":11,\"y\":21,\"button\":\"middle\"}]}",
            json,
        )
    }

    @Test
    fun scrollKeysTextFocusAndReleaseAllAreEncoded() {
        assertEquals(
            "{\"events\":[{\"type\":\"scroll\",\"x\":1,\"y\":2,\"ticks\":-3}]}",
            AwtScrollEvent(1f, 2f, -3).toBatchJson(),
        )
        assertEquals(
            "{\"events\":[{\"type\":\"key_down\",\"code\":27}]}",
            AwtKeyEvent(down = true, code = 27).toBatchJson(),
        )
        assertEquals(
            "{\"events\":[{\"type\":\"key_up\",\"name\":\"left.shift\"}]}",
            AwtKeyEvent(down = false, name = AwtKeyNames.SHIFT).toBatchJson(),
        )
        assertEquals(
            "{\"events\":[{\"type\":\"focus\",\"gained\":false}]}",
            AwtFocusEvent(false).toBatchJson(),
        )
        assertEquals(
            "{\"events\":[{\"type\":\"release_all\"}]}",
            AwtReleaseAllEvent.toBatchJson(),
        )
        assertEquals(
            "{\"events\":[{\"type\":\"reset_input\"}]}",
            AwtResetInputEvent.toBatchJson(),
        )
    }

    @Test
    fun textIsJsonEscapedSoAnImeCannotBreakTheWire() {
        val json = AwtTextEvent("a\"b\\c\n").toBatchJson()
        assertTrue(json, json.contains("\\\"") && json.contains("\\\\") && json.contains("\\n"))
        // A code point outside the BMP survives as-is (surrogate pair).
        assertTrue(AwtTextEvent("\uD83D\uDE00").toBatchJson().contains("\uD83D\uDE00"))
    }

    @Test
    fun aKeyEventNeedsACodeOrAName() {
        try {
            AwtKeyEvent(down = true)
            throw AssertionError("expected an IllegalArgumentException")
        } catch (expected: IllegalArgumentException) {
            // expected
        }
    }

    @Test
    fun nonFinitePointerCoordinatesAreNeutralised() {
        val json = AwtPointerEvent(AwtPointerPhase.MOVE, Float.NaN, Float.POSITIVE_INFINITY).toBatchJson()
        assertEquals(
            "{\"events\":[{\"type\":\"pointer\",\"phase\":\"move\",\"x\":0,\"y\":0,\"button\":\"left\"}]}",
            json,
        )
    }

    @Test
    fun anEmptyBatchIsStillValidJson() {
        assertEquals("{\"events\":[]}", emptyList<AwtInputEvent>().toBatchJson())
    }

    @Test
    fun mouseButtonsUseAwtNumbering() {
        assertEquals(1, AwtMouseButton.LEFT.number)
        assertEquals(2, AwtMouseButton.MIDDLE.number)
        assertEquals(3, AwtMouseButton.RIGHT.number)
        assertEquals(AwtMouseButton.RIGHT, AwtMouseButton.fromNumber(3))
        assertEquals(AwtMouseButton.LEFT, AwtMouseButton.fromNumber(9))
    }

    @Test
    fun keyNamesForPrintableCharacters() {
        assertEquals("w", AwtKeyNames.forChar('W'))
        assertEquals("3", AwtKeyNames.forChar('3'))
        assertEquals(AwtKeyNames.SPACE, AwtKeyNames.forChar(' '))
        assertEquals(null, AwtKeyNames.forChar('中'))
    }
}
