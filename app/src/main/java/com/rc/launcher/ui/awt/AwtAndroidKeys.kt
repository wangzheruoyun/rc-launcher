package com.rc.launcher.ui.awt

import android.view.KeyEvent as AndroidKeyEvent
import android.view.MotionEvent as AndroidMotionEvent

/**
 * Android key codes → the key *names* the Rust core resolves into
 * `java.awt.event.KeyEvent.VK_*` codes (task 18).
 *
 * Naming a key instead of hard-coding a VK number keeps one translation table in
 * the core (shared with the task-15 control layouts, which store the same
 * Minecraft/GLFW-style names) and lets the UI stay a one-liner. Unknown keys
 * return `null`, and the caller then forwards the character as typed text — so an
 * exotic layout still reaches the game instead of injecting a bogus key.
 *
 * The `KEYCODE_*` values are compile-time constants, so this file is
 * unit-testable on the JVM even though it names an Android class.
 */
fun awtKeyNameForAndroidKeyCode(keyCode: Int): String? = when (keyCode) {
    // letters -------------------------------------------------------------
    in AndroidKeyEvent.KEYCODE_A..AndroidKeyEvent.KEYCODE_Z ->
        ('a' + (keyCode - AndroidKeyEvent.KEYCODE_A)).toString()
    // digits --------------------------------------------------------------
    in AndroidKeyEvent.KEYCODE_0..AndroidKeyEvent.KEYCODE_9 ->
        ('0' + (keyCode - AndroidKeyEvent.KEYCODE_0)).toString()
    // function keys -------------------------------------------------------
    in AndroidKeyEvent.KEYCODE_F1..AndroidKeyEvent.KEYCODE_F12 ->
        "f" + (keyCode - AndroidKeyEvent.KEYCODE_F1 + 1)
    // numeric keypad ------------------------------------------------------
    in AndroidKeyEvent.KEYCODE_NUMPAD_0..AndroidKeyEvent.KEYCODE_NUMPAD_9 ->
        "keypad." + (keyCode - AndroidKeyEvent.KEYCODE_NUMPAD_0)
    AndroidKeyEvent.KEYCODE_NUMPAD_ENTER -> "keypad.enter"
    AndroidKeyEvent.KEYCODE_NUMPAD_ADD -> "keypad.add"
    AndroidKeyEvent.KEYCODE_NUMPAD_SUBTRACT -> "keypad.subtract"
    AndroidKeyEvent.KEYCODE_NUMPAD_MULTIPLY -> "keypad.multiply"
    AndroidKeyEvent.KEYCODE_NUMPAD_DIVIDE -> "keypad.divide"
    AndroidKeyEvent.KEYCODE_NUMPAD_DOT -> "keypad.decimal"
    // editing / navigation ------------------------------------------------
    AndroidKeyEvent.KEYCODE_ENTER -> AwtKeyNames.ENTER
    AndroidKeyEvent.KEYCODE_ESCAPE, AndroidKeyEvent.KEYCODE_BACK -> AwtKeyNames.ESCAPE
    AndroidKeyEvent.KEYCODE_TAB -> AwtKeyNames.TAB
    AndroidKeyEvent.KEYCODE_DEL -> AwtKeyNames.BACKSPACE
    AndroidKeyEvent.KEYCODE_FORWARD_DEL -> AwtKeyNames.DELETE
    AndroidKeyEvent.KEYCODE_INSERT -> "insert"
    AndroidKeyEvent.KEYCODE_SPACE -> AwtKeyNames.SPACE
    AndroidKeyEvent.KEYCODE_DPAD_LEFT -> AwtKeyNames.LEFT
    AndroidKeyEvent.KEYCODE_DPAD_RIGHT -> AwtKeyNames.RIGHT
    AndroidKeyEvent.KEYCODE_DPAD_UP -> AwtKeyNames.UP
    AndroidKeyEvent.KEYCODE_DPAD_DOWN -> AwtKeyNames.DOWN
    AndroidKeyEvent.KEYCODE_MOVE_HOME -> AwtKeyNames.HOME
    AndroidKeyEvent.KEYCODE_MOVE_END -> AwtKeyNames.END
    AndroidKeyEvent.KEYCODE_PAGE_UP -> AwtKeyNames.PAGE_UP
    AndroidKeyEvent.KEYCODE_PAGE_DOWN -> AwtKeyNames.PAGE_DOWN
    // modifiers (left / right collapse onto one AWT code, as AWT does) -----
    AndroidKeyEvent.KEYCODE_SHIFT_LEFT, AndroidKeyEvent.KEYCODE_SHIFT_RIGHT -> AwtKeyNames.SHIFT
    AndroidKeyEvent.KEYCODE_CTRL_LEFT, AndroidKeyEvent.KEYCODE_CTRL_RIGHT -> AwtKeyNames.CONTROL
    AndroidKeyEvent.KEYCODE_ALT_LEFT, AndroidKeyEvent.KEYCODE_ALT_RIGHT -> AwtKeyNames.ALT
    AndroidKeyEvent.KEYCODE_META_LEFT, AndroidKeyEvent.KEYCODE_META_RIGHT -> AwtKeyNames.META
    AndroidKeyEvent.KEYCODE_CAPS_LOCK -> "caps.lock"
    // punctuation ---------------------------------------------------------
    AndroidKeyEvent.KEYCODE_COMMA -> "comma"
    AndroidKeyEvent.KEYCODE_PERIOD -> "period"
    AndroidKeyEvent.KEYCODE_SLASH -> "slash"
    AndroidKeyEvent.KEYCODE_BACKSLASH -> "backslash"
    AndroidKeyEvent.KEYCODE_SEMICOLON -> "semicolon"
    AndroidKeyEvent.KEYCODE_EQUALS -> "equal"
    AndroidKeyEvent.KEYCODE_MINUS -> "minus"
    AndroidKeyEvent.KEYCODE_LEFT_BRACKET -> "left.bracket"
    AndroidKeyEvent.KEYCODE_RIGHT_BRACKET -> "right.bracket"
    AndroidKeyEvent.KEYCODE_GRAVE -> "grave.accent"
    AndroidKeyEvent.KEYCODE_APOSTROPHE -> "apostrophe"
    else -> null
}

/**
 * Which mouse buttons a `MotionEvent.getButtonState()` bitmask holds (task 12).
 *
 * Android reports the *state*, never the transition, so the caller
 * ([AwtMouseTracker]) diffs consecutive states to find the press and the release.
 * Extra buttons are folded onto the three AWT knows: back / forward become a
 * middle click (nothing in Minecraft binds them, and swallowing them silently
 * would look broken), and the stylus buttons follow their pen convention.
 *
 * The `BUTTON_*` values are compile-time constants, so this stays unit-testable
 * on the JVM.
 */
fun awtMouseButtonsForButtonState(buttonState: Int): Set<AwtMouseButton> {
    if (buttonState == 0) return emptySet()
    val out = LinkedHashSet<AwtMouseButton>(3)
    if (buttonState and AndroidMotionEvent.BUTTON_PRIMARY != 0) out += AwtMouseButton.LEFT
    if (buttonState and AndroidMotionEvent.BUTTON_STYLUS_PRIMARY != 0) out += AwtMouseButton.LEFT
    if (buttonState and AndroidMotionEvent.BUTTON_SECONDARY != 0) out += AwtMouseButton.RIGHT
    if (buttonState and AndroidMotionEvent.BUTTON_STYLUS_SECONDARY != 0) out += AwtMouseButton.RIGHT
    if (buttonState and AndroidMotionEvent.BUTTON_TERTIARY != 0) out += AwtMouseButton.MIDDLE
    if (buttonState and AndroidMotionEvent.BUTTON_BACK != 0) out += AwtMouseButton.MIDDLE
    if (buttonState and AndroidMotionEvent.BUTTON_FORWARD != 0) out += AwtMouseButton.MIDDLE
    return out
}

/**
 * Which device a `MotionEvent.getToolType()` describes (task 12).
 *
 * A mouse and a stylus have a hover position and may drive the captured pointer;
 * a finger does not, which is exactly the distinction the core filters on.
 */
fun awtPointerSourceForToolType(toolType: Int): AwtPointerSource = when (toolType) {
    AndroidMotionEvent.TOOL_TYPE_MOUSE -> AwtPointerSource.MOUSE
    AndroidMotionEvent.TOOL_TYPE_STYLUS, AndroidMotionEvent.TOOL_TYPE_ERASER -> AwtPointerSource.STYLUS
    else -> AwtPointerSource.TOUCH
}

/**
 * The Linux **evdev** scancode of an Android key code (`0` when unknown).
 *
 * Only a fallback: a real hardware key arrives with `KeyEvent.getScanCode()`,
 * which *is* the evdev code, and forwarding that verbatim is what makes a non-US
 * layout work. This table covers the synthetic presses instead — an on-screen
 * control-layout button, a gamepad mapped to a key, an IME that reports `0` —
 * because `GLFWKeyCallback` still wants a scancode and Minecraft falls back to it
 * for anything its key enumeration does not cover.
 *
 * Mirrors `launch::input::scancode_for_glfw_key`; the two are compared by
 * `scripts/check_awt_wire.py`.
 */
fun awtScancodeForAndroidKeyCode(keyCode: Int): Int = when (keyCode) {
    // letters: evdev is laid out by QWERTY position, not alphabetically ---------
    in AndroidKeyEvent.KEYCODE_A..AndroidKeyEvent.KEYCODE_Z ->
        LETTER_SCANCODES[keyCode - AndroidKeyEvent.KEYCODE_A]
    // digits: KEY_1..KEY_9 = 2..10, KEY_0 = 11 ---------------------------------
    AndroidKeyEvent.KEYCODE_0 -> 11
    in AndroidKeyEvent.KEYCODE_1..AndroidKeyEvent.KEYCODE_9 ->
        2 + (keyCode - AndroidKeyEvent.KEYCODE_1)
    // function keys -----------------------------------------------------------
    in AndroidKeyEvent.KEYCODE_F1..AndroidKeyEvent.KEYCODE_F10 ->
        59 + (keyCode - AndroidKeyEvent.KEYCODE_F1)
    AndroidKeyEvent.KEYCODE_F11 -> 87
    AndroidKeyEvent.KEYCODE_F12 -> 88
    // numeric keypad (not contiguous in evdev) --------------------------------
    in AndroidKeyEvent.KEYCODE_NUMPAD_0..AndroidKeyEvent.KEYCODE_NUMPAD_9 ->
        KEYPAD_SCANCODES[keyCode - AndroidKeyEvent.KEYCODE_NUMPAD_0]
    AndroidKeyEvent.KEYCODE_NUMPAD_ENTER -> 96
    AndroidKeyEvent.KEYCODE_NUMPAD_ADD -> 78
    AndroidKeyEvent.KEYCODE_NUMPAD_SUBTRACT -> 74
    AndroidKeyEvent.KEYCODE_NUMPAD_MULTIPLY -> 55
    AndroidKeyEvent.KEYCODE_NUMPAD_DIVIDE -> 98
    AndroidKeyEvent.KEYCODE_NUMPAD_DOT -> 83
    // editing / navigation ----------------------------------------------------
    AndroidKeyEvent.KEYCODE_ESCAPE, AndroidKeyEvent.KEYCODE_BACK -> 1
    AndroidKeyEvent.KEYCODE_MINUS -> 12
    AndroidKeyEvent.KEYCODE_EQUALS -> 13
    AndroidKeyEvent.KEYCODE_DEL -> 14
    AndroidKeyEvent.KEYCODE_TAB -> 15
    AndroidKeyEvent.KEYCODE_LEFT_BRACKET -> 26
    AndroidKeyEvent.KEYCODE_RIGHT_BRACKET -> 27
    AndroidKeyEvent.KEYCODE_ENTER -> 28
    AndroidKeyEvent.KEYCODE_SEMICOLON -> 39
    AndroidKeyEvent.KEYCODE_APOSTROPHE -> 40
    AndroidKeyEvent.KEYCODE_GRAVE -> 41
    AndroidKeyEvent.KEYCODE_BACKSLASH -> 43
    AndroidKeyEvent.KEYCODE_COMMA -> 51
    AndroidKeyEvent.KEYCODE_PERIOD -> 52
    AndroidKeyEvent.KEYCODE_SLASH -> 53
    AndroidKeyEvent.KEYCODE_SPACE -> 57
    AndroidKeyEvent.KEYCODE_CAPS_LOCK -> 58
    AndroidKeyEvent.KEYCODE_NUM_LOCK -> 69
    AndroidKeyEvent.KEYCODE_SCROLL_LOCK -> 70
    AndroidKeyEvent.KEYCODE_MOVE_HOME -> 102
    AndroidKeyEvent.KEYCODE_DPAD_UP -> 103
    AndroidKeyEvent.KEYCODE_PAGE_UP -> 104
    AndroidKeyEvent.KEYCODE_DPAD_LEFT -> 105
    AndroidKeyEvent.KEYCODE_DPAD_RIGHT -> 106
    AndroidKeyEvent.KEYCODE_MOVE_END -> 107
    AndroidKeyEvent.KEYCODE_DPAD_DOWN -> 108
    AndroidKeyEvent.KEYCODE_PAGE_DOWN -> 109
    AndroidKeyEvent.KEYCODE_INSERT -> 110
    AndroidKeyEvent.KEYCODE_FORWARD_DEL -> 111
    AndroidKeyEvent.KEYCODE_BREAK -> 119
    // modifiers (the side matters here, unlike in AWT) ------------------------
    AndroidKeyEvent.KEYCODE_CTRL_LEFT -> 29
    AndroidKeyEvent.KEYCODE_SHIFT_LEFT -> 42
    AndroidKeyEvent.KEYCODE_SHIFT_RIGHT -> 54
    AndroidKeyEvent.KEYCODE_ALT_LEFT -> 56
    AndroidKeyEvent.KEYCODE_CTRL_RIGHT -> 97
    AndroidKeyEvent.KEYCODE_ALT_RIGHT -> 100
    AndroidKeyEvent.KEYCODE_META_LEFT -> 125
    AndroidKeyEvent.KEYCODE_META_RIGHT -> 126
    AndroidKeyEvent.KEYCODE_MENU -> 127
    AndroidKeyEvent.KEYCODE_SYSRQ -> 99
    else -> 0
}

/**
 * The **side-aware** key name of an Android key code (task 12).
 *
 * [awtKeyNameForAndroidKeyCode] collapses left and right modifiers because AWT
 * has one code for both. GLFW does not, and Minecraft's key bindings can tell
 * `left.shift` from `right.shift`, so the game path uses this one instead.
 */
fun gameKeyNameForAndroidKeyCode(keyCode: Int): String? = when (keyCode) {
    AndroidKeyEvent.KEYCODE_SHIFT_RIGHT -> "right.shift"
    AndroidKeyEvent.KEYCODE_CTRL_RIGHT -> "right.control"
    AndroidKeyEvent.KEYCODE_ALT_RIGHT -> "right.alt"
    AndroidKeyEvent.KEYCODE_META_RIGHT -> "right.super"
    AndroidKeyEvent.KEYCODE_MENU -> "menu"
    AndroidKeyEvent.KEYCODE_NUM_LOCK -> "num.lock"
    AndroidKeyEvent.KEYCODE_SCROLL_LOCK -> "scroll.lock"
    AndroidKeyEvent.KEYCODE_SYSRQ -> "print.screen"
    AndroidKeyEvent.KEYCODE_BREAK -> "pause"
    else -> awtKeyNameForAndroidKeyCode(keyCode)
}

/**
 * evdev codes of `KEYCODE_A`..`KEYCODE_Z`, in Android's alphabetical order.
 *
 * The QWERTY layout is why this is a table and not arithmetic: `a` is 30 but `b`
 * is 48, because evdev numbers the physical rows.
 */
private val LETTER_SCANCODES = intArrayOf(
    30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50,
    49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44,
)

/** evdev codes of `KEYCODE_NUMPAD_0`..`KEYCODE_NUMPAD_9`. */
private val KEYPAD_SCANCODES = intArrayOf(82, 79, 80, 81, 75, 76, 77, 71, 72, 73)
