package com.rc.launcher.ui.model

import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.parseJson
import com.rc.launcher.ui.model.json.toJsonString

/**
 * Built-in physical gamepad mapping database + plug-and-play identification +
 * input calibration (task 4).
 *
 * FCL ships `assets/controllers/<id>.json` *touch* layouts but no physical-pad
 * recognition; RC already has the control-layout UI ([ControlLayoutCatalog]) yet
 * ships zero device mapping resources. This object closes that gap with an
 * offline database of mainstream controllers (Xbox, PlayStation, 8BitDo,
 * Flydigi/Feitian and the generic Android HID gamepad) keyed by USB
 * vendor/product id, plug-and-play identification, user custom remapping, and
 * the dead-zone / sensitivity calibration the input layer applies to analog
 * axes and sticks.
 *
 * The data mirrors the Rust core's `gamepad` module (exposed via
 * [com.rc.launcher.core.RustBridge]) so the UI works without loading the native
 * library; the Rust side owns the authoritative copy and is the source of truth
 * for runtime behaviour.
 *
 * Everything here is plain Kotlin (no Android imports) so it is unit-testable on
 * the JVM, matching the control-layout model split.
 */

/** A canonical gamepad button, independent of the physical device. */
enum class StandardButton(val code: String, val label: String) {
    SOUTH("south", "A / ✕"),
    EAST("east", "B / ○"),
    NORTH("north", "X / □"),
    WEST("west", "Y / △"),
    LEFT_BUMPER("left_bumper", "LB"),
    RIGHT_BUMPER("right_bumper", "RB"),
    LEFT_TRIGGER("left_trigger", "LT"),
    RIGHT_TRIGGER("right_trigger", "RT"),
    LEFT_STICK("left_stick", "L3"),
    RIGHT_STICK("right_stick", "R3"),
    START("start", "Start"),
    SELECT("select", "Select"),
    GUIDE("guide", "Home"),
    DPAD_UP("dpad_up", "↑"),
    DPAD_DOWN("dpad_down", "↓"),
    DPAD_LEFT("dpad_left", "←"),
    DPAD_RIGHT("dpad_right", "→");

    companion object {
        /** Resolve a [StandardButton] from its stable [code], or null. */
        fun fromCode(code: String?): StandardButton? = entries.firstOrNull { it.code == code }
    }
}

/** A canonical gamepad analog axis. */
enum class StandardAxis(val code: String, val label: String) {
    LEFT_X("left_x", "左摇杆 X"),
    LEFT_Y("left_y", "左摇杆 Y"),
    RIGHT_X("right_x", "右摇杆 X"),
    RIGHT_Y("right_y", "右摇杆 Y"),
    TRIGGER_LEFT("trigger_left", "LT"),
    TRIGGER_RIGHT("trigger_right", "RT");

    companion object {
        /** Resolve a [StandardAxis] from its stable [code], or null. */
        fun fromCode(code: String?): StandardAxis? = entries.firstOrNull { it.code == code }
    }
}

/** A built-in controller profile, keyed by USB vendor/product id. */
data class ControllerProfile(
    val id: String,
    val name: String,
    val vendorId: Int,
    val productId: Int,
    val description: String,
    val defaultDeadzone: Float = 0.15f,
    val defaultSensitivity: Float = 1.0f,
) {
    /** Whether this is the generic fallback profile. */
    val isGeneric: Boolean get() = id == GENERIC_ID

    companion object {
        const val GENERIC_ID = "generic"
    }
}

/** Built-in controller database (in display / priority order). */
object GamepadDatabase {
    val GENERIC = ControllerProfile(
        id = ControllerProfile.GENERIC_ID,
        name = "通用手柄",
        vendorId = 0x0000,
        productId = 0x0000,
        description = "标准 Android HID 手柄（未知设备回退）",
        defaultDeadzone = 0.15f,
    )
    val XBOX = ControllerProfile(
        id = "xbox",
        name = "Xbox 手柄",
        vendorId = 0x045E,
        productId = 0x02EA,
        description = "Microsoft Xbox One / Series 手柄",
        defaultDeadzone = 0.15f,
    )
    val PLAYSTATION = ControllerProfile(
        id = "playstation",
        name = "PlayStation 手柄",
        vendorId = 0x054C,
        productId = 0x09CC,
        description = "索尼 DualShock 4 / DualSense（扳机走 BRAKE/GAS）",
        defaultDeadzone = 0.12f,
    )
    val EIGHTBITDO = ControllerProfile(
        id = "8bitdo",
        name = "8BitDo 手柄",
        vendorId = 0x2DC8,
        productId = 0x6001,
        description = "8BitDo SN30 Pro / Pro 2 / Ultimate",
        defaultDeadzone = 0.18f,
    )
    val FLYDIGI = ControllerProfile(
        id = "flydigi",
        name = "飞智手柄",
        vendorId = 0x294B,
        productId = 0x1903,
        description = "飞智游戏手柄（右摇杆走 RX/RY，扳机走 BRAKE/GAS）",
        defaultSensitivity = 1.05f,
        defaultDeadzone = 0.16f,
    )

    /** Every built-in profile. */
    val all: List<ControllerProfile> = listOf(GENERIC, XBOX, PLAYSTATION, EIGHTBITDO, FLYDIGI)

    /** The generic fallback profile. */
    val default: ControllerProfile = GENERIC

    /**
     * Plug-and-play identification by USB vendor/product id.
     *
     * Exact `(vendorId, productId)` match wins; otherwise any profile sharing
     * the vendor id is returned (vendor-level quirk coverage); otherwise the
     * generic fallback is used so an unknown pad still gets a sane mapping.
     */
    fun identify(vendorId: Int, productId: Int): ControllerProfile =
        all.firstOrNull { it.vendorId == vendorId && it.productId == productId }
            ?: all.firstOrNull { it.vendorId == vendorId }
            ?: GENERIC

    /** Look up a built-in profile by its stable [id], or the generic fallback. */
    fun byId(id: String?): ControllerProfile = all.firstOrNull { it.id == id } ?: GENERIC

    /** Metadata for every built-in profile (for the UI picker). */
    fun allMetas(): List<ControllerProfile> = all
}

// ============================================================================
// Custom remapping (layered on top of a profile)
// ============================================================================

/**
 * User overrides layered on top of a [ControllerProfile].
 *
 * Android gamepads are inconsistent (e.g. some remap A/B for left-handed play,
 * or swap triggers); these overrides let the user repair a single physical
 * control without discarding the whole database entry.
 *
 * Serialised as compact JSON for persistence in [LauncherSettings].
 */
data class ControllerRemap(
    val buttons: Map<Int, StandardButton> = emptyMap(),
    val axes: Map<Int, StandardAxis> = emptyMap(),
) {
    /** True when no overrides are present. */
    fun isEmpty(): Boolean = buttons.isEmpty() && axes.isEmpty()

    /** Resolve a native button code to a standard button (overrides first). */
    fun resolveButton(nativeCode: Int, fallback: StandardButton?): StandardButton? =
        buttons[nativeCode] ?: fallback

    /** Resolve a native axis id to a standard axis (overrides first). */
    fun resolveAxis(nativeCode: Int, fallback: StandardAxis?): StandardAxis? =
        axes[nativeCode] ?: fallback

    /** Serialise to a compact, stable JSON string. */
    fun toJsonString(): String {
        val bObj = JsonValue.Obj(
            buttons.map { (code, btn) ->
                code.toString() to JsonValue.Str(btn.code)
            }.toMap(),
        )
        val aObj = JsonValue.Obj(
            axes.map { (code, axis) ->
                code.toString() to JsonValue.Str(axis.code)
            }.toMap(),
        )
        return JsonValue.Obj(
            mapOf("buttons" to bObj, "axes" to aObj),
        ).toJsonString()
    }

    companion object {
        /** Parse a [ControllerRemap] from JSON text, or null if malformed. */
        fun fromJson(text: String?): ControllerRemap? {
            if (text.isNullOrBlank()) return ControllerRemap()
            val root = parseJson(text) as? JsonValue.Obj ?: return null
            val buttons = (root.entries["buttons"] as? JsonValue.Obj)
                ?.entries
                ?.mapNotNull { (k, v) ->
                    val code = k.toIntOrNull() ?: return@mapNotNull null
                    val btn = (v as? JsonValue.Str)?.value?.let { StandardButton.fromCode(it) }
                        ?: return@mapNotNull null
                    code to btn
                }
                ?.toMap()
                .orEmpty()
            val axes = (root.entries["axes"] as? JsonValue.Obj)
                ?.entries
                ?.mapNotNull { (k, v) ->
                    val code = k.toIntOrNull() ?: return@mapNotNull null
                    val axis = (v as? JsonValue.Str)?.value?.let { StandardAxis.fromCode(it) }
                        ?: return@mapNotNull null
                    code to axis
                }
                ?.toMap()
                .orEmpty()
            return ControllerRemap(buttons, axes)
        }
    }
}

// ============================================================================
// Input calibration (dead-zone / sensitivity) — the Input layer
// ============================================================================

/**
 * Analog input calibration: a dead-zone, a sensitivity multiplier and optional
 * axis inversion, applied by the input layer to gamepad analog values (task 4).
 *
 * Mirrors the Rust core's `gamepad::StickCalibration` / `AxisCalibration` so the
 * UI and the native engine agree on the math.
 */
data class InputCalibration(
    val deadzone: Float = DEFAULT_DEADZONE,
    val sensitivity: Float = DEFAULT_SENSITIVITY,
    val invertX: Boolean = false,
    val invertY: Boolean = false,
) {
    /** Coerce every field into a safe, usable range (never throws). */
    fun sanitized(): InputCalibration = copy(
        deadzone = deadzone.coerceIn(MIN_DEADZONE, MAX_DEADZONE),
        sensitivity = sensitivity.coerceIn(MIN_SENSITIVITY, MAX_SENSITIVITY),
    )

    /**
     * Calibrate a single analog value in `[-1, 1]`.
     *
     * Values inside the dead-zone collapse to `0`; values outside are remapped
     * into the remaining `[deadzone, 1]` range, scaled by `sensitivity`, clamped
     * back to `[-1, 1]`, and inverted when [invert] is set.
     */
    fun calibrateAxis(value: Float, invert: Boolean = invertX): Float = with(sanitized()) {
        val v = value.coerceIn(-1f, 1f)
        val mag = kotlin.math.abs(v)
        if (mag <= deadzone) return@with 0f
        val scaled = ((mag - deadzone) / (1f - deadzone)) * sensitivity * kotlin.math.sign(v)
        val out = scaled.coerceIn(-1f, 1f)
        if (invert) -out else out
    }

    /**
     * Calibrate a `(x, y)` thumbstick vector with a *radial* dead-zone.
     *
     * The whole vector is zeroed while its magnitude is within [deadzone];
     * otherwise it is rescaled into the remaining range, multiplied by
     * [sensitivity], clamped to the unit circle and axis-inverted per flag.
     */
    fun calibrateStick(x: Float, y: Float): Pair<Float, Float> = with(sanitized()) {
        val cx = x.coerceIn(-1f, 1f)
        val cy = y.coerceIn(-1f, 1f)
        val mag = kotlin.math.sqrt(cx * cx + cy * cy)
        if (mag <= deadzone || mag == 0f) return@with Pair(0f, 0f)
        val scale = ((mag - deadzone) / (1f - deadzone)) / mag * sensitivity
        var nx = (cx * scale).coerceIn(-1f, 1f)
        var ny = (cy * scale).coerceIn(-1f, 1f)
        if (invertX) nx = -nx
        if (invertY) ny = -ny
        Pair(nx, ny)
    }

    companion object {
        const val DEFAULT_DEADZONE = 0.15f
        const val DEFAULT_SENSITIVITY = 1.0f
        const val MIN_DEADZONE = 0.0f
        const val MAX_DEADZONE = 0.99f
        const val MIN_SENSITIVITY = 0.1f
        const val MAX_SENSITIVITY = 4.0f

        /** Build calibration from a profile's recommended defaults. */
        fun fromProfile(profile: ControllerProfile): InputCalibration = InputCalibration(
            deadzone = profile.defaultDeadzone,
            sensitivity = profile.defaultSensitivity,
        )

        val DEFAULT = InputCalibration()
    }
}
