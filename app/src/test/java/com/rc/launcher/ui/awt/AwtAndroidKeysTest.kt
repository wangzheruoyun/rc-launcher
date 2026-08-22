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
}
