package com.rc.launcher.ui.model

import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.parseJson
import com.rc.launcher.ui.model.json.toJsonString

/**
 * Controller / input-mapping model for the launcher (task 15).
 *
 * Benchmarks FCL-Controllers and Zalith Launcher control mapping: a layout is a
 * collection of on-screen [ControlElement]s (virtual buttons + touch joysticks)
 * bound to [MappedKey]s. The same [MappedKey] vocabulary covers keyboard keys,
 * mouse buttons and external gamepad buttons/axes, so a single layout can mix
 * touch, keyboard and gamepad input the way FCL/Zalith do.
 *
 * The whole model is plain Kotlin (no Android imports) so it is unit-testable
 * on the JVM and can be (de)serialized by [MiniJson] for persistence.
 */

/** A logical key/action a control can emit, mirroring Minecraft key bindings. */
enum class MappedKey(val code: String, val label: String) {
    // --- Keyboard (Minecraft lwjgl key codes) ---
    KEY_W("key.keyboard.w", "W"),
    KEY_A("key.keyboard.a", "A"),
    KEY_S("key.keyboard.s", "S"),
    KEY_D("key.keyboard.d", "D"),
    KEY_SPACE("key.keyboard.space", "空格"),
    KEY_SHIFT_LEFT("key.keyboard.shift.left", "L-Shift"),
    KEY_CTRL_LEFT("key.keyboard.control.left", "L-Ctrl"),
    KEY_ALT_LEFT("key.keyboard.alt.left", "L-Alt"),
    KEY_E("key.keyboard.e", "E"),
    KEY_Q("key.keyboard.q", "Q"),
    KEY_F("key.keyboard.f", "F"),
    KEY_R("key.keyboard.r", "R"),
    KEY_T("key.keyboard.t", "T"),
    KEY_TAB("key.keyboard.tab", "Tab"),
    KEY_ESC("key.keyboard.escape", "Esc"),
    KEY_SLASH("key.keyboard.slash", "/"),
    KEY_ENTER("key.keyboard.enter", "Enter"),
    KEY_1("key.keyboard.1", "1"),
    KEY_2("key.keyboard.2", "2"),
    KEY_3("key.keyboard.3", "3"),

    // --- Mouse ---
    MOUSE_LEFT("key.mouse.left", "鼠标左键"),
    MOUSE_RIGHT("key.mouse.right", "鼠标右键"),
    MOUSE_MIDDLE("key.mouse.middle", "鼠标中键"),

    // --- External gamepad (FCL/Zalith button vocabulary) ---
    BTN_A("button.a", "A"),
    BTN_B("button.b", "B"),
    BTN_X("button.x", "X"),
    BTN_Y("button.y", "Y"),
    BTN_LB("button.left.bumper", "LB"),
    BTN_RB("button.right.bumper", "RB"),
    BTN_LT("button.left.trigger", "LT"),
    BTN_RT("button.right.trigger", "RT"),
    BTN_LS("button.left.stick", "L3"),
    BTN_RS("button.right.stick", "R3"),
    BTN_START("button.start", "Start"),
    BTN_SELECT("button.select", "Select"),
    DPAD_UP("button.dpad.up", "↑"),
    DPAD_DOWN("button.dpad.down", "↓"),
    DPAD_LEFT("button.dpad.left", "←"),
    DPAD_RIGHT("button.dpad.right", "→");

    companion object {
        /** Resolve a [MappedKey] from its stable [code], or null if unknown. */
        fun fromCode(code: String?): MappedKey? = entries.firstOrNull { it.code == code }

        /** All keys grouped for the editor's key picker, in display order. */
        val ALL: List<MappedKey> = entries
    }
}

/** What a virtual joystick drives: character movement or camera look. */
enum class JoystickKind(val code: String, val label: String) {
    MOVE("move", "移动"),
    LOOK("look", "视角");

    companion object {
        fun fromName(name: String?): JoystickKind =
            entries.firstOrNull { it.name == name } ?: MOVE
    }
}

/** Common bound of every on-screen control. */
interface ControlElement {
    val id: String
    /** Normalised centre X in [0..1] of the (landscape) touch surface. */
    val x: Float
    /** Normalised centre Y in [0..1] of the (landscape) touch surface. */
    val y: Float
}

/** A tap-and-hold on-screen button bound to one or more [MappedKey]s. */
data class VirtualButton(
    override val id: String,
    override val x: Float,
    override val y: Float,
    val keys: List<MappedKey> = emptyList(),
    /** Optional override label; falls back to the first key's label. */
    val label: String = "",
    /** Normalised diameter as a fraction of the surface's min dimension. */
    val size: Float = DEFAULT_SIZE,
    /** ARGB colour used for the button fill. */
    val colorArgb: Int = DEFAULT_COLOR,
) : ControlElement {
    /** A copy with every field clamped to a valid, on-screen range. */
    fun normalized(): VirtualButton = copy(
        x = x.coerceIn(0f, 1f),
        y = y.coerceIn(0f, 1f),
        size = size.coerceIn(MIN_SIZE, MAX_SIZE),
        keys = keys.distinct(),
    )

    val displayLabel: String
        get() = label.ifBlank { keys.firstOrNull()?.label ?: id }

    companion object {
        const val DEFAULT_SIZE = 0.13f
        const val MIN_SIZE = 0.05f
        const val MAX_SIZE = 0.5f
        const val DEFAULT_COLOR = 0x66000000.toInt()
    }
}

/** A draggable touch joystick driving a movement or look axis pair. */
data class VirtualJoystick(
    override val id: String,
    override val x: Float,
    override val y: Float,
    /** Normalised radius as a fraction of the surface's min dimension. */
    val radius: Float = DEFAULT_RADIUS,
    val kind: JoystickKind = JoystickKind.MOVE,
) : ControlElement {
    fun normalized(): VirtualJoystick = copy(
        x = x.coerceIn(0f, 1f),
        y = y.coerceIn(0f, 1f),
        radius = radius.coerceIn(MIN_RADIUS, MAX_RADIUS),
    )

    companion object {
        const val DEFAULT_RADIUS = 0.18f
        const val MIN_RADIUS = 0.06f
        const val MAX_RADIUS = 0.6f
    }
}

/**
 * A complete, persistable control layout.
 *
 * [editable] is `false` for the built-in layouts shipped with the app; a user
 * can load one but saving always produces a new custom (editable) layout so the
 * built-ins stay intact (matches FCL/Zalith "save as" behaviour).
 */
data class ControlLayout(
    val id: String,
    val name: String,
    val elements: List<ControlElement> = emptyList(),
    val editable: Boolean = true,
    val createdAt: Long = 0L,
) {
    init {
        require(id.isNotBlank()) { "ControlLayout.id must not be blank" }
        require(name.isNotBlank()) { "ControlLayout.name must not be blank" }
    }

    /** Append [el], replacing any existing element with the same id. */
    fun withElement(el: ControlElement): ControlLayout =
        copy(elements = (elements.filter { it.id != el.id } + el))

    /** Remove the element with [id] (no-op if absent). */
    fun withoutElement(id: String): ControlLayout =
        copy(elements = elements.filter { it.id != id })

    /** True when an element with [id] exists. */
    fun hasElement(id: String): Boolean = elements.any { it.id == id }

    /** A copy with every element clamped into a valid, on-screen range. */
    fun sanitized(): ControlLayout = copy(
        elements = elements.map {
            when (it) {
                is VirtualButton -> it.normalized()
                is VirtualJoystick -> it.normalized()
                else -> it
            }
        },
    )

    /** Compact metadata used by pickers and the repository index. */
    fun meta(): ControlLayoutMeta = ControlLayoutMeta(id, name, !editable)

    companion object {
        const val DEFAULT_ID = "default"
        const val WASD_ID = "wasd"
        const val GAMEPAD_ID = "gamepad"
    }
}

/** Lightweight summary of a layout, safe to enumerate without loading it. */
data class ControlLayoutMeta(
    val id: String,
    val name: String,
    val builtIn: Boolean,
)

// ============================================================================
// Built-in layout catalogue (benchmark: FCL-Controllers / Zalith)
// ============================================================================

object ControlLayoutCatalog {
    /** Touch layout: two joysticks + the most-used action buttons. */
    fun default(): ControlLayout = ControlLayout(
        id = ControlLayout.DEFAULT_ID,
        name = "默认布局（触控）",
        editable = false,
        elements = listOf(
            VirtualJoystick("stick_move", 0.18f, 0.74f, 0.19f, JoystickKind.MOVE),
            VirtualJoystick("stick_look", 0.82f, 0.74f, 0.17f, JoystickKind.LOOK),
            VirtualButton("jump", 0.86f, 0.54f, listOf(MappedKey.KEY_SPACE), "跳"),
            VirtualButton("sneak", 0.70f, 0.86f, listOf(MappedKey.KEY_SHIFT_LEFT), "潜"),
            VirtualButton("sprint", 0.60f, 0.92f, listOf(MappedKey.KEY_CTRL_LEFT), "跑"),
            VirtualButton("use", 0.66f, 0.56f, listOf(MappedKey.MOUSE_LEFT), "打"),
            VirtualButton("place", 0.76f, 0.62f, listOf(MappedKey.MOUSE_RIGHT), "放"),
            VirtualButton("inv", 0.94f, 0.30f, listOf(MappedKey.KEY_E), "背包"),
            VirtualButton("drop", 0.88f, 0.38f, listOf(MappedKey.KEY_Q), "丢"),
            VirtualButton("chat", 0.50f, 0.08f, listOf(MappedKey.KEY_T), "聊天"),
            VirtualButton("pause", 0.06f, 0.08f, listOf(MappedKey.KEY_ESC), "菜单"),
        ),
    )

    /** Keyboard + mouse layout (no joysticks). */
    fun wasd(): ControlLayout = ControlLayout(
        id = ControlLayout.WASD_ID,
        name = "WASD + 鼠标",
        editable = false,
        elements = listOf(
            VirtualButton("k_w", 0.30f, 0.62f, listOf(MappedKey.KEY_W), "W"),
            VirtualButton("k_a", 0.22f, 0.74f, listOf(MappedKey.KEY_A), "A"),
            VirtualButton("k_s", 0.30f, 0.74f, listOf(MappedKey.KEY_S), "S"),
            VirtualButton("k_d", 0.38f, 0.74f, listOf(MappedKey.KEY_D), "D"),
            VirtualButton("k_space", 0.30f, 0.88f, listOf(MappedKey.KEY_SPACE), "跳"),
            VirtualButton("k_shift", 0.22f, 0.88f, listOf(MappedKey.KEY_SHIFT_LEFT), "潜"),
            VirtualButton("k_ctrl", 0.38f, 0.88f, listOf(MappedKey.KEY_CTRL_LEFT), "跑"),
            VirtualButton("k_e", 0.86f, 0.30f, listOf(MappedKey.KEY_E), "背包"),
            VirtualButton("k_q", 0.80f, 0.30f, listOf(MappedKey.KEY_Q), "丢"),
            VirtualButton("k_t", 0.50f, 0.08f, listOf(MappedKey.KEY_T), "聊天"),
            VirtualButton("k_f", 0.80f, 0.40f, listOf(MappedKey.KEY_F), "换"),
            VirtualButton("k_esc", 0.06f, 0.08f, listOf(MappedKey.KEY_ESC), "菜单"),
            VirtualButton("m_left", 0.74f, 0.62f, listOf(MappedKey.MOUSE_LEFT), "左键"),
            VirtualButton("m_right", 0.86f, 0.62f, listOf(MappedKey.MOUSE_RIGHT), "右键"),
        ),
    )

    /** External gamepad layout (mirrors a physical controller). */
    fun gamepad(): ControlLayout = ControlLayout(
        id = ControlLayout.GAMEPAD_ID,
        name = "手柄布局",
        editable = false,
        elements = listOf(
            VirtualJoystick("gp_move", 0.20f, 0.74f, 0.20f, JoystickKind.MOVE),
            VirtualJoystick("gp_look", 0.80f, 0.74f, 0.18f, JoystickKind.LOOK),
            VirtualButton("gp_a", 0.86f, 0.62f, listOf(MappedKey.BTN_A), "A"),
            VirtualButton("gp_b", 0.80f, 0.70f, listOf(MappedKey.BTN_B), "B"),
            VirtualButton("gp_x", 0.80f, 0.54f, listOf(MappedKey.BTN_X), "X"),
            VirtualButton("gp_y", 0.74f, 0.62f, listOf(MappedKey.BTN_Y), "Y"),
            VirtualButton("gp_lb", 0.62f, 0.50f, listOf(MappedKey.BTN_LB), "LB"),
            VirtualButton("gp_rb", 0.88f, 0.50f, listOf(MappedKey.BTN_RB), "RB"),
            VirtualButton("gp_lt", 0.56f, 0.44f, listOf(MappedKey.BTN_LT), "LT"),
            VirtualButton("gp_rt", 0.94f, 0.44f, listOf(MappedKey.BTN_RT), "RT"),
            VirtualButton("gp_start", 0.60f, 0.30f, listOf(MappedKey.BTN_START), "Start"),
            VirtualButton("gp_select", 0.40f, 0.30f, listOf(MappedKey.BTN_SELECT), "Sel"),
            VirtualButton("gp_dpad_u", 0.50f, 0.50f, listOf(MappedKey.DPAD_UP), "↑"),
            VirtualButton("gp_dpad_d", 0.50f, 0.62f, listOf(MappedKey.DPAD_DOWN), "↓"),
            VirtualButton("gp_dpad_l", 0.44f, 0.56f, listOf(MappedKey.DPAD_LEFT), "←"),
            VirtualButton("gp_dpad_r", 0.56f, 0.56f, listOf(MappedKey.DPAD_RIGHT), "→"),
        ),
    )

    /** All built-in layouts. */
    fun all(): List<ControlLayout> = listOf(default(), wasd(), gamepad())

    /** Metadata for every built-in layout. */
    fun allMetas(): List<ControlLayoutMeta> = all().map { it.meta() }

    /** Look up a built-in layout by id, or null if it is not built-in. */
    fun builtInById(id: String): ControlLayout? = all().firstOrNull { it.id == id }

    /** The layout that should be shown first / on reset. */
    fun defaultLayout(): ControlLayout = default()
}

// ============================================================================
// JSON (de)serialization via MiniJson
// ============================================================================

private fun num(v: Float): JsonValue = JsonValue.Num(v.toDouble())
private fun num(v: Long): JsonValue = JsonValue.Num(v.toDouble())

/** Convert this layout into a [JsonValue] (for [MiniJson] serialization). */
fun ControlLayout.toJsonValue(): JsonValue {
    val elementValues = elements.map { el ->
        when (el) {
            is VirtualButton -> JsonValue.Obj(
                mapOf(
                    "t" to JsonValue.Str("b"),
                    "id" to JsonValue.Str(el.id),
                    "x" to num(el.x),
                    "y" to num(el.y),
                    "s" to num(el.size),
                    "c" to num(el.colorArgb.toLong() and 0xFFFFFFFFL),
                    "l" to JsonValue.Str(el.label),
                    "k" to JsonValue.Arr(el.keys.map { JsonValue.Str(it.code) }),
                ),
            )
            is VirtualJoystick -> JsonValue.Obj(
                mapOf(
                    "t" to JsonValue.Str("j"),
                    "id" to JsonValue.Str(el.id),
                    "x" to num(el.x),
                    "y" to num(el.y),
                    "r" to num(el.radius),
                    "k" to JsonValue.Str(el.kind.name),
                ),
            )
            else -> JsonValue.Null
        }
    }
    return JsonValue.Obj(
        mapOf(
            "id" to JsonValue.Str(id),
            "name" to JsonValue.Str(name),
            "e" to JsonValue.Arr(elementValues),
            "ed" to JsonValue.Bool(editable),
            "ct" to num(createdAt),
        ),
    )
}

private fun JsonValue.Obj.str(key: String): String? = (entries[key] as? JsonValue.Str)?.value
private fun JsonValue.Obj.dbl(key: String): Double? = (entries[key] as? JsonValue.Num)?.value
private fun JsonValue.Obj.bool(key: String): Boolean? = (entries[key] as? JsonValue.Bool)?.value
private fun JsonValue.Obj.arr(key: String): List<JsonValue>? = (entries[key] as? JsonValue.Arr)?.items

private fun JsonValue.toElement(): ControlElement? {
    if (this !is JsonValue.Obj) return null
    val id = str("id") ?: return null
    val x = dbl("x")?.toFloat() ?: return null
    val y = dbl("y")?.toFloat() ?: return null
    return when (str("t")) {
        "b" -> {
            val keys = (arr("k") ?: emptyList())
                .mapNotNull { (it as? JsonValue.Str)?.value }
                .mapNotNull { MappedKey.fromCode(it) }
            VirtualButton(
                id = id,
                x = x,
                y = y,
                keys = keys,
                label = str("l") ?: "",
                size = (dbl("s") ?: VirtualButton.DEFAULT_SIZE.toDouble()).toFloat(),
                colorArgb = (dbl("c") ?: VirtualButton.DEFAULT_COLOR.toDouble()).toLong().toInt(),
            )
        }
        "j" -> VirtualJoystick(
            id = id,
            x = x,
            y = y,
            radius = (dbl("r") ?: VirtualJoystick.DEFAULT_RADIUS.toDouble()).toFloat(),
            kind = JoystickKind.fromName(str("k")),
        )
        else -> null
    }
}

/** Parse a [ControlLayout] from JSON text, or null if [text] is malformed. */
fun parseControlLayout(text: String): ControlLayout? {
    val root = parseJson(text) ?: return null
    if (root !is JsonValue.Obj) return null
    val id = root.str("id") ?: return null
    val name = root.str("name") ?: return null
    if (id.isBlank() || name.isBlank()) return null
    val elements = (root.arr("e") ?: emptyList()).mapNotNull { it.toElement() }
    return ControlLayout(
        id = id,
        name = name,
        elements = elements,
        editable = root.bool("ed") ?: true,
        createdAt = root.dbl("ct")?.toLong() ?: 0L,
    ).sanitized()
}

/** Serialize this layout to a compact JSON string. */
fun ControlLayout.toJsonString(): String = toJsonValue().toJsonString()

