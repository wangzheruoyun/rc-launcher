//! Physical keyboard & mouse for the running game (task 12).
//!
//! The AWT bridge ([`super::awt`], [`super::fakefx`]) makes Minecraft's *embedded
//! Swing UI* usable with a finger. This module is the other half of the input
//! story: a real USB / Bluetooth keyboard and mouse driving the **game itself**.
//!
//! ```text
//!  physical device            Compose (ui/awt/*.kt)             Rust core
//!  ---------------            ---------------------             ---------
//!  mouse moved 3 px  ──▶ MotionEvent(AXIS_RELATIVE_*)  ──▶  MouseMotion::step
//!  (pointer captured)                                          │  sensitivity
//!                                                              ▼
//!                                          AwtInputTranslator  ·  GameInputTranslator
//!                                          (MOUSE_MOVED,           (EVENT_TYPE_CURSOR_POS,
//!                                           Swing dialogs)          Minecraft itself)
//! ```
//!
//! Two consumers, one gesture, and that is the whole point: a launcher on
//! Android has to feed **both** event queues from the same physical device, or
//! the mouse works in the Forge installer but not in the game (or the other way
//! round).
//!
//! # The game-side contract
//!
//! FCL's LWJGL fork (shipped by task 1 as `lwjgl-*-merged-modules.jar`) contains
//! `org.lwjgl.glfw.CallbackBridge`, whose whole job is to accept input from the
//! Android side and hand it to the GLFW stub the game is linked against:
//!
//! | `CallbackBridge` | reaches the game as | parameters |
//! |---|---|---|
//! | `EVENT_TYPE_KEY` (1005) | `GLFWKeyCallback` | key, **scancode**, action, mods |
//! | `EVENT_TYPE_CHAR` (1000) | `GLFWCharCallback` | codepoint |
//! | `EVENT_TYPE_CHAR_MODS` (1001) | `GLFWCharModsCallback` | codepoint, mods |
//! | `EVENT_TYPE_CURSOR_POS` (1003) | `GLFWCursorPosCallback` | x, y |
//! | `EVENT_TYPE_CURSOR_ENTER` (1002) | `GLFWCursorEnterCallback` | entered |
//! | `EVENT_TYPE_MOUSE_BUTTON` (1006) | `GLFWMouseButtonCallback` | button, action, mods |
//! | `EVENT_TYPE_SCROLL` (1007) | `GLFWScrollCallback` | xoffset, yoffset |
//! | `EVENT_TYPE_FRAMEBUFFER_SIZE` (1004) | `GLFWFramebufferSizeCallback` | width, height |
//! | `EVENT_TYPE_WINDOW_SIZE` (1008) | `GLFWWindowSizeCallback` | width, height |
//! | `ANDROID_TYPE_GRAB_STATE` (0) | `Mouse.setGrabbed` / `onGrabStateChanged` | grabbing |
//!
//! Those numbers are not invented here: they are the `public static final int`s
//! of the `CallbackBridge` class inside the LWJGL jar this launcher ships, and
//! [`game_event`] mirrors them so a drift becomes a failing test instead of a
//! game that ignores the keyboard.
//!
//! Because the game JVM is a *child process* (not an in-process VM as in FCL),
//! the events travel over the existing event channel as ordinary 32-byte
//! [`AwtEventRecord`]s carrying the reserved id [`GAME_INPUT_EVENT_ID`] — exactly
//! how the control plane already reuses that stream. A JVM-side bridge switches
//! on the id: AWT ids go to `EventQueue.postEvent`, `GAME_INPUT_EVENT_ID` goes to
//! `CallbackBridge`.
//!
//! # Why a scancode matters
//!
//! `GLFWKeyCallback` takes a *key* **and** a *scancode*. Minecraft uses the key
//! for its bindings but falls back to the scancode for everything the key
//! enumeration does not cover (`InputConstants.getKey(key, scancode)`,
//! `glfwGetKeyName`), which is what makes non-US layouts and the extra keys of a
//! phone keyboard work. Android hands us the real thing: `KeyEvent.getScanCode()`
//! *is* the Linux evdev code, so the UI forwards it verbatim and
//! [`scancode_for_glfw_key`] only fills in the blanks (on-screen buttons, a
//! gamepad mapped to a key, an IME that reports 0).
//!
//! Everything in this module is integer-only and free of I/O, so the whole
//! translation is unit-testable and bit-for-bit reproducible on the host.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::awt::{vk_for_key, AwtEventRecord, MouseButton};
use crate::error::{RcError, RcResult};

/// Reserved [`AwtEventRecord::id`] for a *game* input record.
///
/// Sits next to `CONTROL_EVENT_ID` (`0x7263_0001`) in the private `0x7263_xxxx`
/// range, so it can never collide with a `java.awt.event` id (all of which are
/// small positive integers).
pub const GAME_INPUT_EVENT_ID: i32 = 0x7263_0002;

/// `org.lwjgl.glfw.CallbackBridge` event types (verbatim from the shipped jar).
pub mod game_event {
    /// `ANDROID_TYPE_GRAB_STATE` — the pointer was captured / released.
    pub const GRAB_STATE: i32 = 0;
    /// `EVENT_TYPE_CHAR` — a typed codepoint.
    pub const CHAR: i32 = 1000;
    /// `EVENT_TYPE_CHAR_MODS` — a typed codepoint plus modifiers.
    pub const CHAR_MODS: i32 = 1001;
    /// `EVENT_TYPE_CURSOR_ENTER` — the pointer entered / left the window.
    pub const CURSOR_ENTER: i32 = 1002;
    /// `EVENT_TYPE_CURSOR_POS` — absolute cursor position, in window pixels.
    pub const CURSOR_POS: i32 = 1003;
    /// `EVENT_TYPE_FRAMEBUFFER_SIZE` — the drawable was resized.
    pub const FRAMEBUFFER_SIZE: i32 = 1004;
    /// `EVENT_TYPE_KEY` — key, scancode, action, mods.
    pub const KEY: i32 = 1005;
    /// `EVENT_TYPE_MOUSE_BUTTON` — button, action, mods.
    pub const MOUSE_BUTTON: i32 = 1006;
    /// `EVENT_TYPE_SCROLL` — wheel offsets.
    pub const SCROLL: i32 = 1007;
    /// `EVENT_TYPE_WINDOW_SIZE` — the window was resized.
    pub const WINDOW_SIZE: i32 = 1008;

    /// Human-readable name, for diagnostics and error messages.
    pub fn name(kind: i32) -> &'static str {
        match kind {
            GRAB_STATE => "grab_state",
            CHAR => "char",
            CHAR_MODS => "char_mods",
            CURSOR_ENTER => "cursor_enter",
            CURSOR_POS => "cursor_pos",
            FRAMEBUFFER_SIZE => "framebuffer_size",
            KEY => "key",
            MOUSE_BUTTON => "mouse_button",
            SCROLL => "scroll",
            WINDOW_SIZE => "window_size",
            _ => "unknown",
        }
    }

    /// Whether [`name`] knows this kind (an unknown kind is never encoded).
    pub fn is_known(kind: i32) -> bool {
        name(kind) != "unknown"
    }
}

/// GLFW 3 constants the game side speaks (keys, actions, mods, buttons).
///
/// Only the values a phone can actually produce are listed; everything else in
/// the GLFW enumeration would be dead weight.
pub mod glfw {
    /// `GLFW_RELEASE`
    pub const RELEASE: i32 = 0;
    /// `GLFW_PRESS`
    pub const PRESS: i32 = 1;
    /// `GLFW_REPEAT`
    pub const REPEAT: i32 = 2;

    /// `GLFW_MOD_SHIFT`
    pub const MOD_SHIFT: i32 = 0x0001;
    /// `GLFW_MOD_CONTROL`
    pub const MOD_CONTROL: i32 = 0x0002;
    /// `GLFW_MOD_ALT`
    pub const MOD_ALT: i32 = 0x0004;
    /// `GLFW_MOD_SUPER`
    pub const MOD_SUPER: i32 = 0x0008;
    /// `GLFW_MOD_CAPS_LOCK`
    pub const MOD_CAPS_LOCK: i32 = 0x0010;
    /// `GLFW_MOD_NUM_LOCK`
    pub const MOD_NUM_LOCK: i32 = 0x0020;

    /// `GLFW_MOUSE_BUTTON_LEFT` (note: GLFW counts from 0, AWT from 1).
    pub const MOUSE_BUTTON_LEFT: i32 = 0;
    /// `GLFW_MOUSE_BUTTON_RIGHT`
    pub const MOUSE_BUTTON_RIGHT: i32 = 1;
    /// `GLFW_MOUSE_BUTTON_MIDDLE`
    pub const MOUSE_BUTTON_MIDDLE: i32 = 2;

    /// `GLFW_KEY_SPACE`
    pub const KEY_SPACE: i32 = 32;
    /// `GLFW_KEY_APOSTROPHE`
    pub const KEY_APOSTROPHE: i32 = 39;
    /// `GLFW_KEY_COMMA`
    pub const KEY_COMMA: i32 = 44;
    /// `GLFW_KEY_MINUS`
    pub const KEY_MINUS: i32 = 45;
    /// `GLFW_KEY_PERIOD`
    pub const KEY_PERIOD: i32 = 46;
    /// `GLFW_KEY_SLASH`
    pub const KEY_SLASH: i32 = 47;
    /// `GLFW_KEY_0` (digits are their ASCII codes, like AWT)
    pub const KEY_0: i32 = 48;
    /// `GLFW_KEY_SEMICOLON`
    pub const KEY_SEMICOLON: i32 = 59;
    /// `GLFW_KEY_EQUAL`
    pub const KEY_EQUAL: i32 = 61;
    /// `GLFW_KEY_A` (letters are their upper-case ASCII codes, like AWT)
    pub const KEY_A: i32 = 65;
    /// `GLFW_KEY_LEFT_BRACKET`
    pub const KEY_LEFT_BRACKET: i32 = 91;
    /// `GLFW_KEY_BACKSLASH`
    pub const KEY_BACKSLASH: i32 = 92;
    /// `GLFW_KEY_RIGHT_BRACKET`
    pub const KEY_RIGHT_BRACKET: i32 = 93;
    /// `GLFW_KEY_GRAVE_ACCENT`
    pub const KEY_GRAVE_ACCENT: i32 = 96;
    /// `GLFW_KEY_ESCAPE`
    pub const KEY_ESCAPE: i32 = 256;
    /// `GLFW_KEY_ENTER`
    pub const KEY_ENTER: i32 = 257;
    /// `GLFW_KEY_TAB`
    pub const KEY_TAB: i32 = 258;
    /// `GLFW_KEY_BACKSPACE`
    pub const KEY_BACKSPACE: i32 = 259;
    /// `GLFW_KEY_INSERT`
    pub const KEY_INSERT: i32 = 260;
    /// `GLFW_KEY_DELETE`
    pub const KEY_DELETE: i32 = 261;
    /// `GLFW_KEY_RIGHT`
    pub const KEY_RIGHT: i32 = 262;
    /// `GLFW_KEY_LEFT`
    pub const KEY_LEFT: i32 = 263;
    /// `GLFW_KEY_DOWN`
    pub const KEY_DOWN: i32 = 264;
    /// `GLFW_KEY_UP`
    pub const KEY_UP: i32 = 265;
    /// `GLFW_KEY_PAGE_UP`
    pub const KEY_PAGE_UP: i32 = 266;
    /// `GLFW_KEY_PAGE_DOWN`
    pub const KEY_PAGE_DOWN: i32 = 267;
    /// `GLFW_KEY_HOME`
    pub const KEY_HOME: i32 = 268;
    /// `GLFW_KEY_END`
    pub const KEY_END: i32 = 269;
    /// `GLFW_KEY_CAPS_LOCK`
    pub const KEY_CAPS_LOCK: i32 = 280;
    /// `GLFW_KEY_SCROLL_LOCK`
    pub const KEY_SCROLL_LOCK: i32 = 281;
    /// `GLFW_KEY_NUM_LOCK`
    pub const KEY_NUM_LOCK: i32 = 282;
    /// `GLFW_KEY_PRINT_SCREEN`
    pub const KEY_PRINT_SCREEN: i32 = 283;
    /// `GLFW_KEY_PAUSE`
    pub const KEY_PAUSE: i32 = 284;
    /// `GLFW_KEY_F1` (`F1..F10` are contiguous, so `Fn == KEY_F1 + n - 1`)
    pub const KEY_F1: i32 = 290;
    /// `GLFW_KEY_F11`
    pub const KEY_F11: i32 = 300;
    /// `GLFW_KEY_F12`
    pub const KEY_F12: i32 = 301;
    /// `GLFW_KEY_KP_0`
    pub const KEY_KP_0: i32 = 320;
    /// `GLFW_KEY_KP_DECIMAL`
    pub const KEY_KP_DECIMAL: i32 = 330;
    /// `GLFW_KEY_KP_DIVIDE`
    pub const KEY_KP_DIVIDE: i32 = 331;
    /// `GLFW_KEY_KP_MULTIPLY`
    pub const KEY_KP_MULTIPLY: i32 = 332;
    /// `GLFW_KEY_KP_SUBTRACT`
    pub const KEY_KP_SUBTRACT: i32 = 333;
    /// `GLFW_KEY_KP_ADD`
    pub const KEY_KP_ADD: i32 = 334;
    /// `GLFW_KEY_KP_ENTER`
    pub const KEY_KP_ENTER: i32 = 335;
    /// `GLFW_KEY_KP_EQUAL`
    pub const KEY_KP_EQUAL: i32 = 336;
    /// `GLFW_KEY_LEFT_SHIFT`
    pub const KEY_LEFT_SHIFT: i32 = 340;
    /// `GLFW_KEY_LEFT_CONTROL`
    pub const KEY_LEFT_CONTROL: i32 = 341;
    /// `GLFW_KEY_LEFT_ALT`
    pub const KEY_LEFT_ALT: i32 = 342;
    /// `GLFW_KEY_LEFT_SUPER`
    pub const KEY_LEFT_SUPER: i32 = 343;
    /// `GLFW_KEY_RIGHT_SHIFT`
    pub const KEY_RIGHT_SHIFT: i32 = 344;
    /// `GLFW_KEY_RIGHT_CONTROL`
    pub const KEY_RIGHT_CONTROL: i32 = 345;
    /// `GLFW_KEY_RIGHT_ALT`
    pub const KEY_RIGHT_ALT: i32 = 346;
    /// `GLFW_KEY_RIGHT_SUPER`
    pub const KEY_RIGHT_SUPER: i32 = 347;
    /// `GLFW_KEY_MENU`
    pub const KEY_MENU: i32 = 348;
    /// `GLFW_KEY_LAST`
    pub const KEY_LAST: i32 = KEY_MENU;
}

/// Normalise a key name the way [`vk_for_key`] does, so one vocabulary drives
/// AWT codes, GLFW codes, scancodes and the task-15 control layouts.
///
/// `"key.keyboard.LEFT_SHIFT"`, `"Left.Shift"` and `"left shift"` all become
/// `"left.shift"`.
pub fn normalise_key_name(name: &str) -> String {
    let lowered = name.trim().to_ascii_lowercase().replace(['_', ' '], ".");
    lowered
        .strip_prefix("key.keyboard.")
        .unwrap_or(&lowered)
        .to_string()
}

/// A mouse button named in the task-15 vocabulary (`key.mouse.left`).
///
/// Returns `None` for anything that is not a mouse binding, so a caller can try
/// [`glfw_key_for_key`] next.
pub fn mouse_button_for_name(name: &str) -> Option<MouseButton> {
    let n = name.trim().to_ascii_lowercase().replace(['_', ' '], ".");
    let n = n.strip_prefix("key.mouse.").unwrap_or(&n);
    match n {
        "mouse.left" | "left" | "primary" | "mouse1" => Some(MouseButton::Left),
        "mouse.right" | "right" | "secondary" | "mouse2" => Some(MouseButton::Right),
        "mouse.middle" | "middle" | "mouse3" => Some(MouseButton::Middle),
        _ => None,
    }
}

/// The `GLFW_MOUSE_BUTTON_*` number of an AWT button.
///
/// GLFW counts from zero *and* orders right before middle, so this is not an
/// off-by-one: `BUTTON2` (AWT middle) is `GLFW_MOUSE_BUTTON_MIDDLE` = 2, while
/// `BUTTON3` (AWT right) is `GLFW_MOUSE_BUTTON_RIGHT` = 1.
pub fn glfw_button(button: MouseButton) -> i32 {
    match button {
        MouseButton::Left => glfw::MOUSE_BUTTON_LEFT,
        MouseButton::Middle => glfw::MOUSE_BUTTON_MIDDLE,
        MouseButton::Right => glfw::MOUSE_BUTTON_RIGHT,
    }
}

/// Translate a key *name* into a `GLFW_KEY_*` code.
///
/// Accepts the same vocabulary as [`vk_for_key`] but keeps the distinction AWT
/// throws away: `left.shift` and `right.shift` are *different* GLFW keys, and
/// Minecraft's key bindings can tell them apart.
///
/// Returns `None` for a name GLFW has no key for, so the caller degrades to a
/// character event instead of pressing something wrong.
pub fn glfw_key_for_key(name: &str) -> Option<i32> {
    let n = normalise_key_name(name);
    if n.len() == 1 {
        let c = n.as_bytes()[0];
        if c.is_ascii_lowercase() {
            return Some(c.to_ascii_uppercase() as i32); // GLFW_KEY_A == 'A'
        }
        if c.is_ascii_digit() {
            return Some(c as i32); // GLFW_KEY_0 == '0'
        }
    }
    if let Some(rest) = n.strip_prefix('f') {
        if let Ok(i) = rest.parse::<i32>() {
            if (1..=25).contains(&i) {
                return Some(glfw::KEY_F1 + i - 1);
            }
        }
    }
    if let Some(rest) = n.strip_prefix("keypad.") {
        if let Ok(i) = rest.parse::<i32>() {
            if (0..=9).contains(&i) {
                return Some(glfw::KEY_KP_0 + i);
            }
        }
        return match rest {
            "enter" => Some(glfw::KEY_KP_ENTER),
            "add" | "plus" => Some(glfw::KEY_KP_ADD),
            "subtract" | "minus" => Some(glfw::KEY_KP_SUBTRACT),
            "multiply" => Some(glfw::KEY_KP_MULTIPLY),
            "divide" => Some(glfw::KEY_KP_DIVIDE),
            "decimal" | "dot" => Some(glfw::KEY_KP_DECIMAL),
            "equal" => Some(glfw::KEY_KP_EQUAL),
            _ => None,
        };
    }
    Some(match n.as_str() {
        "escape" | "esc" => glfw::KEY_ESCAPE,
        "space" => glfw::KEY_SPACE,
        "enter" | "return" => glfw::KEY_ENTER,
        "tab" => glfw::KEY_TAB,
        "backspace" | "back.space" => glfw::KEY_BACKSPACE,
        "delete" => glfw::KEY_DELETE,
        "insert" => glfw::KEY_INSERT,
        "home" => glfw::KEY_HOME,
        "end" => glfw::KEY_END,
        "page.up" | "pageup" | "prior" => glfw::KEY_PAGE_UP,
        "page.down" | "pagedown" | "next" => glfw::KEY_PAGE_DOWN,
        "left" | "arrow.left" | "left.arrow" => glfw::KEY_LEFT,
        "right" | "arrow.right" | "right.arrow" => glfw::KEY_RIGHT,
        "up" | "arrow.up" | "up.arrow" => glfw::KEY_UP,
        "down" | "arrow.down" | "down.arrow" => glfw::KEY_DOWN,
        "shift" | "left.shift" | "shift.left" => glfw::KEY_LEFT_SHIFT,
        "right.shift" | "shift.right" => glfw::KEY_RIGHT_SHIFT,
        "control" | "ctrl" | "left.control" | "control.left" | "left.ctrl" | "ctrl.left" => {
            glfw::KEY_LEFT_CONTROL
        }
        "right.control" | "control.right" | "right.ctrl" | "ctrl.right" => glfw::KEY_RIGHT_CONTROL,
        "alt" | "left.alt" | "alt.left" => glfw::KEY_LEFT_ALT,
        "right.alt" | "alt.right" | "alt.gr" | "altgr" => glfw::KEY_RIGHT_ALT,
        "super" | "meta" | "left.super" | "left.win" | "win" | "super.left" => glfw::KEY_LEFT_SUPER,
        "right.super" | "right.win" | "super.right" => glfw::KEY_RIGHT_SUPER,
        "menu" | "context.menu" => glfw::KEY_MENU,
        "caps.lock" | "capslock" => glfw::KEY_CAPS_LOCK,
        "num.lock" | "numlock" => glfw::KEY_NUM_LOCK,
        "scroll.lock" | "scrolllock" => glfw::KEY_SCROLL_LOCK,
        "print.screen" | "printscreen" | "sysrq" => glfw::KEY_PRINT_SCREEN,
        "pause" | "break" => glfw::KEY_PAUSE,
        "comma" => glfw::KEY_COMMA,
        "period" | "dot" => glfw::KEY_PERIOD,
        "slash" => glfw::KEY_SLASH,
        "backslash" | "back.slash" => glfw::KEY_BACKSLASH,
        "semicolon" => glfw::KEY_SEMICOLON,
        "equal" | "equals" => glfw::KEY_EQUAL,
        "minus" => glfw::KEY_MINUS,
        "left.bracket" | "open.bracket" | "bracket.left" => glfw::KEY_LEFT_BRACKET,
        "right.bracket" | "close.bracket" | "bracket.right" => glfw::KEY_RIGHT_BRACKET,
        "grave.accent" | "grave" | "back.quote" => glfw::KEY_GRAVE_ACCENT,
        "apostrophe" | "quote" => glfw::KEY_APOSTROPHE,
        _ => return None,
    })
}

/// The `GLFW_MOD_*` bit a modifier *key* contributes while it is held.
pub fn glfw_mod_for_key(key: i32) -> Option<i32> {
    match key {
        glfw::KEY_LEFT_SHIFT | glfw::KEY_RIGHT_SHIFT => Some(glfw::MOD_SHIFT),
        glfw::KEY_LEFT_CONTROL | glfw::KEY_RIGHT_CONTROL => Some(glfw::MOD_CONTROL),
        glfw::KEY_LEFT_ALT | glfw::KEY_RIGHT_ALT => Some(glfw::MOD_ALT),
        glfw::KEY_LEFT_SUPER | glfw::KEY_RIGHT_SUPER => Some(glfw::MOD_SUPER),
        glfw::KEY_CAPS_LOCK => Some(glfw::MOD_CAPS_LOCK),
        glfw::KEY_NUM_LOCK => Some(glfw::MOD_NUM_LOCK),
        _ => None,
    }
}

/// The Linux **evdev** scancode of a GLFW key (`0` when there is no sensible
/// one).
///
/// This is the number `KeyEvent.getScanCode()` reports on Android for the same
/// physical key, which is why the UI can forward the real one and only fall back
/// to this table for synthetic presses (on-screen buttons, a gamepad mapped to a
/// key, an IME that reports `0`).
pub fn scancode_for_glfw_key(key: i32) -> i32 {
    // Letters and digits are the two dense rows; spell them out via lookup
    // tables so the mapping stays obvious and total.
    const LETTERS: [i32; 26] = [
        30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17,
        45, 21, 44,
    ];
    const DIGITS: [i32; 10] = [11, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    if (glfw::KEY_A..=glfw::KEY_A + 25).contains(&key) {
        return LETTERS[(key - glfw::KEY_A) as usize];
    }
    if (glfw::KEY_0..=glfw::KEY_0 + 9).contains(&key) {
        return DIGITS[(key - glfw::KEY_0) as usize];
    }
    if (glfw::KEY_F1..=glfw::KEY_F1 + 9).contains(&key) {
        return 59 + (key - glfw::KEY_F1); // F1..F10 = 59..68
    }
    if (glfw::KEY_KP_0..=glfw::KEY_KP_0 + 9).contains(&key) {
        // The keypad is not contiguous in evdev: KP7 is 71, KP0 is 82.
        const KP: [i32; 10] = [82, 79, 80, 81, 75, 76, 77, 71, 72, 73];
        return KP[(key - glfw::KEY_KP_0) as usize];
    }
    match key {
        glfw::KEY_ESCAPE => 1,
        glfw::KEY_MINUS => 12,
        glfw::KEY_EQUAL => 13,
        glfw::KEY_BACKSPACE => 14,
        glfw::KEY_TAB => 15,
        glfw::KEY_LEFT_BRACKET => 26,
        glfw::KEY_RIGHT_BRACKET => 27,
        glfw::KEY_ENTER => 28,
        glfw::KEY_LEFT_CONTROL => 29,
        glfw::KEY_SEMICOLON => 39,
        glfw::KEY_APOSTROPHE => 40,
        glfw::KEY_GRAVE_ACCENT => 41,
        glfw::KEY_LEFT_SHIFT => 42,
        glfw::KEY_BACKSLASH => 43,
        glfw::KEY_COMMA => 51,
        glfw::KEY_PERIOD => 52,
        glfw::KEY_SLASH => 53,
        glfw::KEY_RIGHT_SHIFT => 54,
        glfw::KEY_KP_MULTIPLY => 55,
        glfw::KEY_LEFT_ALT => 56,
        glfw::KEY_SPACE => 57,
        glfw::KEY_CAPS_LOCK => 58,
        glfw::KEY_NUM_LOCK => 69,
        glfw::KEY_SCROLL_LOCK => 70,
        glfw::KEY_KP_SUBTRACT => 74,
        glfw::KEY_KP_ADD => 78,
        glfw::KEY_KP_DECIMAL => 83,
        glfw::KEY_F11 => 87,
        glfw::KEY_F12 => 88,
        glfw::KEY_KP_ENTER => 96,
        glfw::KEY_RIGHT_CONTROL => 97,
        glfw::KEY_KP_DIVIDE => 98,
        glfw::KEY_PRINT_SCREEN => 99,
        glfw::KEY_RIGHT_ALT => 100,
        glfw::KEY_HOME => 102,
        glfw::KEY_UP => 103,
        glfw::KEY_PAGE_UP => 104,
        glfw::KEY_LEFT => 105,
        glfw::KEY_RIGHT => 106,
        glfw::KEY_END => 107,
        glfw::KEY_DOWN => 108,
        glfw::KEY_PAGE_DOWN => 109,
        glfw::KEY_INSERT => 110,
        glfw::KEY_DELETE => 111,
        glfw::KEY_PAUSE => 119,
        glfw::KEY_LEFT_SUPER => 125,
        glfw::KEY_RIGHT_SUPER => 126,
        glfw::KEY_MENU => 127,
        _ => 0,
    }
}

/// The evdev scancode of a *named* key (`0` when unknown).
pub fn scancode_for_key(name: &str) -> i32 {
    glfw_key_for_key(name)
        .map(scancode_for_glfw_key)
        .unwrap_or(0)
}

/// The `GLFW_KEY_*` code of an AWT `VK_*` code (`None` when there is none).
///
/// AWT collapses the left and right modifier keys onto one code, so a `VK_SHIFT`
/// necessarily becomes `GLFW_KEY_LEFT_SHIFT` — the side is information the AWT
/// event never carried. Everything the UI knows the *name* of should go through
/// [`glfw_key_for_key`] instead, which keeps the side.
pub fn glfw_key_for_vk(vk: i32) -> Option<i32> {
    // Letters and digits share their ASCII value between AWT and GLFW.
    if (b'A' as i32..=b'Z' as i32).contains(&vk) || (b'0' as i32..=b'9' as i32).contains(&vk) {
        return Some(vk);
    }
    // VK_F1 == 112, GLFW_KEY_F1 == 290 (both contiguous up to F12/F25).
    if (112..=123).contains(&vk) {
        return Some(glfw::KEY_F1 + vk - 112);
    }
    // VK_NUMPAD0 == 96, GLFW_KEY_KP_0 == 320.
    if (96..=105).contains(&vk) {
        return Some(glfw::KEY_KP_0 + vk - 96);
    }
    Some(match vk {
        27 => glfw::KEY_ESCAPE,        // VK_ESCAPE
        32 => glfw::KEY_SPACE,         // VK_SPACE
        10 => glfw::KEY_ENTER,         // VK_ENTER
        9 => glfw::KEY_TAB,            // VK_TAB
        8 => glfw::KEY_BACKSPACE,      // VK_BACK_SPACE
        127 => glfw::KEY_DELETE,       // VK_DELETE
        155 => glfw::KEY_INSERT,       // VK_INSERT
        36 => glfw::KEY_HOME,          // VK_HOME
        35 => glfw::KEY_END,           // VK_END
        33 => glfw::KEY_PAGE_UP,       // VK_PAGE_UP
        34 => glfw::KEY_PAGE_DOWN,     // VK_PAGE_DOWN
        37 => glfw::KEY_LEFT,          // VK_LEFT
        39 => glfw::KEY_RIGHT,         // VK_RIGHT
        38 => glfw::KEY_UP,            // VK_UP
        40 => glfw::KEY_DOWN,          // VK_DOWN
        16 => glfw::KEY_LEFT_SHIFT,    // VK_SHIFT
        17 => glfw::KEY_LEFT_CONTROL,  // VK_CONTROL
        18 => glfw::KEY_LEFT_ALT,      // VK_ALT
        157 => glfw::KEY_LEFT_SUPER,   // VK_META
        20 => glfw::KEY_CAPS_LOCK,     // VK_CAPS_LOCK
        44 => glfw::KEY_COMMA,         // VK_COMMA
        45 => glfw::KEY_MINUS,         // VK_MINUS
        46 => glfw::KEY_PERIOD,        // VK_PERIOD
        47 => glfw::KEY_SLASH,         // VK_SLASH
        59 => glfw::KEY_SEMICOLON,     // VK_SEMICOLON
        61 => glfw::KEY_EQUAL,         // VK_EQUALS
        91 => glfw::KEY_LEFT_BRACKET,  // VK_OPEN_BRACKET
        92 => glfw::KEY_BACKSLASH,     // VK_BACK_SLASH
        93 => glfw::KEY_RIGHT_BRACKET, // VK_CLOSE_BRACKET
        192 => glfw::KEY_GRAVE_ACCENT, // VK_BACK_QUOTE
        222 => glfw::KEY_APOSTROPHE,   // VK_QUOTE
        106 => glfw::KEY_KP_MULTIPLY,  // VK_MULTIPLY
        107 => glfw::KEY_KP_ADD,       // VK_ADD
        109 => glfw::KEY_KP_SUBTRACT,  // VK_SUBTRACT
        110 => glfw::KEY_KP_DECIMAL,   // VK_DECIMAL
        111 => glfw::KEY_KP_DIVIDE,    // VK_DIVIDE
        _ => return None,
    })
}

/// The evdev scancode of an AWT `VK_*` code (`0` when unknown).
pub fn scancode_for_vk(vk: i32) -> i32 {
    glfw_key_for_vk(vk).map(scancode_for_glfw_key).unwrap_or(0)
}

// ===========================================================================
// Settings: what the user can tune
// ===========================================================================

/// How a physical pointer drives the game.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerMode {
    /// The cursor is visible and follows an absolute position — menus, the
    /// inventory, a Forge dialog. A touch and a mouse behave the same way.
    #[default]
    Absolute,
    /// The pointer is *captured*: Android delivers relative motion only and the
    /// game owns the cursor (`GLFW_CURSOR_DISABLED`). This is the mode that makes
    /// looking around with a mouse feel like the desktop game.
    Captured,
}

impl PointerMode {
    /// Stable id used on the wire / in JSON.
    pub fn id(self) -> &'static str {
        match self {
            PointerMode::Absolute => "absolute",
            PointerMode::Captured => "captured",
        }
    }

    /// Parse an id; anything unknown is `None` (callers keep the old mode).
    pub fn from_id(id: &str) -> Option<PointerMode> {
        match id.trim().to_ascii_lowercase().as_str() {
            "absolute" | "pointer" | "released" => Some(PointerMode::Absolute),
            "captured" | "capture" | "grabbed" | "relative" => Some(PointerMode::Captured),
            _ => None,
        }
    }

    /// `true` while the pointer is captured.
    pub fn is_captured(self) -> bool {
        matches!(self, PointerMode::Captured)
    }
}

/// Which device produced a pointer sample.
///
/// Needed for the *hybrid* mode: with a mouse plugged in, a stray palm touch on
/// the screen must not yank the crosshair away, while a deliberate tap still has
/// to work when the mouse is idle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerSource {
    /// A finger.
    #[default]
    Touch,
    /// A physical mouse / trackpad.
    Mouse,
    /// A stylus (behaves like a mouse: it has a hover position).
    Stylus,
}

impl PointerSource {
    /// Stable id used on the wire / in JSON.
    pub fn id(self) -> &'static str {
        match self {
            PointerSource::Touch => "touch",
            PointerSource::Mouse => "mouse",
            PointerSource::Stylus => "stylus",
        }
    }

    /// Parse an id; anything unknown degrades to [`PointerSource::Touch`], the
    /// conservative choice (a touch is always allowed).
    pub fn from_id(id: &str) -> PointerSource {
        match id.trim().to_ascii_lowercase().as_str() {
            "mouse" | "pointer" | "trackpad" | "touchpad" => PointerSource::Mouse,
            "stylus" | "pen" | "eraser" => PointerSource::Stylus,
            _ => PointerSource::Touch,
        }
    }

    /// Whether this device carries a *hover* position (so relative motion and a
    /// captured pointer make sense for it).
    pub fn is_mouse_like(self) -> bool {
        matches!(self, PointerSource::Mouse | PointerSource::Stylus)
    }
}

/// Pointer sensitivity, in **per-mille** so the whole pipeline stays integer.
///
/// `1000` = 1:1 (one screen pixel per reported pixel), `500` = half speed,
/// `2500` = 2.5×. Floats are accepted at the edge ([`MouseSensitivity::uniform`])
/// and immediately quantised, which is what keeps the translation reproducible
/// and the state `Eq` — a test can compare two translators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseSensitivity {
    /// Horizontal factor, per-mille.
    pub x_permille: u32,
    /// Vertical factor, per-mille.
    pub y_permille: u32,
    /// Invert the vertical axis ("flight-stick" look).
    pub invert_y: bool,
}

impl MouseSensitivity {
    /// Slowest allowed factor (0.1×) — slower than this feels broken.
    pub const MIN_PERMILLE: u32 = 100;
    /// Fastest allowed factor (10×) — faster than this is unusable.
    pub const MAX_PERMILLE: u32 = 10_000;
    /// 1:1.
    pub const DEFAULT_PERMILLE: u32 = 1_000;

    /// The same factor on both axes.
    pub fn uniform(factor: f32) -> Self {
        Self::from_factors(factor, factor)
    }

    /// Per-axis factors, clamped into [`MouseSensitivity::MIN_PERMILLE`]
    /// ..=[`MouseSensitivity::MAX_PERMILLE`]. A `NaN` becomes 1:1.
    pub fn from_factors(x: f32, y: f32) -> Self {
        Self {
            x_permille: Self::quantise(x),
            y_permille: Self::quantise(y),
            invert_y: false,
        }
    }

    /// The same value with [`MouseSensitivity::invert_y`] set.
    pub fn with_invert_y(mut self, invert: bool) -> Self {
        self.invert_y = invert;
        self
    }

    /// Horizontal factor as a float (for the UI).
    pub fn x(&self) -> f32 {
        self.x_permille as f32 / 1000.0
    }

    /// Vertical factor as a float (for the UI).
    pub fn y(&self) -> f32 {
        self.y_permille as f32 / 1000.0
    }

    /// Clamp both axes into the legal range (a hostile / stale config file).
    pub fn sanitized(mut self) -> Self {
        self.x_permille = self
            .x_permille
            .clamp(Self::MIN_PERMILLE, Self::MAX_PERMILLE);
        self.y_permille = self
            .y_permille
            .clamp(Self::MIN_PERMILLE, Self::MAX_PERMILLE);
        self
    }

    /// JSON form (`{"x":1.5,"y":1.5,"invert_y":false}`), for the UI.
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "x": self.x(),
            "y": self.y(),
            "x_permille": self.x_permille,
            "y_permille": self.y_permille,
            "invert_y": self.invert_y,
        })
    }

    /// Parse the JSON form, accepting either the float or the per-mille spelling.
    /// Missing keys keep `self`'s value, so a partial update is a partial update.
    pub fn merged_from_json(mut self, value: &serde_json::Value) -> Self {
        if let Some(v) = value.get("x_permille").and_then(|v| v.as_u64()) {
            self.x_permille = v.min(u32::MAX as u64) as u32;
        } else if let Some(v) = value.get("x").and_then(|v| v.as_f64()) {
            self.x_permille = Self::quantise(v as f32);
        }
        if let Some(v) = value.get("y_permille").and_then(|v| v.as_u64()) {
            self.y_permille = v.min(u32::MAX as u64) as u32;
        } else if let Some(v) = value.get("y").and_then(|v| v.as_f64()) {
            self.y_permille = Self::quantise(v as f32);
        }
        if let Some(v) = value.get("invert_y").and_then(|v| v.as_bool()) {
            self.invert_y = v;
        }
        self.sanitized()
    }

    fn quantise(factor: f32) -> u32 {
        if !factor.is_finite() {
            return Self::DEFAULT_PERMILLE;
        }
        let permille = (factor * 1000.0).round();
        if permille <= Self::MIN_PERMILLE as f32 {
            Self::MIN_PERMILLE
        } else if permille >= Self::MAX_PERMILLE as f32 {
            Self::MAX_PERMILLE
        } else {
            permille as u32
        }
    }
}

impl Default for MouseSensitivity {
    fn default() -> Self {
        Self {
            x_permille: Self::DEFAULT_PERMILLE,
            y_permille: Self::DEFAULT_PERMILLE,
            invert_y: false,
        }
    }
}

/// Sub-pixel accumulator for relative pointer motion.
///
/// A 0.4× sensitivity would otherwise throw away every single-pixel sample and
/// the pointer would simply not move; keeping the remainder in **milli-pixels**
/// makes slow movement smooth *and* exactly reproducible (no float state).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MouseMotion {
    residue: (i64, i64),
    samples: u64,
    coalesced: u64,
}

impl MouseMotion {
    /// The largest single sample we believe (a bad driver can report absurd
    /// values, and one bogus sample must not teleport the crosshair).
    pub const MAX_DELTA_PX: f32 = 4096.0;

    /// Scale one relative sample and return the whole-pixel step to apply.
    ///
    /// `(0, 0)` means "the movement was sub-pixel, it is remembered" — the caller
    /// then emits **no** event, which is exactly what keeps a slow mouse from
    /// flooding the event queue with no-op moves.
    pub fn step(&mut self, sensitivity: &MouseSensitivity, dx: f32, dy: f32) -> (i32, i32) {
        self.samples += 1;
        let dx = Self::sane(dx);
        let dy = Self::sane(dy);
        let mut milli_x = (dx * sensitivity.x_permille as f32).round() as i64;
        let mut milli_y = (dy * sensitivity.y_permille as f32).round() as i64;
        if sensitivity.invert_y {
            milli_y = -milli_y;
        }
        // Saturating: MAX_DELTA_PX * MAX_PERMILLE is far from i64 range, but the
        // accumulated residue must not be able to wrap even in a pathological
        // stream of samples the caller never converts into a step.
        milli_x = milli_x.saturating_add(self.residue.0);
        milli_y = milli_y.saturating_add(self.residue.1);
        let step_x = milli_x / 1000;
        let step_y = milli_y / 1000;
        self.residue = (milli_x - step_x * 1000, milli_y - step_y * 1000);
        if step_x == 0 && step_y == 0 {
            self.coalesced += 1;
        }
        (
            step_x.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            step_y.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        )
    }

    /// Drop the remembered sub-pixel remainder (focus loss, mode change).
    pub fn reset(&mut self) {
        self.residue = (0, 0);
    }

    /// Samples seen so far.
    pub fn samples(&self) -> u64 {
        self.samples
    }

    /// Samples that were entirely sub-pixel (and therefore produced no event).
    pub fn coalesced(&self) -> u64 {
        self.coalesced
    }

    fn sane(delta: f32) -> f32 {
        if !delta.is_finite() {
            0.0
        } else {
            delta.clamp(-Self::MAX_DELTA_PX, Self::MAX_DELTA_PX)
        }
    }
}

/// User key / button remapping.
///
/// **Single hop by design.** `a → b` and `b → c` never turn `a` into `c`, so a
/// cycle cannot exist and a lookup is always O(log n) with no visit set. Users
/// think of a remap as "this key now does that", not as a rewriting system.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputBindings {
    keys: BTreeMap<String, String>,
    buttons: BTreeMap<i32, i32>,
}

impl InputBindings {
    /// Hard cap on remaps, so a corrupt config cannot grow the map without end.
    pub const MAX_ENTRIES: usize = 256;

    /// An empty table (everything is itself).
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` when nothing is remapped.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.buttons.is_empty()
    }

    /// Number of key remaps.
    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    /// Number of button remaps.
    pub fn button_count(&self) -> usize {
        self.buttons.len()
    }

    /// Remap `from` to `to`, both in the task-15 key vocabulary.
    ///
    /// Both names must resolve to a real key (GLFW *or* AWT — a key AWT does not
    /// know can still be sent to the game), otherwise the binding is rejected
    /// instead of silently swallowing the key.
    pub fn bind_key(&mut self, from: &str, to: &str) -> RcResult<()> {
        let from = normalise_key_name(from);
        let to = normalise_key_name(to);
        if from.is_empty() || to.is_empty() {
            return Err(RcError::Launch("a key binding needs both names".into()));
        }
        for name in [&from, &to] {
            if glfw_key_for_key(name).is_none() && vk_for_key(name).is_none() {
                return Err(RcError::Launch(format!("unknown key name {name:?}")));
            }
        }
        if self.keys.len() >= Self::MAX_ENTRIES && !self.keys.contains_key(&from) {
            return Err(RcError::Launch(format!(
                "too many key bindings (max {})",
                Self::MAX_ENTRIES
            )));
        }
        if from == to {
            self.keys.remove(&from);
        } else {
            self.keys.insert(from, to);
        }
        Ok(())
    }

    /// Drop one key remap; `true` when there was one.
    pub fn unbind_key(&mut self, from: &str) -> bool {
        self.keys.remove(&normalise_key_name(from)).is_some()
    }

    /// Remap a mouse button (left-handed users, or "middle acts as right").
    pub fn bind_button(&mut self, from: MouseButton, to: MouseButton) {
        if from == to {
            self.buttons.remove(&from.number());
        } else {
            self.buttons.insert(from.number(), to.number());
        }
    }

    /// The name a key is bound to (itself when unbound).
    pub fn resolve_key(&self, name: &str) -> String {
        let normalised = normalise_key_name(name);
        match self.keys.get(&normalised) {
            Some(target) => target.clone(),
            None => normalised,
        }
    }

    /// The button a button is bound to (itself when unbound).
    pub fn resolve_button(&self, button: MouseButton) -> MouseButton {
        self.buttons
            .get(&button.number())
            .and_then(|n| MouseButton::from_number(*n))
            .unwrap_or(button)
    }

    /// Forget every remap.
    pub fn clear(&mut self) {
        self.keys.clear();
        self.buttons.clear();
    }

    /// JSON form: `{"keys":{"e":"f"},"buttons":{"1":3}}`.
    pub fn to_json(&self) -> serde_json::Value {
        let keys: serde_json::Map<String, serde_json::Value> = self
            .keys
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        let buttons: serde_json::Map<String, serde_json::Value> = self
            .buttons
            .iter()
            .map(|(k, v)| (k.to_string(), json!(v)))
            .collect();
        json!({ "keys": keys, "buttons": buttons })
    }

    /// Parse the JSON form. An entry that names an unknown key is *skipped* (and
    /// counted), because one stale binding must not cost the user the rest of the
    /// table; the returned vector carries a message per skipped entry.
    pub fn from_json(value: &serde_json::Value) -> (Self, Vec<String>) {
        let mut out = Self::new();
        let mut skipped = Vec::new();
        if let Some(map) = value.get("keys").and_then(|v| v.as_object()) {
            for (from, to) in map {
                let Some(to) = to.as_str() else {
                    skipped.push(format!("key binding {from:?} has no target"));
                    continue;
                };
                if let Err(e) = out.bind_key(from, to) {
                    skipped.push(e.to_string());
                }
            }
        }
        if let Some(map) = value.get("buttons").and_then(|v| v.as_object()) {
            for (from, to) in map {
                let parsed = from
                    .parse::<i32>()
                    .ok()
                    .and_then(MouseButton::from_number)
                    .zip(to.as_i64().and_then(|n| MouseButton::from_number(n as i32)));
                match parsed {
                    Some((from, to)) => out.bind_button(from, to),
                    None => skipped.push(format!("bad button binding {from:?} -> {to}")),
                }
            }
        }
        (out, skipped)
    }
}

/// Everything the user can tune about physical input, in one place.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputSettings {
    /// Absolute cursor or captured pointer.
    pub pointer_mode: PointerMode,
    /// Accept touch samples while the pointer is captured (touch + mouse mixed).
    pub hybrid_touch: bool,
    /// Also emit the native (`CallbackBridge`) records, not only AWT ones.
    pub native_input: bool,
    /// Pointer sensitivity.
    pub sensitivity: MouseSensitivity,
    /// Wheel scaling, per-mille (`1000` = one notch per reported notch).
    pub scroll_permille: u32,
    /// Key / button remapping.
    pub bindings: InputBindings,
}

impl InputSettings {
    /// Slowest / fastest wheel scaling.
    pub const MIN_SCROLL_PERMILLE: u32 = 100;
    /// Fastest wheel scaling (10 notches per notch).
    pub const MAX_SCROLL_PERMILLE: u32 = 10_000;

    /// Defaults: absolute pointer, hybrid touch on, native records on, 1:1.
    pub fn new() -> Self {
        Self {
            pointer_mode: PointerMode::Absolute,
            hybrid_touch: true,
            native_input: true,
            sensitivity: MouseSensitivity::default(),
            scroll_permille: 1_000,
            bindings: InputBindings::new(),
        }
    }

    /// Clamp everything into its legal range.
    pub fn sanitized(mut self) -> Self {
        self.sensitivity = self.sensitivity.sanitized();
        self.scroll_permille = self
            .scroll_permille
            .clamp(Self::MIN_SCROLL_PERMILLE, Self::MAX_SCROLL_PERMILLE);
        self
    }

    /// Whether a sample from `source` may move the pointer right now.
    ///
    /// A captured pointer belongs to the mouse; a finger only gets through when
    /// the user asked for the mixed mode (`hybrid_touch`).
    pub fn accepts(&self, source: PointerSource) -> bool {
        if source.is_mouse_like() {
            return true;
        }
        !self.pointer_mode.is_captured() || self.hybrid_touch
    }

    /// Scale a wheel delta (in notches) by [`InputSettings::scroll_permille`].
    pub fn scale_scroll(&self, ticks: i32) -> i32 {
        let scaled = ticks as i64 * self.scroll_permille as i64 / 1000;
        // Never round a real notch down to "no scroll at all".
        let scaled = if scaled == 0 && ticks != 0 {
            ticks.signum() as i64
        } else {
            scaled
        };
        scaled.clamp(i32::MIN as i64, i32::MAX as i64) as i32
    }

    /// JSON snapshot for the UI / diagnostics.
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "pointer_mode": self.pointer_mode.id(),
            "captured": self.pointer_mode.is_captured(),
            "hybrid_touch": self.hybrid_touch,
            "native_input": self.native_input,
            "sensitivity": self.sensitivity.to_json(),
            "scroll_permille": self.scroll_permille,
            "bindings": self.bindings.to_json(),
        })
    }

    /// Apply a *partial* JSON update (the `awtConfigure` payload).
    ///
    /// Unknown / malformed members are reported instead of failing the whole
    /// update: the mode still changes when one binding is stale.
    pub fn apply_json(&mut self, value: &serde_json::Value) -> Vec<String> {
        let mut notes = Vec::new();
        if let Some(mode) = value.get("pointer_mode").and_then(|v| v.as_str()) {
            match PointerMode::from_id(mode) {
                Some(m) => self.pointer_mode = m,
                None => notes.push(format!("unknown pointer mode {mode:?}")),
            }
        }
        if let Some(captured) = value.get("captured").and_then(|v| v.as_bool()) {
            self.pointer_mode = if captured {
                PointerMode::Captured
            } else {
                PointerMode::Absolute
            };
        }
        if let Some(v) = value.get("hybrid_touch").and_then(|v| v.as_bool()) {
            self.hybrid_touch = v;
        }
        if let Some(v) = value.get("native_input").and_then(|v| v.as_bool()) {
            self.native_input = v;
        }
        if let Some(v) = value.get("sensitivity") {
            self.sensitivity = self.sensitivity.merged_from_json(v);
        }
        if let Some(v) = value.get("scroll_permille").and_then(|v| v.as_u64()) {
            self.scroll_permille = v.min(u32::MAX as u64) as u32;
        }
        if let Some(v) = value.get("bindings") {
            let (bindings, skipped) = InputBindings::from_json(v);
            self.bindings = bindings;
            notes.extend(skipped);
        }
        *self = self.clone().sanitized();
        notes
    }
}

// ===========================================================================
// Native game input: one `CallbackBridge` call per record
// ===========================================================================

/// One `CallbackBridge` event, in the wire form the event channel carries.
///
/// The parameters are positional because that is what the GLFW callbacks are:
///
/// | kind | `p0` | `p1` | `p2` | `p3` | `ch` |
/// |---|---|---|---|---|---|
/// | [`game_event::KEY`] | GLFW key | scancode | action | mods | – |
/// | [`game_event::CHAR`] / [`game_event::CHAR_MODS`] | – | – | – | mods | codepoint |
/// | [`game_event::CURSOR_POS`] | x | y | – | – | – |
/// | [`game_event::CURSOR_ENTER`] | entered (0/1) | – | – | – | – |
/// | [`game_event::MOUSE_BUTTON`] | GLFW button | action | – | mods | – |
/// | [`game_event::SCROLL`] | dx, milli-notches | dy, milli-notches | – | – | – |
/// | [`game_event::FRAMEBUFFER_SIZE`] / [`game_event::WINDOW_SIZE`] | width | height | – | – | – |
/// | [`game_event::GRAB_STATE`] | grabbing (0/1) | – | – | – | – |
///
/// Wheel offsets are carried in **milli-notches** because the record is integral
/// while `GLFWScrollCallback` takes doubles: a high-resolution trackpad still
/// scrolls smoothly instead of being rounded down to nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameInputEvent {
    /// One of [`game_event`].
    pub kind: i32,
    /// First parameter (see the table).
    pub p0: i32,
    /// Second parameter.
    pub p1: i32,
    /// Third parameter.
    pub p2: i32,
    /// Fourth parameter (the GLFW modifier bits, where there are any).
    pub p3: i32,
    /// Unicode codepoint, for the character events.
    pub ch: u32,
}

impl GameInputEvent {
    /// A key press / release (`GLFWKeyCallback`).
    pub fn key(key: i32, scancode: i32, action: i32, mods: i32) -> Self {
        Self {
            kind: game_event::KEY,
            p0: key,
            p1: scancode,
            p2: action,
            p3: mods,
            ch: 0,
        }
    }

    /// A typed codepoint (`GLFWCharCallback`).
    pub fn character(ch: char, mods: i32) -> Self {
        Self {
            kind: game_event::CHAR,
            p0: 0,
            p1: 0,
            p2: 0,
            p3: mods,
            ch: ch as u32,
        }
    }

    /// An absolute cursor position in window pixels (`GLFWCursorPosCallback`).
    pub fn cursor(x: i32, y: i32) -> Self {
        Self {
            kind: game_event::CURSOR_POS,
            p0: x,
            p1: y,
            p2: 0,
            p3: 0,
            ch: 0,
        }
    }

    /// The pointer entered / left the window (`GLFWCursorEnterCallback`).
    pub fn cursor_enter(entered: bool) -> Self {
        Self {
            kind: game_event::CURSOR_ENTER,
            p0: i32::from(entered),
            p1: 0,
            p2: 0,
            p3: 0,
            ch: 0,
        }
    }

    /// A mouse button press / release (`GLFWMouseButtonCallback`).
    pub fn mouse_button(button: i32, action: i32, mods: i32) -> Self {
        Self {
            kind: game_event::MOUSE_BUTTON,
            p0: button,
            p1: action,
            p2: 0,
            p3: mods,
            ch: 0,
        }
    }

    /// A wheel / trackpad scroll in milli-notches (`GLFWScrollCallback`).
    pub fn scroll(dx_milli: i32, dy_milli: i32) -> Self {
        Self {
            kind: game_event::SCROLL,
            p0: dx_milli,
            p1: dy_milli,
            p2: 0,
            p3: 0,
            ch: 0,
        }
    }

    /// The drawable was resized (`GLFWFramebufferSizeCallback`).
    pub fn framebuffer_size(width: i32, height: i32) -> Self {
        Self {
            kind: game_event::FRAMEBUFFER_SIZE,
            p0: width,
            p1: height,
            p2: 0,
            p3: 0,
            ch: 0,
        }
    }

    /// The window was resized (`GLFWWindowSizeCallback`).
    pub fn window_size(width: i32, height: i32) -> Self {
        Self {
            kind: game_event::WINDOW_SIZE,
            p0: width,
            p1: height,
            p2: 0,
            p3: 0,
            ch: 0,
        }
    }

    /// The pointer was captured / released (`ANDROID_TYPE_GRAB_STATE`).
    pub fn grab(grabbing: bool) -> Self {
        Self {
            kind: game_event::GRAB_STATE,
            p0: i32::from(grabbing),
            p1: 0,
            p2: 0,
            p3: 0,
            ch: 0,
        }
    }

    /// Whether this is a *motion* event, i.e. one that load shedding may drop
    /// because a newer one supersedes it (a lost key press is a bug; a lost
    /// intermediate cursor position is invisible).
    pub fn is_motion(&self) -> bool {
        matches!(self.kind, game_event::CURSOR_POS | game_event::SCROLL)
    }

    /// Encode into the 32-byte record the event channel carries.
    pub fn to_record(&self) -> AwtEventRecord {
        AwtEventRecord {
            id: GAME_INPUT_EVENT_ID,
            x: self.kind,
            y: self.p0,
            button: self.p1,
            key_code: self.p2,
            key_char: self.ch,
            modifiers: self.p3,
            wheel: 0,
        }
    }

    /// Decode a record, or `None` when it is not a (known) game input record.
    pub fn from_record(record: &AwtEventRecord) -> Option<Self> {
        if record.id != GAME_INPUT_EVENT_ID || !game_event::is_known(record.x) {
            return None;
        }
        Some(Self {
            kind: record.x,
            p0: record.y,
            p1: record.button,
            p2: record.key_code,
            p3: record.modifiers,
            ch: record.key_char,
        })
    }

    /// The parameter list as the JVM-side bridge passes it on, for logs and for
    /// the cross-language contract test: `"1005,87,17,1,0"` is
    /// `CallbackBridge.sendData(EVENT_TYPE_KEY, "87,17,1,0")`, i.e.
    /// `nativeSendKey(GLFW_KEY_W, 17, GLFW_PRESS, 0)`.
    pub fn payload(&self) -> String {
        match self.kind {
            game_event::KEY => format!("{},{},{},{}", self.p0, self.p1, self.p2, self.p3),
            game_event::CHAR => format!("{}", self.ch),
            game_event::CHAR_MODS => format!("{},{}", self.ch, self.p3),
            game_event::CURSOR_POS
            | game_event::SCROLL
            | game_event::FRAMEBUFFER_SIZE
            | game_event::WINDOW_SIZE => format!("{},{}", self.p0, self.p1),
            game_event::MOUSE_BUTTON => format!("{},{},{}", self.p0, self.p1, self.p3),
            _ => format!("{}", self.p0),
        }
    }

    /// One-line description for the diagnostics panel.
    pub fn describe(&self) -> String {
        format!("{}({})", game_event::name(self.kind), self.payload())
    }
}

/// Counters of the native input path (diagnostics; never resets by itself).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameInputStats {
    /// `CURSOR_POS` events emitted.
    pub cursor_events: u64,
    /// `KEY` events emitted.
    pub key_events: u64,
    /// `MOUSE_BUTTON` events emitted.
    pub button_events: u64,
    /// `SCROLL` events emitted.
    pub scroll_events: u64,
    /// `CHAR` events emitted.
    pub char_events: u64,
    /// Grab-state changes.
    pub grabs: u64,
    /// Times the free-running captured cursor was rebased (see
    /// [`GameInputTranslator::CURSOR_REBASE_PX`]).
    pub rebases: u64,
    /// Samples refused (unknown key name, closed window, filtered source).
    pub rejected: u64,
}

/// Turns physical input into `CallbackBridge` events for the running game.
///
/// It owns exactly the state GLFW would: the cursor position, which keys and
/// buttons are held, the modifier bits, and whether the pointer is grabbed.
///
/// Two behaviours are worth spelling out:
///
/// * **A held key repeats.** Pressing an already-held key emits `GLFW_REPEAT`,
///   not a second `GLFW_PRESS` — Minecraft's chat/`GuiTextField` relies on repeat
///   for held backspace, and a second press would be swallowed.
/// * **A grabbed cursor is free-running.** While the pointer is captured the game
///   computes a *delta* from consecutive positions, so clamping to the window
///   would silently cap how far you can turn. The virtual position therefore
///   leaves the window, exactly like a desktop mouse under `GLFW_CURSOR_DISABLED`
///   — and is rebased (once, with a counter) long before an `i32` could overflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameInputTranslator {
    framebuffer: (u32, u32),
    cursor: (i32, i32),
    grabbed: bool,
    inside: bool,
    mods: i32,
    held_keys: BTreeSet<i32>,
    held_buttons: BTreeSet<i32>,
    stats: GameInputStats,
}

impl Default for GameInputTranslator {
    fn default() -> Self {
        Self::new(854, 480)
    }
}

impl GameInputTranslator {
    /// How far the free-running captured cursor may travel from the origin before
    /// it is rebased to the window centre.
    ///
    /// 16.7 M pixels is ~28 minutes of *uninterrupted* motion in one direction at
    /// 10 000 px/s, so in practice this never fires — but it means an `i32` can
    /// never overflow no matter how long a session runs (task 19).
    pub const CURSOR_REBASE_PX: i32 = 1 << 24;

    /// Hard cap on held keys, so a stuck-key storm cannot grow the set.
    pub const MAX_HELD_KEYS: usize = 64;

    /// A translator for a `width x height` game window.
    pub fn new(width: u32, height: u32) -> Self {
        let framebuffer = (width.max(1), height.max(1));
        Self {
            framebuffer,
            cursor: (framebuffer.0 as i32 / 2, framebuffer.1 as i32 / 2),
            grabbed: false,
            inside: true,
            mods: 0,
            held_keys: BTreeSet::new(),
            held_buttons: BTreeSet::new(),
            stats: GameInputStats::default(),
        }
    }

    /// Current window size.
    pub fn framebuffer(&self) -> (u32, u32) {
        self.framebuffer
    }

    /// Tell the game about a new window size (and keep the cursor inside it).
    pub fn set_framebuffer(&mut self, width: u32, height: u32) -> Vec<GameInputEvent> {
        let next = (width.max(1), height.max(1));
        if next == self.framebuffer {
            return Vec::new();
        }
        self.framebuffer = next;
        let mut out = vec![
            GameInputEvent::framebuffer_size(next.0 as i32, next.1 as i32),
            GameInputEvent::window_size(next.0 as i32, next.1 as i32),
        ];
        if !self.grabbed {
            let clamped = self.clamp_cursor(self.cursor);
            if clamped != self.cursor {
                self.cursor = clamped;
                self.stats.cursor_events += 1;
                out.push(GameInputEvent::cursor(clamped.0, clamped.1));
            }
        }
        out
    }

    /// Current cursor position, in window pixels (may be outside the window while
    /// the pointer is captured — that is the point).
    pub fn cursor(&self) -> (i32, i32) {
        self.cursor
    }

    /// Whether the pointer is captured.
    pub fn is_grabbed(&self) -> bool {
        self.grabbed
    }

    /// GLFW modifier bits currently held.
    pub fn mods(&self) -> i32 {
        self.mods
    }

    /// Keys currently held (GLFW codes).
    pub fn held_keys(&self) -> usize {
        self.held_keys.len()
    }

    /// Capture / release the pointer.
    ///
    /// Capturing re-centres the cursor first: the game reads the cursor position
    /// when it grabs, and starting from the centre is what stops the view from
    /// snapping on the first captured sample.
    pub fn set_grab(&mut self, grabbing: bool) -> Vec<GameInputEvent> {
        if grabbing == self.grabbed {
            return Vec::new();
        }
        self.grabbed = grabbing;
        self.stats.grabs += 1;
        let mut out = Vec::new();
        let centre = self.centre();
        if self.cursor != centre {
            self.cursor = centre;
            self.stats.cursor_events += 1;
            out.push(GameInputEvent::cursor(centre.0, centre.1));
        }
        out.push(GameInputEvent::grab(grabbing));
        out
    }

    /// Move the cursor to an absolute window position (touch, or an absolute
    /// mouse). Clamped into the window; emits nothing when it did not move.
    pub fn move_to(&mut self, x: i32, y: i32) -> Vec<GameInputEvent> {
        let next = self.clamp_cursor((x, y));
        if next == self.cursor {
            return Vec::new();
        }
        self.cursor = next;
        self.stats.cursor_events += 1;
        vec![GameInputEvent::cursor(next.0, next.1)]
    }

    /// Move the cursor by a *relative* step (a captured physical mouse).
    ///
    /// While grabbed the position is free-running (see the type docs); while not
    /// grabbed it behaves like a desktop mouse and stops at the window edge.
    pub fn move_by(&mut self, dx: i32, dy: i32) -> Vec<GameInputEvent> {
        if dx == 0 && dy == 0 {
            return Vec::new();
        }
        let raw = (
            self.cursor.0.saturating_add(dx),
            self.cursor.1.saturating_add(dy),
        );
        let mut out = Vec::new();
        let next = if self.grabbed {
            if raw.0.abs() > Self::CURSOR_REBASE_PX || raw.1.abs() > Self::CURSOR_REBASE_PX {
                self.stats.rebases += 1;
                self.centre()
            } else {
                raw
            }
        } else {
            self.clamp_cursor(raw)
        };
        if next != self.cursor {
            self.cursor = next;
            self.stats.cursor_events += 1;
            out.push(GameInputEvent::cursor(next.0, next.1));
        }
        out
    }

    /// A key press / release by GLFW code, with the physical scancode.
    ///
    /// `scancode <= 0` is filled in from [`scancode_for_glfw_key`], so a
    /// synthetic press (an on-screen button, a gamepad binding) still carries the
    /// number Minecraft needs for `InputConstants.getKey`.
    pub fn key(&mut self, key: i32, scancode: i32, down: bool) -> Vec<GameInputEvent> {
        let scancode = if scancode > 0 {
            scancode
        } else {
            scancode_for_glfw_key(key)
        };
        let action = if down {
            if self.held_keys.contains(&key) {
                glfw::REPEAT
            } else {
                if self.held_keys.len() >= Self::MAX_HELD_KEYS {
                    self.stats.rejected += 1;
                    return Vec::new();
                }
                self.held_keys.insert(key);
                glfw::PRESS
            }
        } else {
            self.held_keys.remove(&key);
            glfw::RELEASE
        };
        // The modifier bit must already be set for the press that *is* the
        // modifier (GLFW reports `mods` including the key being pressed).
        if let Some(bit) = glfw_mod_for_key(key) {
            if down {
                self.mods |= bit;
            } else {
                self.mods &= !bit;
            }
        }
        self.stats.key_events += 1;
        vec![GameInputEvent::key(key, scancode, action, self.mods)]
    }

    /// A key press / release by *name*; `None` when GLFW has no such key (the
    /// caller then sends the character instead).
    pub fn key_named(
        &mut self,
        name: &str,
        scancode: Option<i32>,
        down: bool,
    ) -> Option<Vec<GameInputEvent>> {
        let key = glfw_key_for_key(name)?;
        Some(self.key(key, scancode.unwrap_or(0), down))
    }

    /// A typed character (soft keyboard, IME, or a hardware key's unicode value).
    pub fn character(&mut self, ch: char) -> Vec<GameInputEvent> {
        self.stats.char_events += 1;
        vec![GameInputEvent::character(ch, self.mods)]
    }

    /// A mouse button press / release.
    pub fn button(&mut self, button: MouseButton, down: bool) -> Vec<GameInputEvent> {
        let glfw_button = glfw_button(button);
        if down {
            self.held_buttons.insert(glfw_button);
        } else if !self.held_buttons.remove(&glfw_button) {
            // A release without a press would make the game think a click
            // happened that never did (and Minecraft would keep mining).
            self.stats.rejected += 1;
            return Vec::new();
        }
        self.stats.button_events += 1;
        vec![GameInputEvent::mouse_button(
            glfw_button,
            if down { glfw::PRESS } else { glfw::RELEASE },
            self.mods,
        )]
    }

    /// A wheel scroll, in notches (positive `dy` scrolls up, as GLFW reports it).
    pub fn scroll(&mut self, dx_milli: i32, dy_milli: i32) -> Vec<GameInputEvent> {
        if dx_milli == 0 && dy_milli == 0 {
            return Vec::new();
        }
        self.stats.scroll_events += 1;
        vec![GameInputEvent::scroll(dx_milli, dy_milli)]
    }

    /// The pointer entered / left the window.
    pub fn set_inside(&mut self, inside: bool) -> Vec<GameInputEvent> {
        if inside == self.inside {
            return Vec::new();
        }
        self.inside = inside;
        vec![GameInputEvent::cursor_enter(inside)]
    }

    /// Release every held key and button (focus loss / app backgrounded).
    ///
    /// Without this the game would keep walking forward after the launcher lost
    /// focus with `W` held — the single most reported "the game is possessed" bug.
    pub fn release_all(&mut self) -> Vec<GameInputEvent> {
        let mut out = Vec::new();
        let buttons: Vec<i32> = self.held_buttons.iter().copied().collect();
        for button in buttons {
            self.held_buttons.remove(&button);
            self.stats.button_events += 1;
            out.push(GameInputEvent::mouse_button(
                button,
                glfw::RELEASE,
                self.mods,
            ));
        }
        let keys: Vec<i32> = self.held_keys.iter().copied().collect();
        for key in keys {
            self.held_keys.remove(&key);
            if let Some(bit) = glfw_mod_for_key(key) {
                self.mods &= !bit;
            }
            self.stats.key_events += 1;
            out.push(GameInputEvent::key(
                key,
                scancode_for_glfw_key(key),
                glfw::RELEASE,
                self.mods,
            ));
        }
        self.mods = 0;
        out
    }

    /// Forget all state (a new game session): no events, just state.
    pub fn reset(&mut self) {
        self.cursor = self.centre();
        self.grabbed = false;
        self.inside = true;
        self.mods = 0;
        self.held_keys.clear();
        self.held_buttons.clear();
    }

    /// Diagnostics counters.
    pub fn stats(&self) -> GameInputStats {
        self.stats
    }

    /// JSON snapshot for the diagnostics panel.
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "framebuffer": { "width": self.framebuffer.0, "height": self.framebuffer.1 },
            "cursor": { "x": self.cursor.0, "y": self.cursor.1 },
            "grabbed": self.grabbed,
            "inside": self.inside,
            "mods": self.mods,
            "held_keys": self.held_keys.len(),
            "held_buttons": self.held_buttons.len(),
            "stats": self.stats,
        })
    }

    fn centre(&self) -> (i32, i32) {
        (self.framebuffer.0 as i32 / 2, self.framebuffer.1 as i32 / 2)
    }

    fn clamp_cursor(&self, at: (i32, i32)) -> (i32, i32) {
        (
            at.0.clamp(0, self.framebuffer.0 as i32 - 1),
            at.1.clamp(0, self.framebuffer.1 as i32 - 1),
        )
    }
}

/// Encode a batch of game events as event-channel records.
pub fn to_records(events: &[GameInputEvent]) -> Vec<AwtEventRecord> {
    events.iter().map(GameInputEvent::to_record).collect()
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- key vocabulary ----------------------------------------------------

    #[test]
    fn glfw_keys_cover_letters_digits_and_function_keys() {
        assert_eq!(glfw_key_for_key("w"), Some(glfw::KEY_A + 22));
        assert_eq!(glfw_key_for_key("W"), Some('W' as i32));
        assert_eq!(glfw_key_for_key("key.keyboard.w"), Some('W' as i32));
        assert_eq!(glfw_key_for_key("5"), Some('5' as i32));
        assert_eq!(glfw_key_for_key("f3"), Some(292));
        assert_eq!(glfw_key_for_key("f25"), Some(314));
        assert_eq!(glfw_key_for_key("f26"), None);
        assert_eq!(glfw_key_for_key("keypad.7"), Some(glfw::KEY_KP_0 + 7));
        assert_eq!(glfw_key_for_key("keypad.enter"), Some(glfw::KEY_KP_ENTER));
    }

    #[test]
    fn glfw_keeps_the_side_awt_throws_away() {
        // The whole point of the GLFW table: Minecraft can bind left and right
        // shift separately, AWT cannot even express it.
        assert_eq!(glfw_key_for_key("left.shift"), Some(glfw::KEY_LEFT_SHIFT));
        assert_eq!(glfw_key_for_key("right.shift"), Some(glfw::KEY_RIGHT_SHIFT));
        assert_eq!(vk_for_key("left.shift"), vk_for_key("right.shift"));
        assert_eq!(
            glfw_key_for_key("key.keyboard.control.left"),
            Some(glfw::KEY_LEFT_CONTROL)
        );
        assert_eq!(
            glfw_key_for_key("key.keyboard.left.win"),
            Some(glfw::KEY_LEFT_SUPER)
        );
    }

    #[test]
    fn the_task_15_control_layout_vocabulary_resolves() {
        // Every keyboard code the Kotlin `MappedKey` enum can store must reach
        // the game, or a control layout silently loses buttons.
        for name in [
            "key.keyboard.w",
            "key.keyboard.space",
            "key.keyboard.shift.left",
            "key.keyboard.control.left",
            "key.keyboard.alt.left",
            "key.keyboard.escape",
            "key.keyboard.tab",
            "key.keyboard.enter",
            "key.keyboard.slash",
            "key.keyboard.caps.lock",
            "key.keyboard.backspace",
            "key.keyboard.delete",
            "key.keyboard.home",
            "key.keyboard.end",
            "key.keyboard.page.up",
            "key.keyboard.page.down",
            "key.keyboard.minus",
            "key.keyboard.equal",
            "key.keyboard.bracket.left",
            "key.keyboard.bracket.right",
            "key.keyboard.semicolon",
            "key.keyboard.apostrophe",
            "key.keyboard.comma",
            "key.keyboard.period",
            "key.keyboard.up",
            "key.keyboard.down",
            "key.keyboard.left",
            "key.keyboard.right",
            "key.keyboard.f1",
            "key.keyboard.f5",
            "key.keyboard.left.win",
            "key.keyboard.0",
            "key.keyboard.9",
        ] {
            assert!(
                glfw_key_for_key(name).is_some(),
                "control-layout key {name} has no GLFW code"
            );
        }
        for name in ["key.mouse.left", "key.mouse.right", "key.mouse.middle"] {
            assert!(
                mouse_button_for_name(name).is_some(),
                "control-layout button {name} does not resolve"
            );
        }
    }

    #[test]
    fn unknown_names_are_none_instead_of_a_wrong_key() {
        assert_eq!(glfw_key_for_key("fn"), None);
        assert_eq!(glfw_key_for_key(""), None);
        assert_eq!(glfw_key_for_key("kana"), None);
        assert_eq!(mouse_button_for_name("key.keyboard.w"), None);
    }

    #[test]
    fn scancodes_are_the_evdev_numbers_android_reports() {
        assert_eq!(scancode_for_key("w"), 17); // KEY_W
        assert_eq!(scancode_for_key("a"), 30); // KEY_A
        assert_eq!(scancode_for_key("z"), 44); // KEY_Z
        assert_eq!(scancode_for_key("1"), 2); // KEY_1
        assert_eq!(scancode_for_key("0"), 11); // KEY_0
        assert_eq!(scancode_for_key("escape"), 1);
        assert_eq!(scancode_for_key("space"), 57);
        assert_eq!(scancode_for_key("left.shift"), 42);
        assert_eq!(scancode_for_key("right.shift"), 54);
        assert_eq!(scancode_for_key("f5"), 63);
        assert_eq!(scancode_for_key("f11"), 87);
        assert_eq!(scancode_for_key("keypad.0"), 82);
        assert_eq!(scancode_for_key("keypad.7"), 71);
        assert_eq!(scancode_for_key("nope"), 0);
    }

    #[test]
    fn vk_maps_onto_glfw_and_its_scancode() {
        assert_eq!(glfw_key_for_vk(vk_for_key("w").unwrap()), Some('W' as i32));
        assert_eq!(scancode_for_vk(vk_for_key("w").unwrap()), 17);
        assert_eq!(
            glfw_key_for_vk(vk_for_key("f1").unwrap()),
            Some(glfw::KEY_F1)
        );
        assert_eq!(
            glfw_key_for_vk(vk_for_key("keypad.0").unwrap()),
            Some(glfw::KEY_KP_0)
        );
        // AWT collapsed the side, so the VK path can only produce the left key.
        assert_eq!(
            glfw_key_for_vk(vk_for_key("right.shift").unwrap()),
            Some(glfw::KEY_LEFT_SHIFT)
        );
        assert_eq!(glfw_key_for_vk(-7), None);
        assert_eq!(scancode_for_vk(-7), 0);
    }

    #[test]
    fn glfw_orders_right_before_middle() {
        // Off-by-one traps live here: AWT is 1/2/3 (left/middle/right) while GLFW
        // is 0/1/2 (left/right/middle).
        assert_eq!(glfw_button(MouseButton::Left), 0);
        assert_eq!(glfw_button(MouseButton::Right), 1);
        assert_eq!(glfw_button(MouseButton::Middle), 2);
        assert_eq!(MouseButton::Middle.number(), 2);
        assert_eq!(MouseButton::Right.number(), 3);
    }

    #[test]
    fn modifier_bits_follow_the_key() {
        assert_eq!(
            glfw_mod_for_key(glfw::KEY_RIGHT_CONTROL),
            Some(glfw::MOD_CONTROL)
        );
        assert_eq!(glfw_mod_for_key(glfw::KEY_LEFT_ALT), Some(glfw::MOD_ALT));
        assert_eq!(glfw_mod_for_key('W' as i32), None);
    }

    #[test]
    fn names_are_normalised_the_same_way_everywhere() {
        assert_eq!(
            normalise_key_name("  KEY.KEYBOARD.LEFT_SHIFT "),
            "left.shift"
        );
        assert_eq!(normalise_key_name("Left Shift"), "left.shift");
        assert_eq!(
            glfw_key_for_key("LEFT_SHIFT"),
            glfw_key_for_key("left.shift")
        );
    }

    // ---- settings ----------------------------------------------------------

    #[test]
    fn pointer_mode_round_trips_and_degrades() {
        for mode in [PointerMode::Absolute, PointerMode::Captured] {
            assert_eq!(PointerMode::from_id(mode.id()), Some(mode));
        }
        assert_eq!(PointerMode::from_id("grabbed"), Some(PointerMode::Captured));
        assert_eq!(PointerMode::from_id("sideways"), None);
        assert!(PointerMode::Captured.is_captured());
    }

    #[test]
    fn pointer_source_degrades_to_touch() {
        assert_eq!(PointerSource::from_id("mouse"), PointerSource::Mouse);
        assert_eq!(PointerSource::from_id("pen"), PointerSource::Stylus);
        assert_eq!(PointerSource::from_id("nonsense"), PointerSource::Touch);
        assert!(PointerSource::Stylus.is_mouse_like());
        assert!(!PointerSource::Touch.is_mouse_like());
    }

    #[test]
    fn sensitivity_is_clamped_and_nan_safe() {
        assert_eq!(MouseSensitivity::uniform(1.0).x_permille, 1000);
        assert_eq!(MouseSensitivity::uniform(0.0).x_permille, 100);
        assert_eq!(MouseSensitivity::uniform(1e9).x_permille, 10_000);
        assert_eq!(MouseSensitivity::uniform(f32::NAN).x_permille, 1000);
        assert_eq!(MouseSensitivity::uniform(2.5).x(), 2.5);
        let hostile = MouseSensitivity {
            x_permille: 0,
            y_permille: 99_999,
            invert_y: true,
        }
        .sanitized();
        assert_eq!(hostile.x_permille, 100);
        assert_eq!(hostile.y_permille, 10_000);
        assert!(hostile.invert_y);
    }

    #[test]
    fn sensitivity_merges_a_partial_json_update() {
        let merged = MouseSensitivity::default().merged_from_json(&json!({ "x": 2.0 }));
        assert_eq!(merged.x_permille, 2000);
        assert_eq!(merged.y_permille, 1000, "y must keep its old value");
        let merged = merged.merged_from_json(&json!({ "y_permille": 1500, "invert_y": true }));
        assert_eq!(merged.y_permille, 1500);
        assert!(merged.invert_y);
        // A garbage document changes nothing.
        assert_eq!(merged.merged_from_json(&json!("nope")), merged);
    }

    #[test]
    fn slow_movement_accumulates_instead_of_vanishing() {
        // 0.4x: a stream of 1 px samples must still move the pointer, or a low
        // sensitivity would look like a dead mouse.
        let sensitivity = MouseSensitivity::uniform(0.4);
        let mut motion = MouseMotion::default();
        let steps: Vec<i32> = (0..5)
            .map(|_| motion.step(&sensitivity, 1.0, 0.0).0)
            .collect();
        assert_eq!(steps, vec![0, 0, 1, 0, 1]);
        assert_eq!(motion.samples(), 5);
        assert_eq!(motion.coalesced(), 3);
    }

    #[test]
    fn motion_scales_inverts_and_survives_garbage() {
        let mut motion = MouseMotion::default();
        let fast = MouseSensitivity::uniform(2.0);
        assert_eq!(motion.step(&fast, 3.0, -2.0), (6, -4));
        let inverted = MouseSensitivity::uniform(1.0).with_invert_y(true);
        assert_eq!(motion.step(&inverted, 0.0, 5.0), (0, -5));
        // NaN / infinity are garbage and contribute nothing at all…
        assert_eq!(motion.step(&fast, f32::NAN, f32::INFINITY), (0, 0));
        // …while an absurd but *finite* delta is clamped instead of teleporting
        // the crosshair across the world.
        assert_eq!(
            motion.step(&fast, 1e9, 0.0),
            ((MouseMotion::MAX_DELTA_PX * 2.0) as i32, 0)
        );
        motion.reset();
        assert_eq!(
            motion.step(&MouseSensitivity::uniform(0.1), 1.0, 0.0),
            (0, 0)
        );
    }

    #[test]
    fn bindings_are_single_hop_so_a_cycle_is_impossible() {
        let mut bindings = InputBindings::new();
        bindings.bind_key("e", "f").unwrap();
        bindings.bind_key("f", "e").unwrap();
        assert_eq!(bindings.resolve_key("e"), "f");
        assert_eq!(bindings.resolve_key("f"), "e");
        assert_eq!(bindings.resolve_key("key.keyboard.e"), "f");
        assert_eq!(bindings.resolve_key("q"), "q");
        assert_eq!(bindings.key_count(), 2);
    }

    #[test]
    fn bindings_reject_a_name_no_key_layer_knows() {
        let mut bindings = InputBindings::new();
        assert!(bindings.bind_key("e", "banana").is_err());
        assert!(bindings.bind_key("banana", "e").is_err());
        assert!(bindings.bind_key("", "e").is_err());
        assert!(bindings.is_empty(), "a rejected binding must not be stored");
        // Binding a key to itself is how the UI clears one.
        bindings.bind_key("e", "f").unwrap();
        bindings.bind_key("e", "e").unwrap();
        assert!(bindings.is_empty());
    }

    #[test]
    fn button_bindings_swap_and_round_trip_through_json() {
        let mut bindings = InputBindings::new();
        bindings.bind_button(MouseButton::Left, MouseButton::Right);
        bindings
            .bind_key("key.keyboard.e", "key.keyboard.f")
            .unwrap();
        assert_eq!(
            bindings.resolve_button(MouseButton::Left),
            MouseButton::Right
        );
        assert_eq!(
            bindings.resolve_button(MouseButton::Middle),
            MouseButton::Middle
        );
        let (parsed, skipped) = InputBindings::from_json(&bindings.to_json());
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(parsed, bindings);
        bindings.bind_button(MouseButton::Left, MouseButton::Left);
        assert_eq!(bindings.button_count(), 0);
    }

    #[test]
    fn a_stale_binding_does_not_cost_the_whole_table() {
        let (parsed, skipped) = InputBindings::from_json(&json!({
            "keys": { "e": "f", "banana": "q", "w": 7 },
            "buttons": { "1": 3, "9": 1 },
        }));
        assert_eq!(parsed.resolve_key("e"), "f");
        assert_eq!(parsed.resolve_button(MouseButton::Left), MouseButton::Right);
        assert_eq!(skipped.len(), 3, "{skipped:?}");
    }

    #[test]
    fn bindings_are_bounded() {
        let mut bindings = InputBindings::new();
        for i in 0..InputBindings::MAX_ENTRIES {
            let from = format!("f{}", (i % 24) + 1);
            let to = format!("f{}", (i % 23) + 2);
            let _ = bindings.bind_key(&from, &to);
        }
        assert!(bindings.key_count() <= InputBindings::MAX_ENTRIES);
    }

    #[test]
    fn hybrid_mode_decides_which_device_may_move_the_pointer() {
        let mut settings = InputSettings::new();
        // Absolute: everything is welcome.
        assert!(settings.accepts(PointerSource::Touch));
        assert!(settings.accepts(PointerSource::Mouse));
        settings.pointer_mode = PointerMode::Captured;
        settings.hybrid_touch = false;
        assert!(
            !settings.accepts(PointerSource::Touch),
            "a palm must not turn the view"
        );
        assert!(settings.accepts(PointerSource::Mouse));
        assert!(settings.accepts(PointerSource::Stylus));
        settings.hybrid_touch = true;
        assert!(settings.accepts(PointerSource::Touch));
    }

    #[test]
    fn a_real_notch_never_scales_down_to_no_scroll() {
        let mut settings = InputSettings::new();
        assert_eq!(settings.scale_scroll(3), 3);
        settings.scroll_permille = 100;
        assert_eq!(settings.scale_scroll(1), 1);
        assert_eq!(settings.scale_scroll(-1), -1);
        assert_eq!(settings.scale_scroll(0), 0);
        settings.scroll_permille = 3_000;
        assert_eq!(settings.scale_scroll(2), 6);
    }

    #[test]
    fn settings_apply_a_partial_json_update_and_report_the_rest() {
        let mut settings = InputSettings::new();
        let notes = settings.apply_json(&json!({
            "pointer_mode": "captured",
            "hybrid_touch": false,
            "sensitivity": { "x": 1.5, "invert_y": true },
            "scroll_permille": 2000,
            "bindings": { "keys": { "e": "f", "banana": "q" } },
        }));
        assert_eq!(settings.pointer_mode, PointerMode::Captured);
        assert!(!settings.hybrid_touch);
        assert_eq!(settings.sensitivity.x_permille, 1500);
        assert!(settings.sensitivity.invert_y);
        assert_eq!(settings.scroll_permille, 2000);
        assert_eq!(settings.bindings.resolve_key("e"), "f");
        assert_eq!(notes.len(), 1, "{notes:?}");
        // `captured: false` is the other spelling of the same switch.
        settings.apply_json(&json!({ "captured": false }));
        assert_eq!(settings.pointer_mode, PointerMode::Absolute);
        // Nonsense is reported, and everything else survives.
        let notes = settings.apply_json(&json!({ "pointer_mode": "sideways" }));
        assert_eq!(notes.len(), 1);
        assert_eq!(settings.pointer_mode, PointerMode::Absolute);
        assert_eq!(settings.sensitivity.x_permille, 1500);
    }

    #[test]
    fn settings_json_carries_every_key_the_ui_reads() {
        let json = InputSettings::new().to_json();
        for key in [
            "pointer_mode",
            "captured",
            "hybrid_touch",
            "native_input",
            "sensitivity",
            "scroll_permille",
            "bindings",
        ] {
            assert!(json.get(key).is_some(), "settings JSON lost {key}");
        }
        assert!(json["sensitivity"].get("x").is_some());
        assert!(json["bindings"].get("keys").is_some());
    }

    // ---- the native game events -------------------------------------------

    #[test]
    fn game_events_round_trip_through_a_record() {
        let events = [
            GameInputEvent::key('W' as i32, 17, glfw::PRESS, glfw::MOD_SHIFT),
            GameInputEvent::character('好', 0),
            GameInputEvent::cursor(-40, 7000),
            GameInputEvent::cursor_enter(false),
            GameInputEvent::mouse_button(glfw::MOUSE_BUTTON_RIGHT, glfw::RELEASE, 0),
            GameInputEvent::scroll(0, -1000),
            GameInputEvent::framebuffer_size(1920, 1080),
            GameInputEvent::window_size(854, 480),
            GameInputEvent::grab(true),
        ];
        for event in events {
            let record = event.to_record();
            assert_eq!(record.id, GAME_INPUT_EVENT_ID);
            assert!(!record.is_control(), "must not look like a control record");
            assert_eq!(GameInputEvent::from_record(&record), Some(event));
        }
        assert_eq!(to_records(&events).len(), events.len());
    }

    #[test]
    fn a_foreign_record_is_not_a_game_event() {
        let awt = AwtEventRecord {
            id: 401,
            ..Default::default()
        };
        assert_eq!(GameInputEvent::from_record(&awt), None);
        // Right id, nonsense kind: refused rather than sent to the game.
        let bogus = AwtEventRecord {
            id: GAME_INPUT_EVENT_ID,
            x: 4242,
            ..Default::default()
        };
        assert_eq!(GameInputEvent::from_record(&bogus), None);
        assert!(!game_event::is_known(4242));
        assert_eq!(game_event::name(4242), "unknown");
    }

    #[test]
    fn the_payload_is_the_callback_bridge_parameter_list() {
        let key = GameInputEvent::key('W' as i32, 17, glfw::PRESS, 0);
        assert_eq!(key.payload(), "87,17,1,0");
        assert_eq!(key.describe(), "key(87,17,1,0)");
        assert_eq!(GameInputEvent::cursor(3, 4).payload(), "3,4");
        assert_eq!(GameInputEvent::character('A', 1).payload(), "65");
        assert_eq!(
            GameInputEvent::mouse_button(1, glfw::PRESS, 2).payload(),
            "1,1,2"
        );
        assert_eq!(GameInputEvent::grab(true).payload(), "1");
        assert!(GameInputEvent::cursor(0, 0).is_motion());
        assert!(GameInputEvent::scroll(0, 1).is_motion());
        assert!(!GameInputEvent::key(1, 1, 1, 0).is_motion());
    }

    // ---- the native translator --------------------------------------------

    #[test]
    fn a_held_key_repeats_instead_of_pressing_twice() {
        let mut game = GameInputTranslator::new(854, 480);
        let first = game.key('W' as i32, 17, true);
        assert_eq!(first[0].p2, glfw::PRESS);
        let again = game.key('W' as i32, 17, true);
        assert_eq!(again[0].p2, glfw::REPEAT, "chat backspace needs repeat");
        let up = game.key('W' as i32, 17, false);
        assert_eq!(up[0].p2, glfw::RELEASE);
        assert_eq!(game.held_keys(), 0);
    }

    #[test]
    fn a_missing_scancode_is_filled_in() {
        let mut game = GameInputTranslator::new(854, 480);
        let events = game.key('W' as i32, 0, true);
        assert_eq!(events[0].p1, 17, "the core must supply the evdev code");
        let events = game.key('A' as i32, 99, true);
        assert_eq!(events[0].p1, 99, "a real scancode wins");
    }

    #[test]
    fn a_modifier_press_already_carries_its_own_bit() {
        let mut game = GameInputTranslator::new(854, 480);
        let shift = game.key(glfw::KEY_LEFT_SHIFT, 42, true);
        assert_eq!(shift[0].p3, glfw::MOD_SHIFT);
        let w = game.key('W' as i32, 17, true);
        assert_eq!(w[0].p3, glfw::MOD_SHIFT, "sprint needs the modifier");
        let up = game.key(glfw::KEY_LEFT_SHIFT, 42, false);
        assert_eq!(up[0].p3, 0);
        assert_eq!(game.mods(), 0);
    }

    #[test]
    fn a_release_without_a_press_is_refused() {
        let mut game = GameInputTranslator::new(854, 480);
        assert!(game.button(MouseButton::Left, false).is_empty());
        assert_eq!(game.stats().rejected, 1);
        let down = game.button(MouseButton::Left, true);
        assert_eq!(down[0].p0, glfw::MOUSE_BUTTON_LEFT);
        assert_eq!(down[0].p1, glfw::PRESS);
        assert_eq!(game.button(MouseButton::Left, false).len(), 1);
    }

    #[test]
    fn an_absolute_cursor_stays_inside_the_window() {
        let mut game = GameInputTranslator::new(100, 50);
        assert_eq!(game.cursor(), (50, 25));
        game.move_to(-10, 900);
        assert_eq!(game.cursor(), (0, 49));
        assert!(game.move_to(0, 49).is_empty(), "no move, no event");
        game.move_by(5, 0);
        assert_eq!(game.cursor(), (5, 49));
        game.move_by(-500, 0);
        assert_eq!(
            game.cursor(),
            (0, 49),
            "a released pointer stops at the edge"
        );
    }

    #[test]
    fn a_grabbed_cursor_is_free_running_so_you_can_keep_turning() {
        let mut game = GameInputTranslator::new(100, 50);
        let grab = game.set_grab(true);
        assert_eq!(grab.last().unwrap().kind, game_event::GRAB_STATE);
        assert_eq!(grab.last().unwrap().p0, 1);
        assert_eq!(game.cursor(), (50, 25), "grabbing re-centres first");
        for _ in 0..10 {
            game.move_by(1000, 0);
        }
        assert_eq!(game.cursor().0, 50 + 10_000, "clamping would cap the turn");
        assert_eq!(game.stats().rebases, 0);
        // Releasing centres again and reports it.
        let release = game.set_grab(false);
        assert!(release.iter().any(|e| e.kind == game_event::CURSOR_POS));
        assert_eq!(game.cursor(), (50, 25));
        assert!(game.set_grab(false).is_empty(), "no change, no event");
    }

    #[test]
    fn the_free_running_cursor_can_never_overflow() {
        let mut game = GameInputTranslator::new(100, 50);
        game.set_grab(true);
        game.move_by(GameInputTranslator::CURSOR_REBASE_PX, 0);
        assert_eq!(game.cursor(), (50, 25), "rebased to the centre");
        assert_eq!(game.stats().rebases, 1);
        // Even a hostile stream of maximal steps stays finite.
        for _ in 0..8 {
            game.move_by(i32::MAX, i32::MAX);
        }
        assert!(game.cursor().0.abs() <= GameInputTranslator::CURSOR_REBASE_PX);
        assert!(game.cursor().1.abs() <= GameInputTranslator::CURSOR_REBASE_PX);
    }

    #[test]
    fn releasing_everything_unsticks_keys_buttons_and_modifiers() {
        let mut game = GameInputTranslator::new(854, 480);
        game.key(glfw::KEY_LEFT_SHIFT, 42, true);
        game.key('W' as i32, 17, true);
        game.button(MouseButton::Left, true);
        let released = game.release_all();
        assert_eq!(released.len(), 3);
        assert!(released
            .iter()
            .all(|e| e.p1 == glfw::RELEASE || e.kind == game_event::KEY));
        assert_eq!(game.held_keys(), 0);
        assert_eq!(game.mods(), 0);
        assert!(game.release_all().is_empty());
    }

    #[test]
    fn a_resize_tells_the_game_and_keeps_the_cursor_inside() {
        let mut game = GameInputTranslator::new(1280, 720);
        game.move_to(1279, 719);
        let events = game.set_framebuffer(640, 360);
        let kinds: Vec<i32> = events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                game_event::FRAMEBUFFER_SIZE,
                game_event::WINDOW_SIZE,
                game_event::CURSOR_POS
            ]
        );
        assert_eq!(game.cursor(), (639, 359));
        assert!(game.set_framebuffer(640, 360).is_empty());
        // A zero size is nonsense; the translator refuses to divide by it.
        game.set_framebuffer(0, 0);
        assert_eq!(game.framebuffer(), (1, 1));
    }

    #[test]
    fn the_snapshot_carries_the_state_the_panel_shows() {
        let mut game = GameInputTranslator::new(854, 480);
        game.set_grab(true);
        game.key('W' as i32, 17, true);
        let json = game.to_json();
        assert_eq!(json["grabbed"], json!(true));
        assert_eq!(json["held_keys"], json!(1));
        assert!(json["cursor"].get("x").is_some());
        assert!(json["framebuffer"].get("width").is_some());
        assert!(json["stats"].get("key_events").is_some());
        game.reset();
        assert!(!game.is_grabbed());
        assert_eq!(game.held_keys(), 0);
    }
}
