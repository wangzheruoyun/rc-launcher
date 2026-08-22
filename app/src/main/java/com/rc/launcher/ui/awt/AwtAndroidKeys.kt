package com.rc.launcher.ui.awt

import android.view.KeyEvent as AndroidKeyEvent

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
