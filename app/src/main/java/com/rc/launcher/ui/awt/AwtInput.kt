package com.rc.launcher.ui.awt

import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.toJsonString

/**
 * Compose input → AWT events (task 18).
 *
 * The Rust core owns the *stateful* part of the translation (which buttons and
 * modifiers are held, `MOUSE_DRAGGED` vs `MOUSE_MOVED`, the synthetic
 * `MOUSE_CLICKED` after a steady tap, load shedding when the JVM stalls), so the
 * UI only has to describe *what happened*, in surface coordinates. This file is
 * that description plus its JSON encoding — one batch per UI frame, so a drag
 * costs a single JNI call instead of one per touch sample.
 *
 * Pure Kotlin (no Android imports): unit-testable on the JVM.
 */

/** Phase of a pointer gesture coming from Compose. */
enum class AwtPointerPhase(val id: String) {
    DOWN("down"),
    MOVE("move"),
    UP("up"),
}

/**
 * Which device produced a pointer sample (task 12).
 *
 * The core needs it for two decisions: a finger is filtered out while the pointer
 * is captured unless the player asked for the mixed touch+mouse mode, and only a
 * non-captured sample may place the game's cursor absolutely.
 *
 * Mirrors `launch::input::PointerSource`; ids must stay identical
 * (`scripts/check_awt_wire.py` compares them).
 */
enum class AwtPointerSource(val id: String) {
    TOUCH("touch"),
    MOUSE("mouse"),
    STYLUS("stylus"),
    ;

    /** Whether the device has a hover position (so relative motion applies). */
    val isMouseLike: Boolean get() = this != TOUCH

    companion object {
        /** Parse an id; anything unknown degrades to [TOUCH], as the core does. */
        fun fromId(id: String?): AwtPointerSource =
            AwtPointerSource.entries.firstOrNull { it.id == id } ?: TOUCH
    }
}

/** Mouse buttons in `java.awt.event.MouseEvent.BUTTON*` numbering. */
enum class AwtMouseButton(val id: String, val number: Int) {
    /** Left click / a single-finger tap. */
    LEFT("left", 1),

    /** Middle click. */
    MIDDLE("middle", 2),

    /** Right click / a long press. */
    RIGHT("right", 3);

    companion object {
        fun fromNumber(number: Int): AwtMouseButton =
            values().firstOrNull { it.number == number } ?: LEFT
    }
}

/** One input event handed to the core (see [toBatchJson]). */
sealed interface AwtInputEvent {
    /** JSON form understood by `RustBridge.awtInput`. */
    fun toJson(): JsonValue
}

/** A touch / mouse position in **surface** pixels. */
data class AwtPointerEvent(
    val phase: AwtPointerPhase,
    val x: Float,
    val y: Float,
    val button: AwtMouseButton = AwtMouseButton.LEFT,
    /** Which device produced it (task 12); the core filters on this. */
    val source: AwtPointerSource = AwtPointerSource.TOUCH,
) : AwtInputEvent {
    override fun toJson(): JsonValue = JsonValue.Obj(
        linkedMapOf(
            "type" to JsonValue.Str("pointer"),
            "phase" to JsonValue.Str(phase.id),
            "x" to num(x),
            "y" to num(y),
            "button" to JsonValue.Str(button.id),
            "source" to JsonValue.Str(source.id),
        ),
    )
}

/**
 * *Relative* motion from a physical mouse (task 12).
 *
 * Once Android captures the pointer (`View.requestPointerCapture`) it reports how
 * far the mouse moved, not where it is — there is no "where" any more, which is
 * exactly what lets the view keep turning past the edge of the screen. The core
 * scales the delta by the player's sensitivity, remembers the sub-pixel
 * remainder, and drives both the AWT pointer and the game's own cursor with it.
 */
data class AwtRelativePointerEvent(
    val dx: Float,
    val dy: Float,
    val source: AwtPointerSource = AwtPointerSource.MOUSE,
) : AwtInputEvent {
    override fun toJson(): JsonValue = JsonValue.Obj(
        linkedMapOf(
            "type" to JsonValue.Str("pointer_relative"),
            "dx" to num(dx),
            "dy" to num(dy),
            "source" to JsonValue.Str(source.id),
        ),
    )
}

/**
 * A mouse button press / release **at the current pointer position** (task 12).
 *
 * A captured mouse has no surface position to report — that is what capturing
 * means — so its clicks have to land wherever the virtual pointer is. The core
 * owns that position, which is why this event carries no coordinates at all.
 */
data class AwtButtonEvent(
    val button: AwtMouseButton,
    val down: Boolean,
) : AwtInputEvent {
    override fun toJson(): JsonValue = JsonValue.Obj(
        linkedMapOf(
            "type" to JsonValue.Str("button"),
            "button" to JsonValue.Str(button.id),
            "down" to JsonValue.Bool(down),
        ),
    )
}

/**
 * A wheel scroll at the current pointer position (task 12).
 *
 * The captured-pointer sibling of [AwtScrollEvent]: same sign convention, no
 * coordinates.
 */
data class AwtScrollAtPointerEvent(val ticks: Int) : AwtInputEvent {
    override fun toJson(): JsonValue = JsonValue.Obj(
        linkedMapOf(
            "type" to JsonValue.Str("scroll"),
            "ticks" to JsonValue.Num(ticks.toDouble()),
        ),
    )
}

/**
 * The pointer was captured or released (task 12).
 *
 * Capturing tells the game it owns the cursor (`ANDROID_TYPE_GRAB_STATE`), which
 * is what makes Minecraft hide the crosshair-less system pointer and start
 * treating motion as "look around".
 */
data class AwtCaptureEvent(val captured: Boolean) : AwtInputEvent {
    override fun toJson(): JsonValue = JsonValue.Obj(
        linkedMapOf(
            "type" to JsonValue.Str("capture"),
            "captured" to JsonValue.Bool(captured),
        ),
    )
}

/**
 * A scroll gesture in surface pixels.
 *
 * `ticks > 0` is *toward* the user ("scroll down"), the sign
 * `java.awt.event.MouseWheelEvent.getWheelRotation()` and Compose's
 * `scrollDelta.y` both use. GLFW is the odd one out and the core negates it
 * there, so the UI never has to think about it.
 */
data class AwtScrollEvent(val x: Float, val y: Float, val ticks: Int) : AwtInputEvent {
    override fun toJson(): JsonValue = JsonValue.Obj(
        linkedMapOf(
            "type" to JsonValue.Str("scroll"),
            "x" to num(x),
            "y" to num(y),
            "ticks" to JsonValue.Num(ticks.toDouble()),
        ),
    )
}

/**
 * A key press / release, identified either by a `KeyEvent.VK_*` [code] or by a
 * Minecraft/GLFW-style [name] (`"escape"`, `"key.keyboard.left.shift"`, `"w"`),
 * which the core resolves — and degrades to typed text for names AWT has no code
 * for, so an exotic binding still reaches the game.
 */
data class AwtKeyEvent(
    val down: Boolean,
    val code: Int? = null,
    val name: String? = null,
    /**
     * The **physical** scancode, when Android reported one (task 12).
     *
     * `KeyEvent.getScanCode()` is the Linux evdev code, which is exactly what
     * `GLFWKeyCallback` takes — Minecraft falls back to it for every key its own
     * enumeration does not cover, so forwarding it verbatim is what makes a
     * non-US layout usable. `null` / `0` lets the core fill it in from its table.
     */
    val scancode: Int? = null,
) : AwtInputEvent {
    init {
        require(code != null || !name.isNullOrBlank()) { "a key event needs a code or a name" }
    }

    override fun toJson(): JsonValue {
        val entries = linkedMapOf<String, JsonValue>(
            "type" to JsonValue.Str(if (down) "key_down" else "key_up"),
        )
        if (code != null) {
            entries["code"] = JsonValue.Num(code.toDouble())
        } else {
            entries["name"] = JsonValue.Str(name.orEmpty())
        }
        scancode?.takeIf { it > 0 }?.let { entries["scancode"] = JsonValue.Num(it.toDouble()) }
        return JsonValue.Obj(entries)
    }
}

/** Text committed by the soft keyboard / IME (one `KEY_TYPED` per character). */
data class AwtTextEvent(val text: String) : AwtInputEvent {
    override fun toJson(): JsonValue = JsonValue.Obj(
        linkedMapOf(
            "type" to JsonValue.Str("text"),
            "text" to JsonValue.Str(text),
        ),
    )
}

/** The canvas gained or lost focus (losing it releases everything held). */
data class AwtFocusEvent(val gained: Boolean) : AwtInputEvent {
    override fun toJson(): JsonValue = JsonValue.Obj(
        linkedMapOf(
            "type" to JsonValue.Str("focus"),
            "gained" to JsonValue.Bool(gained),
        ),
    )
}

/** Release every held button / modifier (the app went to the background). */
data object AwtReleaseAllEvent : AwtInputEvent {
    override fun toJson(): JsonValue =
        JsonValue.Obj(linkedMapOf("type" to JsonValue.Str("release_all")))
}

/** Forget queued records and input state (bridge restart), keeping the pixels. */
data object AwtResetInputEvent : AwtInputEvent {
    override fun toJson(): JsonValue =
        JsonValue.Obj(linkedMapOf("type" to JsonValue.Str("reset_input")))
}

/** Encode a batch for `RustBridge.awtInput` (`{"events":[…]}`). */
fun List<AwtInputEvent>.toBatchJson(): String =
    JsonValue.Obj(linkedMapOf("events" to JsonValue.Arr(map { it.toJson() }))).toJsonString()

/** Encode a single event as a one-element batch. */
fun AwtInputEvent.toBatchJson(): String = listOf(this).toBatchJson()

/**
 * Canonical key names understood by the core's `vk_for_key`. Only the names a
 * phone can actually produce are listed; letters / digits / `fN` are computed by
 * the core, so `"w"` or `"f3"` work without an entry here.
 */
object AwtKeyNames {
    const val ESCAPE = "escape"
    const val ENTER = "enter"
    const val TAB = "tab"
    const val BACKSPACE = "backspace"
    const val DELETE = "delete"
    const val SPACE = "space"
    const val LEFT = "left"
    const val RIGHT = "right"
    const val UP = "up"
    const val DOWN = "down"
    const val HOME = "home"
    const val END = "end"
    const val PAGE_UP = "page.up"
    const val PAGE_DOWN = "page.down"
    const val SHIFT = "left.shift"
    const val CONTROL = "left.control"
    const val ALT = "left.alt"
    const val META = "left.super"

    /**
     * The name for a printable character, so a hardware keyboard can be forwarded
     * as a key press instead of as typed text (`'a'` → `"a"`, `'1'` → `"1"`).
     * Returns `null` for characters that need [AwtTextEvent] instead.
     */
    fun forChar(ch: Char): String? {
        val lower = ch.lowercaseChar()
        return when {
            lower in 'a'..'z' -> lower.toString()
            lower in '0'..'9' -> lower.toString()
            lower == ' ' -> SPACE
            else -> null
        }
    }
}

private fun num(value: Float): JsonValue =
    JsonValue.Num(if (value.isFinite()) value.toDouble() else 0.0)
