package com.rc.launcher.ui.awt

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * Unit tests for the Android keycode → AWT key-name mapping (task 18).
 *
 * `KeyEvent.KEYCODE_*` are compile-time constants, so the raw values below are
 * exactly what the framework delivers — asserting them keeps the mapping honest
 * without needing an instrumented test.
 */
class AwtAndroidKeysTest {

    @Test
    fun lettersAndDigitsMapToTheirCharacter() {
        assertEquals("a", awtKeyNameForAndroidKeyCode(29)) // KEYCODE_A
        assertEquals("w", awtKeyNameForAndroidKeyCode(51)) // KEYCODE_W
        assertEquals("z", awtKeyNameForAndroidKeyCode(54)) // KEYCODE_Z
        assertEquals("0", awtKeyNameForAndroidKeyCode(7)) // KEYCODE_0
        assertEquals("9", awtKeyNameForAndroidKeyCode(16)) // KEYCODE_9
    }

    @Test
    fun functionAndKeypadKeys() {
        assertEquals("f1", awtKeyNameForAndroidKeyCode(131))
        assertEquals("f12", awtKeyNameForAndroidKeyCode(142))
        assertEquals("keypad.0", awtKeyNameForAndroidKeyCode(144))
        assertEquals("keypad.9", awtKeyNameForAndroidKeyCode(153))
        assertEquals("keypad.enter", awtKeyNameForAndroidKeyCode(160))
        assertEquals("keypad.decimal", awtKeyNameForAndroidKeyCode(158))
    }

    @Test
    fun navigationAndEditingKeys() {
        assertEquals(AwtKeyNames.ENTER, awtKeyNameForAndroidKeyCode(66))
        assertEquals(AwtKeyNames.BACKSPACE, awtKeyNameForAndroidKeyCode(67))
        assertEquals(AwtKeyNames.SPACE, awtKeyNameForAndroidKeyCode(62))
        assertEquals(AwtKeyNames.TAB, awtKeyNameForAndroidKeyCode(61))
        assertEquals(AwtKeyNames.LEFT, awtKeyNameForAndroidKeyCode(21))
        assertEquals(AwtKeyNames.RIGHT, awtKeyNameForAndroidKeyCode(22))
        assertEquals(AwtKeyNames.UP, awtKeyNameForAndroidKeyCode(19))
        assertEquals(AwtKeyNames.DOWN, awtKeyNameForAndroidKeyCode(20))
    }

    @Test
    fun backAndEscapeBothCloseADialog() {
        // Android's hardware/gesture "back" is the natural Escape on a phone.
        assertEquals(AwtKeyNames.ESCAPE, awtKeyNameForAndroidKeyCode(4)) // KEYCODE_BACK
        assertEquals(AwtKeyNames.ESCAPE, awtKeyNameForAndroidKeyCode(111)) // KEYCODE_ESCAPE
    }

    @Test
    fun leftAndRightModifiersCollapseOntoOneAwtCode() {
        assertEquals(AwtKeyNames.SHIFT, awtKeyNameForAndroidKeyCode(59))
        assertEquals(AwtKeyNames.SHIFT, awtKeyNameForAndroidKeyCode(60))
        assertEquals(AwtKeyNames.CONTROL, awtKeyNameForAndroidKeyCode(113))
        assertEquals(AwtKeyNames.CONTROL, awtKeyNameForAndroidKeyCode(114))
        assertEquals(AwtKeyNames.ALT, awtKeyNameForAndroidKeyCode(57))
        assertEquals(AwtKeyNames.META, awtKeyNameForAndroidKeyCode(117))
    }

    @Test
    fun punctuationIsNamedTheWayTheCoreExpects() {
        assertEquals("comma", awtKeyNameForAndroidKeyCode(55))
        assertEquals("period", awtKeyNameForAndroidKeyCode(56))
        assertEquals("slash", awtKeyNameForAndroidKeyCode(76))
        assertEquals("grave.accent", awtKeyNameForAndroidKeyCode(68))
        assertEquals("left.bracket", awtKeyNameForAndroidKeyCode(71))
    }

    @Test
    fun unknownKeysDegradeToTypedText() {
        assertNull(awtKeyNameForAndroidKeyCode(0)) // KEYCODE_UNKNOWN
        assertNull(awtKeyNameForAndroidKeyCode(24)) // KEYCODE_VOLUME_UP
        assertNull(awtKeyNameForAndroidKeyCode(-5))
    }

    // ---- Physical keyboard & mouse (task 12) -------------------------------

    @Test
    fun scancodesAreTheEvdevNumbersTheGameExpects() {
        // Android keycodes: A=29, W=51, Z=54, 0=7, 1=8, ESCAPE=111, SPACE=62,
        // SHIFT_LEFT=59, SHIFT_RIGHT=60, F5=135, NUMPAD_0=144.
        assertEquals(30, awtScancodeForAndroidKeyCode(29)) // evdev KEY_A
        assertEquals(17, awtScancodeForAndroidKeyCode(51)) // KEY_W
        assertEquals(44, awtScancodeForAndroidKeyCode(54)) // KEY_Z
        assertEquals(11, awtScancodeForAndroidKeyCode(7)) // KEY_0 is 11, not 1
        assertEquals(2, awtScancodeForAndroidKeyCode(8)) // KEY_1
        assertEquals(1, awtScancodeForAndroidKeyCode(111)) // KEY_ESC
        assertEquals(57, awtScancodeForAndroidKeyCode(62)) // KEY_SPACE
        assertEquals(42, awtScancodeForAndroidKeyCode(59)) // KEY_LEFTSHIFT
        assertEquals(54, awtScancodeForAndroidKeyCode(60)) // KEY_RIGHTSHIFT
        assertEquals(63, awtScancodeForAndroidKeyCode(135)) // KEY_F5
        assertEquals(82, awtScancodeForAndroidKeyCode(144)) // KEY_KP0
        // Unknown means "no scancode": the core then fills one in.
        assertEquals(0, awtScancodeForAndroidKeyCode(0))
        assertEquals(0, awtScancodeForAndroidKeyCode(24)) // KEYCODE_VOLUME_UP
    }

    @Test
    fun theGameKeyNameKeepsTheSideAwtThrowsAway() {
        // AWT has one code for both shifts; GLFW (and Minecraft) do not.
        assertEquals("left.shift", awtKeyNameForAndroidKeyCode(60))
        assertEquals("right.shift", gameKeyNameForAndroidKeyCode(60))
        assertEquals("left.shift", gameKeyNameForAndroidKeyCode(59))
        assertEquals("right.control", gameKeyNameForAndroidKeyCode(114))
        assertEquals("right.alt", gameKeyNameForAndroidKeyCode(58))
        assertEquals("menu", gameKeyNameForAndroidKeyCode(82))
        // Everything else is the AWT name, so one table stays authoritative.
        assertEquals("w", gameKeyNameForAndroidKeyCode(51))
        assertNull(gameKeyNameForAndroidKeyCode(24))
    }

    @Test
    fun aButtonStateBitmaskBecomesAwtButtons() {
        assertEquals(emptySet<AwtMouseButton>(), awtMouseButtonsForButtonState(0))
        assertEquals(setOf(AwtMouseButton.LEFT), awtMouseButtonsForButtonState(1))
        assertEquals(setOf(AwtMouseButton.RIGHT), awtMouseButtonsForButtonState(2))
        assertEquals(setOf(AwtMouseButton.MIDDLE), awtMouseButtonsForButtonState(4))
        assertEquals(
            setOf(AwtMouseButton.LEFT, AwtMouseButton.RIGHT),
            awtMouseButtonsForButtonState(1 or 2),
        )
        // Stylus buttons follow the pen convention; back / forward fold onto the
        // middle button instead of being swallowed.
        assertEquals(setOf(AwtMouseButton.LEFT), awtMouseButtonsForButtonState(32))
        assertEquals(setOf(AwtMouseButton.RIGHT), awtMouseButtonsForButtonState(64))
        assertEquals(setOf(AwtMouseButton.MIDDLE), awtMouseButtonsForButtonState(8))
    }

    @Test
    fun theToolTypeTellsUsWhichDeviceItWas() {
        assertEquals(AwtPointerSource.TOUCH, awtPointerSourceForToolType(1)) // FINGER
        assertEquals(AwtPointerSource.STYLUS, awtPointerSourceForToolType(2)) // STYLUS
        assertEquals(AwtPointerSource.MOUSE, awtPointerSourceForToolType(3)) // MOUSE
        assertEquals(AwtPointerSource.STYLUS, awtPointerSourceForToolType(4)) // ERASER
        assertEquals(AwtPointerSource.TOUCH, awtPointerSourceForToolType(0)) // UNKNOWN
    }
}
