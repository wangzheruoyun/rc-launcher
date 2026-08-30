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
            AwtPointerEvent(
                AwtPointerPhase.UP,
                11f,
                21f,
                AwtMouseButton.MIDDLE,
                AwtPointerSource.MOUSE,
            ),
        ).toBatchJson()
        assertEquals(
            "{\"events\":[" +
                "{\"type\":\"pointer\",\"phase\":\"down\",\"x\":10,\"y\":20," +
                "\"button\":\"left\",\"source\":\"touch\"}," +
                "{\"type\":\"pointer\",\"phase\":\"move\",\"x\":10.5,\"y\":20," +
                "\"button\":\"right\",\"source\":\"touch\"}," +
                "{\"type\":\"pointer\",\"phase\":\"up\",\"x\":11,\"y\":21," +
                "\"button\":\"middle\",\"source\":\"mouse\"}]}",
            json,
        )
    }

    // ---- Physical keyboard & mouse (task 12) -------------------------------

    @Test
    fun relativeMotionIsEncodedWithoutAPosition() {
        assertEquals(
            "{\"events\":[{\"type\":\"pointer_relative\",\"dx\":-3.5,\"dy\":2," +
                "\"source\":\"mouse\"}]}",
            AwtRelativePointerEvent(-3.5f, 2f).toBatchJson(),
        )
        // A non-finite delta cannot reach the core as `NaN` (the core would drop
        // it, but the JSON would not even parse in a strict reader).
        assertEquals(
            "{\"events\":[{\"type\":\"pointer_relative\",\"dx\":0,\"dy\":0," +
                "\"source\":\"stylus\"}]}",
            AwtRelativePointerEvent(Float.NaN, Float.POSITIVE_INFINITY, AwtPointerSource.STYLUS)
                .toBatchJson(),
        )
    }

    @Test
    fun captureButtonAndScrollAtPointerAreEncoded() {
        assertEquals(
            "{\"events\":[{\"type\":\"capture\",\"captured\":true}]}",
            AwtCaptureEvent(true).toBatchJson(),
        )
        assertEquals(
            "{\"events\":[{\"type\":\"button\",\"button\":\"right\",\"down\":false}]}",
            AwtButtonEvent(AwtMouseButton.RIGHT, down = false).toBatchJson(),
        )
        // No coordinates at all: the core scrolls wherever the pointer is.
        assertEquals(
            "{\"events\":[{\"type\":\"scroll\",\"ticks\":-2}]}",
            AwtScrollAtPointerEvent(-2).toBatchJson(),
        )
    }

    @Test
    fun aKeyCarriesThePhysicalScancodeWhenThereIsOne() {
        assertEquals(
            "{\"events\":[{\"type\":\"key_down\",\"name\":\"w\",\"scancode\":17}]}",
            AwtKeyEvent(down = true, name = "w", scancode = 17).toBatchJson(),
        )
        // 0 / negative means "Android did not tell us": omit it so the core fills
        // it in from its own table instead of forwarding a bogus 0.
        assertEquals(
            "{\"events\":[{\"type\":\"key_down\",\"name\":\"w\"}]}",
            AwtKeyEvent(down = true, name = "w", scancode = 0).toBatchJson(),
        )
        assertEquals(
            "{\"events\":[{\"type\":\"key_up\",\"name\":\"w\",\"scancode\":17}]}",
            AwtKeyEvent(down = false, name = "w", scancode = 17).toBatchJson(),
        )
    }

    @Test
    fun pointerSourcesMirrorTheCore() {
        assertEquals(AwtPointerSource.MOUSE, AwtPointerSource.fromId("mouse"))
        assertEquals(AwtPointerSource.STYLUS, AwtPointerSource.fromId("stylus"))
        // Unknown degrades to touch, exactly as `PointerSource::from_id` does.
        assertEquals(AwtPointerSource.TOUCH, AwtPointerSource.fromId("nonsense"))
        assertEquals(AwtPointerSource.TOUCH, AwtPointerSource.fromId(null))
        assertTrue(AwtPointerSource.MOUSE.isMouseLike)
        assertTrue(AwtPointerSource.STYLUS.isMouseLike)
        assertTrue(!AwtPointerSource.TOUCH.isMouseLike)
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
