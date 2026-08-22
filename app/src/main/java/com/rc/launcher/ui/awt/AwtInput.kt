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
) : AwtInputEvent {
    override fun toJson(): JsonValue = JsonValue.Obj(
        linkedMapOf(
            "type" to JsonValue.Str("pointer"),
            "phase" to JsonValue.Str(phase.id),
            "x" to num(x),
            "y" to num(y),
            "button" to JsonValue.Str(button.id),
        ),
    )
}

/** A scroll gesture in surface pixels (`ticks > 0` scrolls away from the user). */
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
