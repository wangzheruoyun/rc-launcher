package com.rc.launcher.ui.awt

import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.toJsonString

/*
 * Physical keyboard & mouse, Kotlin side (task 12).
 *
 * Players asked for one thing: "let me use a keyboard and a mouse". On Android
 * that is four separate problems, and this file is the UI-side half of each:
 *
 * | problem | here |
 * |---|---|
 * | a mouse reports *motion*, not a position, once captured | [AwtMouseTracker] |
 * | left / right / middle must be told apart | [AwtMouseTracker.onButtonState] |
 * | the player wants it faster / slower / inverted | [AwtMouseSensitivity] |
 * | the player wants other keys | [AwtKeyBindings] |
 *
 * The *stateful* translation lives in the Rust core (`launch::input`), which owns
 * the sub-pixel remainder, the held-key set, the GLFW modifier bits and the
 * free-running captured cursor. This file only has to describe what the hardware
 * did, plus the settings the core should apply — which keeps the JNI boundary at
 * one call per frame and makes every line here unit-testable on the JVM.
 *
 * Mirrors `launch::input::{PointerMode, MouseSensitivity, InputBindings,
 * InputSettings}`; the ids, JSON keys and bounds are compared against the Rust
 * source by `scripts/check_awt_wire.py`, because a drift here is invisible until
 * a player's mouse does nothing.
 *
 * Pure Kotlin (no Android imports): unit-testable on the JVM.
 */

/** How a physical pointer drives the game (mirrors `PointerMode`). */
enum class AwtPointerMode(val id: String, val label: String) {
    /** Cursor visible, absolute position — menus, the inventory, a Forge dialog. */
    ABSOLUTE("absolute", "指针模式"),

    /** Pointer captured, relative motion only — looking around, like the desktop. */
    CAPTURED("captured", "捕获模式"),
    ;

    /** Whether the game owns the cursor in this mode. */
    val isCaptured: Boolean get() = this == CAPTURED

    /** The other mode (what a toggle switches to). */
    fun toggled(): AwtPointerMode = if (isCaptured) ABSOLUTE else CAPTURED

    companion object {
        /** Parse an id; anything unknown keeps [ABSOLUTE] (the safe default). */
        fun fromId(id: String?): AwtPointerMode =
            AwtPointerMode.entries.firstOrNull { it.id == id } ?: ABSOLUTE
    }
}

/**
 * Pointer sensitivity, in **per-mille** — the same integer representation the
 * core uses, so the value the UI shows and the value the core applies can never
 * disagree by a rounding step.
 */
data class AwtMouseSensitivity(
    val xPermille: Int = DEFAULT_PERMILLE,
    val yPermille: Int = DEFAULT_PERMILLE,
    val invertY: Boolean = false,
) {
    /** Horizontal factor (1.0 = 1:1). */
    val x: Float get() = xPermille / 1000f

    /** Vertical factor. */
    val y: Float get() = yPermille / 1000f

    /** Whether both axes share one factor (what the simple slider edits). */
    val isUniform: Boolean get() = xPermille == yPermille

    /** Clamp both axes into the range the core accepts. */
    fun sanitized(): AwtMouseSensitivity = copy(
        xPermille = xPermille.coerceIn(MIN_PERMILLE, MAX_PERMILLE),
        yPermille = yPermille.coerceIn(MIN_PERMILLE, MAX_PERMILLE),
    )

    /** The same value with both axes set from one slider position. */
    fun withUniform(factor: Float): AwtMouseSensitivity =
        copy(xPermille = quantise(factor), yPermille = quantise(factor)).sanitized()

    /** Label for the settings row (`"1.5×"`, `"0.4× · 反转"`). */
    fun label(): String {
        val base = if (isUniform) {
            format(x)
        } else {
            "${format(x)} / ${format(y)}"
        }
        return if (invertY) "$base · 反转Y" else base
    }

    /** JSON form the core merges (`{"x":1.5,"y":1.5,"invert_y":false}`). */
    fun toJson(): JsonValue = JsonValue.Obj(
        linkedMapOf(
            "x_permille" to JsonValue.Num(xPermille.toDouble()),
            "y_permille" to JsonValue.Num(yPermille.toDouble()),
            "invert_y" to JsonValue.Bool(invertY),
        ),
    )

    /** `1.5f` -> `"1.5×"`, `2f` -> `"2×"` (two decimals, trailing zeros dropped). */
    private fun format(value: Float): String {
        val hundredths = Math.round(value * 100f)
        val whole = hundredths / 100
        val fraction = hundredths % 100
        if (fraction == 0) return "$whole×"
        val digits = fraction.toString().padStart(2, '0').trimEnd('0')
        return "$whole.$digits×"
    }

    companion object {
        /** Slowest factor the core accepts (0.1×). */
        const val MIN_PERMILLE = 100

        /** Fastest factor the core accepts (10×). */
        const val MAX_PERMILLE = 10_000

        /** 1:1. */
        const val DEFAULT_PERMILLE = 1_000

        /** 1:1 on both axes. */
        val DEFAULT = AwtMouseSensitivity()

        /** Quantise a float factor into per-mille, clamped and NaN-safe. */
        fun quantise(factor: Float): Int {
            if (!factor.isFinite()) return DEFAULT_PERMILLE
            return Math.round(factor * 1000f).coerceIn(MIN_PERMILLE, MAX_PERMILLE)
        }

        /** Both axes from one factor. */
        fun uniform(factor: Float): AwtMouseSensitivity =
            AwtMouseSensitivity(quantise(factor), quantise(factor))

        /** Parse the core's snapshot; missing members keep the default. */
        fun parse(obj: JsonValue.Obj?): AwtMouseSensitivity {
            if (obj == null) return DEFAULT
            val x = obj.int("x_permille") ?: obj.double("x")?.let { quantise(it.toFloat()) }
            val y = obj.int("y_permille") ?: obj.double("y")?.let { quantise(it.toFloat()) }
            return AwtMouseSensitivity(
                xPermille = x ?: DEFAULT_PERMILLE,
                yPermille = y ?: DEFAULT_PERMILLE,
                invertY = obj.bool("invert_y") ?: false,
            ).sanitized()
        }
    }
}

/**
 * Key / button remapping (mirrors `InputBindings`).
 *
 * **Single hop by design**, exactly like the core: `a → b` and `b → c` never turn
 * `a` into `c`. A remap is "this key now does that", not a rewriting system, and
 * a cycle therefore cannot exist.
 */
data class AwtKeyBindings(
    /** Source key name → target key name, both in the task-15 vocabulary. */
    val keys: Map<String, String> = emptyMap(),
    /** Source button → target button. */
    val buttons: Map<AwtMouseButton, AwtMouseButton> = emptyMap(),
) {
    /** Whether nothing is remapped. */
    val isEmpty: Boolean get() = keys.isEmpty() && buttons.isEmpty()

    /** Number of remaps, for the settings row summary. */
    val size: Int get() = keys.size + buttons.size

    /** The name a key is bound to (itself when unbound). */
    fun resolveKey(name: String): String = keys[normalise(name)] ?: normalise(name)

    /** The button a button is bound to (itself when unbound). */
    fun resolveButton(button: AwtMouseButton): AwtMouseButton = buttons[button] ?: button

    /** Add / replace a key remap; binding a key to itself removes it. */
    fun withKey(from: String, to: String): AwtKeyBindings {
        val source = normalise(from)
        val target = normalise(to)
        if (source.isEmpty() || target.isEmpty()) return this
        return if (source == target) {
            copy(keys = keys - source)
        } else {
            copy(keys = keys + (source to target))
        }
    }

    /** Drop one key remap. */
    fun withoutKey(from: String): AwtKeyBindings = copy(keys = keys - normalise(from))

    /** Add / replace a button remap; binding a button to itself removes it. */
    fun withButton(from: AwtMouseButton, to: AwtMouseButton): AwtKeyBindings =
        if (from == to) copy(buttons = buttons - from) else copy(buttons = buttons + (from to to))

    /** JSON form: `{"keys":{"e":"f"},"buttons":{"1":3}}`. */
    fun toJson(): JsonValue = JsonValue.Obj(
        linkedMapOf(
            // Sorted, so the same table always serialises to the same bytes (the
            // settings backup is diffed by humans).
            "keys" to JsonValue.Obj(keys.toSortedMap().mapValues { JsonValue.Str(it.value) }),
            "buttons" to JsonValue.Obj(
                buttons.entries
                    .sortedBy { it.key.number }
                    .associate {
                        it.key.number.toString() to JsonValue.Num(it.value.number.toDouble())
                    },
            ),
        ),
    )

    companion object {
        /** Nothing remapped. */
        val EMPTY = AwtKeyBindings()

        /** The same normalisation the core applies, so both sides agree on keys. */
        fun normalise(name: String): String = name.trim()
            .lowercase()
            .replace('_', '.')
            .replace(' ', '.')
            .removePrefix("key.keyboard.")

        /** Parse the core's snapshot; malformed entries are skipped, never fatal. */
        fun parse(obj: JsonValue.Obj?): AwtKeyBindings {
            if (obj == null) return EMPTY
            val keys = LinkedHashMap<String, String>()
            (obj.entries["keys"] as? JsonValue.Obj)?.entries?.forEach { (from, to) ->
                val target = (to as? JsonValue.Str)?.value ?: return@forEach
                if (from.isNotBlank() && target.isNotBlank()) keys[normalise(from)] = normalise(target)
            }
            val buttons = LinkedHashMap<AwtMouseButton, AwtMouseButton>()
            (obj.entries["buttons"] as? JsonValue.Obj)?.entries?.forEach { (from, to) ->
                val source = from.toIntOrNull()?.let { n -> AwtMouseButton.entries.firstOrNull { it.number == n } }
                val target = (to as? JsonValue.Num)?.value?.toInt()
                    ?.let { n -> AwtMouseButton.entries.firstOrNull { it.number == n } }
                if (source != null && target != null && source != target) buttons[source] = target
            }
            return AwtKeyBindings(keys, buttons)
        }
    }
}

/**
 * Everything the player can tune about physical input (mirrors `InputSettings`).
 *
 * Sent to the core as one `awtConfigure` member, and read back from every session
 * snapshot, so the UI never keeps a second source of truth.
 */
data class AwtInputSettings(
    val pointerMode: AwtPointerMode = AwtPointerMode.ABSOLUTE,
    /** Accept touch while the pointer is captured (mixed touch + mouse). */
    val hybridTouch: Boolean = true,
    /** Feed the game's own (`CallbackBridge`) queue, not only AWT. */
    val nativeInput: Boolean = true,
    val sensitivity: AwtMouseSensitivity = AwtMouseSensitivity.DEFAULT,
    /** Wheel scaling in per-mille (1000 = one notch per notch). */
    val scrollPermille: Int = DEFAULT_SCROLL_PERMILLE,
    val bindings: AwtKeyBindings = AwtKeyBindings.EMPTY,
) {
    /** Whether the pointer is captured right now. */
    val captured: Boolean get() = pointerMode.isCaptured

    /** Clamp everything into the range the core accepts. */
    fun sanitized(): AwtInputSettings = copy(
        sensitivity = sensitivity.sanitized(),
        scrollPermille = scrollPermille.coerceIn(MIN_SCROLL_PERMILLE, MAX_SCROLL_PERMILLE),
    )

    /** Whether a sample from [source] may move the pointer right now. */
    fun accepts(source: AwtPointerSource): Boolean =
        source.isMouseLike || !captured || hybridTouch

    /** JSON for the `input` member of `awtConfigure` / `awtOpen`. */
    fun toJson(): JsonValue = JsonValue.Obj(
        linkedMapOf(
            "pointer_mode" to JsonValue.Str(pointerMode.id),
            "hybrid_touch" to JsonValue.Bool(hybridTouch),
            "native_input" to JsonValue.Bool(nativeInput),
            "sensitivity" to sensitivity.toJson(),
            "scroll_permille" to JsonValue.Num(scrollPermille.toDouble()),
            "bindings" to bindings.toJson(),
        ),
    )

    /** The same document as a string (persistence / diagnostics). */
    fun toJsonString(): String = toJson().toJsonString()

    companion object {
        /** 1000 = one wheel notch per reported notch. */
        const val DEFAULT_SCROLL_PERMILLE = 1_000

        /** Slowest / fastest wheel scaling the core accepts. */
        const val MIN_SCROLL_PERMILLE = 100
        const val MAX_SCROLL_PERMILLE = 10_000

        /** Defaults: absolute pointer, mixed touch, native path on, 1:1. */
        val DEFAULT = AwtInputSettings()

        /** Parse the `input` member of a session snapshot. */
        fun parse(obj: JsonValue.Obj?): AwtInputSettings {
            if (obj == null) return DEFAULT
            return AwtInputSettings(
                pointerMode = AwtPointerMode.fromId(obj.str("pointer_mode")),
                hybridTouch = obj.bool("hybrid_touch") ?: true,
                nativeInput = obj.bool("native_input") ?: true,
                sensitivity = AwtMouseSensitivity.parse(obj.obj("sensitivity")),
                scrollPermille = obj.int("scroll_permille") ?: DEFAULT_SCROLL_PERMILLE,
                bindings = AwtKeyBindings.parse(obj.obj("bindings")),
            ).sanitized()
        }
    }
}

/** What the game's own input queue currently believes (diagnostics). */
data class AwtGameInputState(
    val cursorX: Int = 0,
    val cursorY: Int = 0,
    val grabbed: Boolean = false,
    val heldKeys: Int = 0,
    val heldButtons: Int = 0,
    val frameWidth: Int = 0,
    val frameHeight: Int = 0,
) {
    /** One-line summary for the diagnostics card. */
    fun describe(): String = buildString {
        append(if (grabbed) "已捕获" else "未捕获")
        append(" · 指针 (").append(cursorX).append(", ").append(cursorY).append(")")
        if (heldKeys > 0) append(" · 按住 ").append(heldKeys).append(" 键")
        if (heldButtons > 0) append(" · ").append(heldButtons).append(" 键位")
    }

    companion object {
        val EMPTY = AwtGameInputState()

        fun parse(obj: JsonValue.Obj?): AwtGameInputState {
            if (obj == null) return EMPTY
            val cursor = obj.obj("cursor")
            val frame = obj.obj("framebuffer")
            return AwtGameInputState(
                cursorX = cursor?.int("x") ?: 0,
                cursorY = cursor?.int("y") ?: 0,
                grabbed = obj.bool("grabbed") ?: false,
                heldKeys = obj.int("held_keys") ?: 0,
                heldButtons = obj.int("held_buttons") ?: 0,
                frameWidth = frame?.int("width") ?: 0,
                frameHeight = frame?.int("height") ?: 0,
            )
        }
    }
}

/**
 * Turns raw Android pointer facts into [AwtInputEvent]s (task 12).
 *
 * Android hands the UI a *state*, not a transition: `MotionEvent.getButtonState()`
 * is a bitmask of what is held right now, and there is no "button 2 went down"
 * event. Minecraft (and Swing) need the transition, so somebody has to diff
 * consecutive states — and doing it here, in one small pure state machine, is
 * what keeps `AwtCanvasSurface` free of input bookkeeping and lets the whole
 * behaviour be tested without an Android device.
 *
 * It also implements the *hybrid* rule: while the pointer is captured, a finger
 * is ignored unless the player asked for the mixed mode, so a palm resting on the
 * screen cannot spin the camera.
 */
class AwtMouseTracker(settings: AwtInputSettings = AwtInputSettings.DEFAULT) {
    /** The settings in force (kept in sync with the core's snapshot). */
    var settings: AwtInputSettings = settings.sanitized()
        private set

    private var held: Set<AwtMouseButton> = emptySet()
    private var lastX: Float = 0f
    private var lastY: Float = 0f
    private var inside: Boolean = false

    /** Buttons currently held, in AWT numbering. */
    val heldButtons: Set<AwtMouseButton> get() = held

    /** Whether any button is held (a move is then a drag). */
    val isDragging: Boolean get() = held.isNotEmpty()

    /** Replace the settings; a mode change drops the in-flight gesture. */
    fun applySettings(settings: AwtInputSettings): List<AwtInputEvent> {
        val previous = this.settings
        this.settings = settings.sanitized()
        return if (previous.pointerMode != this.settings.pointerMode) releaseHeld() else emptyList()
    }

    /**
     * A pointer sample at a surface position, with the button bitmask Android
     * reported. Returns the events to send, in order.
     *
     * [buttonState] uses the `MotionEvent.BUTTON_*` bits; pass `0` for a touch
     * (which the caller then reports as [touchButton]).
     */
    fun onPointer(
        x: Float,
        y: Float,
        buttonState: Int,
        source: AwtPointerSource,
        pressed: Boolean,
        touchButton: AwtMouseButton = AwtMouseButton.LEFT,
    ): List<AwtInputEvent> {
        if (!settings.accepts(source)) return emptyList()
        val wanted: Set<AwtMouseButton> = when {
            source.isMouseLike -> awtMouseButtonsForButtonState(buttonState)
            pressed -> setOf(touchButton)
            else -> emptySet()
        }
        val out = ArrayList<AwtInputEvent>(4)
        val moved = !inside || x != lastX || y != lastY
        // A press must be reported *at* its position, so move first when the
        // pointer travelled (a hover that ends in a click, a tap somewhere new).
        if (moved && (wanted.isNotEmpty() || held.isNotEmpty() || source.isMouseLike)) {
            out += AwtPointerEvent(AwtPointerPhase.MOVE, x, y, held.firstOrNull() ?: touchButton, source)
        }
        for (button in wanted - held) {
            out += AwtPointerEvent(AwtPointerPhase.DOWN, x, y, button, source)
        }
        for (button in held - wanted) {
            out += AwtPointerEvent(AwtPointerPhase.UP, x, y, button, source)
        }
        held = wanted
        lastX = x
        lastY = y
        inside = true
        return out
    }

    /** Relative motion from a captured mouse (or a trackpad in relative mode). */
    fun onRelative(
        dx: Float,
        dy: Float,
        source: AwtPointerSource = AwtPointerSource.MOUSE,
    ): List<AwtInputEvent> {
        if (!settings.accepts(source)) return emptyList()
        if (dx == 0f && dy == 0f) return emptyList()
        if (!dx.isFinite() || !dy.isFinite()) return emptyList()
        return listOf(AwtRelativePointerEvent(dx, dy, source))
    }

    /**
     * The button bitmask changed without the pointer moving (a click on a
     * stationary mouse): the same diff, at the last known position.
     */
    fun onButtonState(
        buttonState: Int,
        source: AwtPointerSource = AwtPointerSource.MOUSE,
    ): List<AwtInputEvent> = onPointer(lastX, lastY, buttonState, source, pressed = false)

    /**
     * The button bitmask of a **captured** mouse.
     *
     * The same diff as [onButtonState], but the result carries no coordinates: a
     * captured pointer has no surface position (that is what capturing means), so
     * the click has to happen wherever the core's virtual pointer currently is —
     * hence [AwtButtonEvent] instead of [AwtPointerEvent].
     */
    fun onCapturedButtonState(buttonState: Int): List<AwtInputEvent> {
        val wanted = awtMouseButtonsForButtonState(buttonState)
        if (wanted == held) return emptyList()
        val out = ArrayList<AwtInputEvent>(2)
        for (button in wanted - held) out += AwtButtonEvent(button, down = true)
        for (button in held - wanted) out += AwtButtonEvent(button, down = false)
        held = wanted
        return out
    }

    /** Capture / release the pointer, releasing whatever the old mode held. */
    fun setCaptured(captured: Boolean): List<AwtInputEvent> {
        if (settings.captured == captured) return emptyList()
        settings = settings.copy(
            pointerMode = if (captured) AwtPointerMode.CAPTURED else AwtPointerMode.ABSOLUTE,
        )
        return releaseHeld() + AwtCaptureEvent(captured)
    }

    /**
     * Forget every held button (focus loss, composition disposal, rotation).
     *
     * A captured pointer releases through [AwtButtonEvent] for the same reason it
     * presses through it: there is no position to report.
     */
    fun releaseHeld(): List<AwtInputEvent> {
        if (held.isEmpty()) {
            inside = false
            return emptyList()
        }
        val captured = settings.captured
        val out: List<AwtInputEvent> = held.map { button ->
            if (captured) {
                AwtButtonEvent(button, down = false)
            } else {
                AwtPointerEvent(AwtPointerPhase.UP, lastX, lastY, button, AwtPointerSource.MOUSE)
            }
        }
        held = emptySet()
        inside = false
        return out
    }

    /** Drop all state without emitting anything (the session was replaced). */
    fun reset() {
        held = emptySet()
        lastX = 0f
        lastY = 0f
        inside = false
    }
}
