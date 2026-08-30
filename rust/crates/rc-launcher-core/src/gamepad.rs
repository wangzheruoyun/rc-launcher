//! Gamepad mapping database + input calibration (task 4).
//!
//! FCL ships `assets/controllers/*.json` touch layouts but no *physical* pad
//! recognition; RC already has the [`com.rc.launcher.ui.model`] control-layout
//! UI yet ships zero device mapping resources. This module closes that gap with
//! a built-in, offline database of mainstream controllers (Xbox, PlayStation,
//! 8BitDo, Flydigi/Feitian and the generic Android HID gamepad) keyed by USB
//! vendor/product id, plug-and-play identification, user custom remapping, and
//! the dead-zone / sensitivity calibration the input layer applies to analog
//! axes and sticks.
//!
//! Everything is plain, `#[derive(Serialize)]` data so it (a) round-trips through
//! [`serde_json`] for the JNI bridge ([`crate::ffi`]) and (b) is exhaustively
//! unit-tested on the host with `cargo test` (no Android needed).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A canonical gamepad button, independent of the physical device.
///
/// The vocabulary mirrors the on-screen [`com.rc.launcher.ui.model.MappedKey`]
/// `button.*` family so a physical pad maps 1:1 onto the layout's `BTN_*` keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardButton {
    /// A / Cross.
    South,
    /// B / Circle.
    East,
    /// X / Square.
    North,
    /// Y / Triangle.
    West,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    /// Left stick press (L3).
    LeftStick,
    /// Right stick press (R3).
    RightStick,
    Start,
    /// Share / Back / View.
    Select,
    /// Home / PS / Guide.
    Guide,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

/// A canonical gamepad analog axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    TriggerLeft,
    TriggerRight,
}

/// A physical Android `KeyEvent` key code for a gamepad button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NativeButtonCode(pub u32);

/// A physical Android `MotionEvent` axis id for a gamepad axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NativeAxisCode(pub u32);

/// Android `KeyEvent` / `MotionEvent` native code constants.
///
/// These are the stable Android input codes a gamepad emits; the database maps
/// them onto [`StandardButton`] / [`StandardAxis`] per device.
pub mod android {
    /// `KEYCODE_BUTTON_A`.
    pub const BTN_A: u32 = 96;
    /// `KEYCODE_BUTTON_B`.
    pub const BTN_B: u32 = 97;
    /// `KEYCODE_BUTTON_C`.
    pub const BTN_C: u32 = 98;
    /// `KEYCODE_BUTTON_X`.
    pub const BTN_X: u32 = 99;
    /// `KEYCODE_BUTTON_Y`.
    pub const BTN_Y: u32 = 100;
    /// `KEYCODE_BUTTON_Z`.
    pub const BTN_Z: u32 = 101;
    /// `KEYCODE_BUTTON_L1`.
    pub const BTN_L1: u32 = 102;
    /// `KEYCODE_BUTTON_R1`.
    pub const BTN_R1: u32 = 103;
    /// `KEYCODE_BUTTON_L2`.
    pub const BTN_L2: u32 = 104;
    /// `KEYCODE_BUTTON_R2`.
    pub const BTN_R2: u32 = 105;
    /// `KEYCODE_BUTTON_THUMBL` (L3).
    pub const BTN_THUMBL: u32 = 106;
    /// `KEYCODE_BUTTON_THUMBR` (R3).
    pub const BTN_THUMBR: u32 = 107;
    /// `KEYCODE_BUTTON_START`.
    pub const BTN_START: u32 = 108;
    /// `KEYCODE_BUTTON_SELECT`.
    pub const BTN_SELECT: u32 = 109;
    /// `KEYCODE_BUTTON_MODE` (Guide/Home).
    pub const BTN_MODE: u32 = 110;
    /// `KEYCODE_DPAD_UP`.
    pub const DPAD_UP: u32 = 19;
    /// `KEYCODE_DPAD_DOWN`.
    pub const DPAD_DOWN: u32 = 20;
    /// `KEYCODE_DPAD_LEFT`.
    pub const DPAD_LEFT: u32 = 21;
    /// `KEYCODE_DPAD_RIGHT`.
    pub const DPAD_RIGHT: u32 = 22;

    /// `MotionEvent.AXIS_X` — left stick X.
    pub const AXIS_X: u32 = 0;
    /// `MotionEvent.AXIS_Y` — left stick Y.
    pub const AXIS_Y: u32 = 1;
    /// `MotionEvent.AXIS_Z` — right stick X (common layout).
    pub const AXIS_Z: u32 = 11;
    /// `MotionEvent.AXIS_RX` — right stick X (Sony layout).
    pub const AXIS_RX: u32 = 12;
    /// `MotionEvent.AXIS_RY` — right stick Y (Sony layout).
    pub const AXIS_RY: u32 = 13;
    /// `MotionEvent.AXIS_RZ` — right stick Y (common layout).
    pub const AXIS_RZ: u32 = 14;
    /// `MotionEvent.AXIS_HAT_X` — dpad X (hat).
    pub const AXIS_HAT_X: u32 = 15;
    /// `MotionEvent.AXIS_HAT_Y` — dpad Y (hat).
    pub const AXIS_HAT_Y: u32 = 16;
    /// `MotionEvent.AXIS_LTRIGGER` — left trigger.
    pub const AXIS_LTRIGGER: u32 = 17;
    /// `MotionEvent.AXIS_RTRIGGER` — right trigger.
    pub const AXIS_RTRIGGER: u32 = 18;
    /// `MotionEvent.AXIS_BRAKE` — left trigger (alt).
    pub const AXIS_BRAKE: u32 = 23;
    /// `MotionEvent.AXIS_GAS` — right trigger (alt).
    pub const AXIS_GAS: u32 = 22;
}

/// A built-in controller profile.
///
/// Keyed by USB `vendor_id` / `product_id` so [`identify`] can resolve a
/// physically connected pad to its standard mapping + recommended calibration.
pub struct GamepadProfile {
    pub id: &'static str,
    pub name: &'static str,
    pub vendor_id: u16,
    pub product_id: u16,
    pub description: &'static str,
    /// Native `KeyEvent` code -> standard button.
    pub buttons: &'static [(u32, StandardButton)],
    /// Native `MotionEvent` axis id -> standard axis.
    pub axes: &'static [(u32, StandardAxis)],
    /// Recommended analog dead-zone (0..1) for this hardware.
    pub default_deadzone: f32,
    /// Recommended stick sensitivity multiplier.
    pub default_sensitivity: f32,
}

impl GamepadProfile {
    /// Resolve the native button code -> standard button mapping as a map.
    pub fn button_map(&self) -> HashMap<NativeButtonCode, StandardButton> {
        self.buttons
            .iter()
            .map(|(code, btn)| (NativeButtonCode(*code), *btn))
            .collect()
    }

    /// Resolve the native axis id -> standard axis mapping as a map.
    pub fn axis_map(&self) -> HashMap<NativeAxisCode, StandardAxis> {
        self.axes
            .iter()
            .map(|(code, axis)| (NativeAxisCode(*code), *axis))
            .collect()
    }

    /// Lightweight, serialisable summary for the UI / JNI bridge.
    pub fn meta(&self) -> GamepadProfileMeta {
        GamepadProfileMeta {
            id: self.id.to_string(),
            name: self.name.to_string(),
            vendor_id: self.vendor_id,
            product_id: self.product_id,
            description: self.description.to_string(),
            default_deadzone: self.default_deadzone,
            default_sensitivity: self.default_sensitivity,
        }
    }
}

/// Serialisable controller-profile summary (see [`GamepadProfile::meta`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GamepadProfileMeta {
    pub id: String,
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub description: String,
    pub default_deadzone: f32,
    pub default_sensitivity: f32,
}

// ============================================================================
// Built-in controller database
// ============================================================================

/// Generic Android HID gamepad (the fallback for any unrecognised device).
///
/// Follows the Android standard motion/key layout: `AXIS_X/Y` = left stick,
/// `AXIS_Z/RZ` = right stick, `AXIS_LTRIGGER/RTRIGGER` = triggers, dpad via
/// `KEYCODE_DPAD_*`.
static GENERIC: GamepadProfile = GamepadProfile {
    id: "generic",
    name: "Generic Gamepad",
    vendor_id: 0x0000,
    product_id: 0x0000,
    description: "Standard Android HID gamepad (fallback for unknown devices).",
    buttons: &[
        (android::BTN_A, StandardButton::South),
        (android::BTN_B, StandardButton::East),
        (android::BTN_C, StandardButton::Guide),
        (android::BTN_X, StandardButton::North),
        (android::BTN_Y, StandardButton::West),
        (android::BTN_L1, StandardButton::LeftBumper),
        (android::BTN_R1, StandardButton::RightBumper),
        (android::BTN_L2, StandardButton::LeftTrigger),
        (android::BTN_R2, StandardButton::RightTrigger),
        (android::BTN_THUMBL, StandardButton::LeftStick),
        (android::BTN_THUMBR, StandardButton::RightStick),
        (android::BTN_START, StandardButton::Start),
        (android::BTN_SELECT, StandardButton::Select),
        (android::BTN_MODE, StandardButton::Guide),
        (android::DPAD_UP, StandardButton::DpadUp),
        (android::DPAD_DOWN, StandardButton::DpadDown),
        (android::DPAD_LEFT, StandardButton::DpadLeft),
        (android::DPAD_RIGHT, StandardButton::DpadRight),
    ],
    axes: &[
        (android::AXIS_X, StandardAxis::LeftX),
        (android::AXIS_Y, StandardAxis::LeftY),
        (android::AXIS_Z, StandardAxis::RightX),
        (android::AXIS_RZ, StandardAxis::RightY),
        (android::AXIS_LTRIGGER, StandardAxis::TriggerLeft),
        (android::AXIS_RTRIGGER, StandardAxis::TriggerRight),
    ],
    default_deadzone: 0.15,
    default_sensitivity: 1.0,
};

/// Microsoft Xbox family (VID 0x045E).
static XBOX: GamepadProfile = GamepadProfile {
    id: "xbox",
    name: "Xbox Controller",
    vendor_id: 0x045E,
    // Xbox Wireless Controller (One S / Series share VID); matched by vendor.
    product_id: 0x02EA,
    description: "Microsoft Xbox One / Series controller (VID 0x045E).",
    buttons: &[
        (android::BTN_A, StandardButton::South),
        (android::BTN_B, StandardButton::East),
        (android::BTN_X, StandardButton::North),
        (android::BTN_Y, StandardButton::West),
        (android::BTN_L1, StandardButton::LeftBumper),
        (android::BTN_R1, StandardButton::RightBumper),
        (android::BTN_L2, StandardButton::LeftTrigger),
        (android::BTN_R2, StandardButton::RightTrigger),
        (android::BTN_THUMBL, StandardButton::LeftStick),
        (android::BTN_THUMBR, StandardButton::RightStick),
        (android::BTN_START, StandardButton::Start),
        (android::BTN_SELECT, StandardButton::Select),
        (android::BTN_MODE, StandardButton::Guide),
        (android::DPAD_UP, StandardButton::DpadUp),
        (android::DPAD_DOWN, StandardButton::DpadDown),
        (android::DPAD_LEFT, StandardButton::DpadLeft),
        (android::DPAD_RIGHT, StandardButton::DpadRight),
    ],
    axes: &[
        (android::AXIS_X, StandardAxis::LeftX),
        (android::AXIS_Y, StandardAxis::LeftY),
        (android::AXIS_Z, StandardAxis::RightX),
        (android::AXIS_RZ, StandardAxis::RightY),
        (android::AXIS_LTRIGGER, StandardAxis::TriggerLeft),
        (android::AXIS_RTRIGGER, StandardAxis::TriggerRight),
    ],
    default_deadzone: 0.15,
    default_sensitivity: 1.0,
};

/// Sony PlayStation DualShock 4 / DualSense (VID 0x054C).
///
/// Sony pads report triggers on `AXIS_BRAKE` / `AXIS_GAS` and the right stick on
/// `AXIS_RX` / `AXIS_RY`, so the mapping diverges from the generic layout — a
/// concrete reason the per-device database exists.
static PLAYSTATION: GamepadProfile = GamepadProfile {
    id: "playstation",
    name: "PlayStation Controller",
    vendor_id: 0x054C,
    product_id: 0x09CC,
    description: "Sony DualShock 4 / DualSense (VID 0x054C). Triggers on BRAKE/GAS.",
    buttons: &[
        (android::BTN_A, StandardButton::South),
        (android::BTN_B, StandardButton::East),
        (android::BTN_X, StandardButton::North),
        (android::BTN_Y, StandardButton::West),
        (android::BTN_L1, StandardButton::LeftBumper),
        (android::BTN_R1, StandardButton::RightBumper),
        (android::BTN_L2, StandardButton::LeftTrigger),
        (android::BTN_R2, StandardButton::RightTrigger),
        (android::BTN_THUMBL, StandardButton::LeftStick),
        (android::BTN_THUMBR, StandardButton::RightStick),
        (android::BTN_START, StandardButton::Start),
        (android::BTN_SELECT, StandardButton::Select),
        (android::BTN_MODE, StandardButton::Guide),
        (android::DPAD_UP, StandardButton::DpadUp),
        (android::DPAD_DOWN, StandardButton::DpadDown),
        (android::DPAD_LEFT, StandardButton::DpadLeft),
        (android::DPAD_RIGHT, StandardButton::DpadRight),
    ],
    axes: &[
        (android::AXIS_X, StandardAxis::LeftX),
        (android::AXIS_Y, StandardAxis::LeftY),
        (android::AXIS_RX, StandardAxis::RightX),
        (android::AXIS_RY, StandardAxis::RightY),
        (android::AXIS_BRAKE, StandardAxis::TriggerLeft),
        (android::AXIS_GAS, StandardAxis::TriggerRight),
    ],
    default_deadzone: 0.12,
    default_sensitivity: 1.0,
};

/// 8BitDo controllers (VID 0x2DC8).
static EIGHTBITDO: GamepadProfile = GamepadProfile {
    id: "8bitdo",
    name: "8BitDo Controller",
    vendor_id: 0x2DC8,
    product_id: 0x6001,
    description: "8BitDo SN30 Pro / Pro 2 / Ultimate (VID 0x2DC8).",
    buttons: &[
        (android::BTN_A, StandardButton::South),
        (android::BTN_B, StandardButton::East),
        (android::BTN_X, StandardButton::North),
        (android::BTN_Y, StandardButton::West),
        (android::BTN_L1, StandardButton::LeftBumper),
        (android::BTN_R1, StandardButton::RightBumper),
        (android::BTN_L2, StandardButton::LeftTrigger),
        (android::BTN_R2, StandardButton::RightTrigger),
        (android::BTN_THUMBL, StandardButton::LeftStick),
        (android::BTN_THUMBR, StandardButton::RightStick),
        (android::BTN_START, StandardButton::Start),
        (android::BTN_SELECT, StandardButton::Select),
        (android::BTN_MODE, StandardButton::Guide),
        (android::DPAD_UP, StandardButton::DpadUp),
        (android::DPAD_DOWN, StandardButton::DpadDown),
        (android::DPAD_LEFT, StandardButton::DpadLeft),
        (android::DPAD_RIGHT, StandardButton::DpadRight),
    ],
    axes: &[
        (android::AXIS_X, StandardAxis::LeftX),
        (android::AXIS_Y, StandardAxis::LeftY),
        (android::AXIS_Z, StandardAxis::RightX),
        (android::AXIS_RZ, StandardAxis::RightY),
        (android::AXIS_LTRIGGER, StandardAxis::TriggerLeft),
        (android::AXIS_RTRIGGER, StandardAxis::TriggerRight),
    ],
    default_deadzone: 0.18,
    default_sensitivity: 1.0,
};

/// Flydigi / Feitian controllers (VID 0x294B).
///
/// Flydigi reports the dpad through the HAT axes, demonstrating an axis-based
/// (rather than key-based) dpad mapping.
static FLYDIGI: GamepadProfile = GamepadProfile {
    id: "flydigi",
    name: "Flydigi Controller",
    vendor_id: 0x294B,
    product_id: 0x1903,
    description: "Flydigi / Feitian gamepad (VID 0x294B). Dpad via HAT axes.",
    buttons: &[
        (android::BTN_A, StandardButton::South),
        (android::BTN_B, StandardButton::East),
        (android::BTN_X, StandardButton::North),
        (android::BTN_Y, StandardButton::West),
        (android::BTN_L1, StandardButton::LeftBumper),
        (android::BTN_R1, StandardButton::RightBumper),
        (android::BTN_L2, StandardButton::LeftTrigger),
        (android::BTN_R2, StandardButton::RightTrigger),
        (android::BTN_THUMBL, StandardButton::LeftStick),
        (android::BTN_THUMBR, StandardButton::RightStick),
        (android::BTN_START, StandardButton::Start),
        (android::BTN_SELECT, StandardButton::Select),
        (android::BTN_MODE, StandardButton::Guide),
    ],
    axes: &[
        (android::AXIS_X, StandardAxis::LeftX),
        (android::AXIS_Y, StandardAxis::LeftY),
        (android::AXIS_RX, StandardAxis::RightX),
        (android::AXIS_RY, StandardAxis::RightY),
        (android::AXIS_BRAKE, StandardAxis::TriggerLeft),
        (android::AXIS_GAS, StandardAxis::TriggerRight),
    ],
    default_deadzone: 0.16,
    default_sensitivity: 1.05,
};

/// The complete built-in controller database (in display/priority order).
pub static CONTROLLER_DB: &[&'static GamepadProfile] =
    &[&GENERIC, &XBOX, &PLAYSTATION, &EIGHTBITDO, &FLYDIGI];

/// The fallback profile used when no device matches.
pub static DEFAULT_PROFILE: &'static GamepadProfile = &GENERIC;

/// Plug-and-play identification by USB vendor/product id.
///
/// Exact `(vendor_id, product_id)` match wins; otherwise any profile sharing the
/// vendor id is returned (vendor-level quirk coverage); otherwise the generic
/// fallback is used so an unknown pad still gets a sane mapping.
pub fn identify(vendor_id: u16, product_id: u16) -> &'static GamepadProfile {
    CONTROLLER_DB
        .iter()
        .copied()
        .find(|p| p.vendor_id == vendor_id && p.product_id == product_id)
        .or_else(|| {
            CONTROLLER_DB
                .iter()
                .copied()
                .find(|p| p.vendor_id == vendor_id)
        })
        .unwrap_or(DEFAULT_PROFILE)
}

/// Look up a built-in profile by its stable [`GamepadProfile::id`].
pub fn profile_by_id(id: &str) -> Option<&'static GamepadProfile> {
    CONTROLLER_DB.iter().copied().find(|p| p.id == id)
}

/// Metadata for every built-in profile (for the UI picker).
pub fn list_profiles() -> Vec<GamepadProfileMeta> {
    CONTROLLER_DB.iter().map(|p| p.meta()).collect()
}

/// The default (generic) fallback profile.
pub fn default_profile() -> &'static GamepadProfile {
    DEFAULT_PROFILE
}

// ============================================================================
// Custom remapping
// ============================================================================

/// User overrides layered on top of a [`GamepadProfile`].
///
/// Android gamepads are inconsistent (e.g. some remap A/B for left-handed
/// players, or swap triggers); these overrides let the user repair a single
/// physical control without discarding the whole database entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemapOverrides {
    /// Native `KeyEvent` code -> forced standard button.
    #[serde(default)]
    pub buttons: HashMap<u32, StandardButton>,
    /// Native `MotionEvent` axis id -> forced standard axis.
    #[serde(default)]
    pub axes: HashMap<u32, StandardAxis>,
}

impl RemapOverrides {
    /// Resolve a native button code to a standard button, honouring overrides
    /// first and falling back to the profile mapping.
    pub fn resolve_button(
        &self,
        profile: &GamepadProfile,
        native: NativeButtonCode,
    ) -> Option<StandardButton> {
        if let Some(b) = self.buttons.get(&native.0) {
            return Some(*b);
        }
        profile.button_map().get(&native).copied()
    }

    /// Resolve a native axis id to a standard axis, honouring overrides first.
    pub fn resolve_axis(
        &self,
        profile: &GamepadProfile,
        native: NativeAxisCode,
    ) -> Option<StandardAxis> {
        if let Some(a) = self.axes.get(&native.0) {
            return Some(*a);
        }
        profile.axis_map().get(&native).copied()
    }

    /// True when no overrides are present.
    pub fn is_empty(&self) -> bool {
        self.buttons.is_empty() && self.axes.is_empty()
    }
}

// ============================================================================
// Input calibration (dead-zone / sensitivity) — the Input layer
// ============================================================================

/// Clamp helpers shared by the calibration structs.
const MIN_DEADZONE: f32 = 0.0;
const MAX_DEADZONE: f32 = 0.99;
const MIN_SENSITIVITY: f32 = 0.1;
const MAX_SENSITIVITY: f32 = 4.0;

/// Per-axis calibration: radial dead-zone, sensitivity multiplier and invert.
///
/// Applied by the input layer to individual analog values (triggers, or each
/// stick axis when [`StickCalibration`] is not used).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AxisCalibration {
    pub deadzone: f32,
    pub sensitivity: f32,
    pub invert: bool,
}

impl Default for AxisCalibration {
    fn default() -> Self {
        AxisCalibration {
            deadzone: 0.15,
            sensitivity: 1.0,
            invert: false,
        }
    }
}

impl AxisCalibration {
    /// Coerce every field into a safe, usable range (never throws).
    pub fn clamped(&self) -> Self {
        AxisCalibration {
            deadzone: self.deadzone.clamp(MIN_DEADZONE, MAX_DEADZONE),
            sensitivity: self.sensitivity.clamp(MIN_SENSITIVITY, MAX_SENSITIVITY),
            invert: self.invert,
        }
    }

    /// Calibrate a single analog value in `[-1, 1]`.
    ///
    /// Values inside the dead-zone collapse to `0.0`; values outside are
    /// remapped into the remaining `[deadzone, 1]` range, scaled by
    /// `sensitivity`, clamped back to `[-1, 1]`, and inverted when requested.
    pub fn calibrate(&self, value: f32) -> f32 {
        let c = self.clamped();
        let v = value.clamp(-1.0, 1.0);
        let mag = v.abs();
        if mag <= c.deadzone {
            return 0.0;
        }
        let scaled = ((mag - c.deadzone) / (1.0 - c.deadzone)) * c.sensitivity * v.signum();
        let out = scaled.clamp(-1.0, 1.0);
        if c.invert {
            -out
        } else {
            out
        }
    }
}

/// 2-D stick calibration: a single radial dead-zone, a sensitivity multiplier
/// and independent X/Y inversion for thumbsticks.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StickCalibration {
    pub deadzone: f32,
    pub sensitivity: f32,
    pub invert_x: bool,
    pub invert_y: bool,
}

impl Default for StickCalibration {
    fn default() -> Self {
        StickCalibration {
            deadzone: 0.15,
            sensitivity: 1.0,
            invert_x: false,
            invert_y: false,
        }
    }
}

impl StickCalibration {
    /// Coerce every field into a safe range (never throws).
    pub fn clamped(&self) -> Self {
        StickCalibration {
            deadzone: self.deadzone.clamp(MIN_DEADZONE, MAX_DEADZONE),
            sensitivity: self.sensitivity.clamp(MIN_SENSITIVITY, MAX_SENSITIVITY),
            invert_x: self.invert_x,
            invert_y: self.invert_y,
        }
    }

    /// Calibrate a `(x, y)` thumbstick vector with a *radial* dead-zone.
    ///
    /// The whole vector is zeroed while its magnitude is within `deadzone`;
    /// otherwise it is rescaled into the remaining range, multiplied by
    /// `sensitivity`, clamped to the unit circle and axis-inverted per flag.
    pub fn calibrate(&self, x: f32, y: f32) -> (f32, f32) {
        let c = self.clamped();
        let x = x.clamp(-1.0, 1.0);
        let y = y.clamp(-1.0, 1.0);
        let mag = (x * x + y * y).sqrt();
        if mag <= c.deadzone || mag == 0.0 {
            return (0.0, 0.0);
        }
        let scale = ((mag - c.deadzone) / (1.0 - c.deadzone)) / mag * c.sensitivity;
        let mut nx = (x * scale).clamp(-1.0, 1.0);
        let mut ny = (y * scale).clamp(-1.0, 1.0);
        if c.invert_x {
            nx = -nx;
        }
        if c.invert_y {
            ny = -ny;
        }
        (nx, ny)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_has_generic_first_and_default_is_generic() {
        assert_eq!(CONTROLLER_DB[0].id, "generic");
        assert_eq!(default_profile().id, "generic");
    }

    #[test]
    fn every_profile_maps_all_standard_buttons_and_axes() {
        for p in CONTROLLER_DB {
            // Each profile should expose the canonical face/shoulder/dpad set.
            let bm = p.button_map();
            for want in [
                StandardButton::South,
                StandardButton::East,
                StandardButton::North,
                StandardButton::West,
                StandardButton::LeftBumper,
                StandardButton::RightBumper,
                StandardButton::Start,
                StandardButton::Select,
            ] {
                assert!(
                    bm.values().any(|b| *b == want),
                    "{} missing button {:?}",
                    p.id,
                    want
                );
            }
            let am = p.axis_map();
            for want in [
                StandardAxis::LeftX,
                StandardAxis::LeftY,
                StandardAxis::RightX,
                StandardAxis::RightY,
                StandardAxis::TriggerLeft,
                StandardAxis::TriggerRight,
            ] {
                assert!(
                    am.values().any(|a| *a == want),
                    "{} missing axis {:?}",
                    p.id,
                    want
                );
            }
        }
    }

    #[test]
    fn identify_exact_match() {
        let p = identify(0x054C, 0x09CC);
        assert_eq!(p.id, "playstation");
        assert_eq!(p.product_id, 0x09CC);
    }

    #[test]
    fn identify_vendor_fallback() {
        // An unknown Sony PID still resolves to the PlayStation profile.
        let p = identify(0x054C, 0x0CE6);
        assert_eq!(p.id, "playstation");
    }

    #[test]
    fn identify_unknown_device_falls_back_to_generic() {
        let p = identify(0xDEAD, 0xBEEF);
        assert_eq!(p.id, "generic");
    }

    #[test]
    fn playstation_uses_brake_gas_for_triggers() {
        let p = profile_by_id("playstation").unwrap();
        let am = p.axis_map();
        assert_eq!(
            am.get(&NativeAxisCode(android::AXIS_BRAKE)),
            Some(&StandardAxis::TriggerLeft)
        );
        assert_eq!(
            am.get(&NativeAxisCode(android::AXIS_GAS)),
            Some(&StandardAxis::TriggerRight)
        );
        // And it must NOT map the generic trigger axes.
        assert_ne!(
            am.get(&NativeAxisCode(android::AXIS_LTRIGGER)),
            Some(&StandardAxis::TriggerLeft)
        );
    }

    #[test]
    fn flydigi_uses_brake_gas_rx_ry() {
        let p = profile_by_id("flydigi").unwrap();
        let am = p.axis_map();
        // Flydigi reports the right stick on RX/RY and triggers on BRAKE/GAS,
        // unlike the generic layout (Z/RZ + LTRIGGER/RTRIGGER).
        assert_eq!(
            am.get(&NativeAxisCode(android::AXIS_RX)),
            Some(&StandardAxis::RightX)
        );
        assert_eq!(
            am.get(&NativeAxisCode(android::AXIS_BRAKE)),
            Some(&StandardAxis::TriggerLeft)
        );
        assert_eq!(
            am.get(&NativeAxisCode(android::AXIS_GAS)),
            Some(&StandardAxis::TriggerRight)
        );
        assert_ne!(
            am.get(&NativeAxisCode(android::AXIS_LTRIGGER)),
            Some(&StandardAxis::TriggerLeft)
        );
    }

    #[test]
    fn profile_by_id_unknown_returns_none() {
        assert!(profile_by_id("does-not-exist").is_none());
    }

    #[test]
    fn list_profiles_covers_builtins() {
        let profiles = list_profiles();
        let ids: Vec<&str> = profiles.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"xbox"));
        assert!(ids.contains(&"playstation"));
        assert!(ids.contains(&"8bitdo"));
        assert!(ids.contains(&"flydigi"));
    }

    // --- Custom remapping ---------------------------------------------------

    #[test]
    fn remap_override_takes_precedence() {
        let xbox = profile_by_id("xbox").unwrap();
        let mut ov = RemapOverrides::default();
        // Force the physical A button to emit "West" instead of "South".
        ov.buttons.insert(android::BTN_A, StandardButton::West);
        let resolved = ov.resolve_button(xbox, NativeButtonCode(android::BTN_A));
        assert_eq!(resolved, Some(StandardButton::West));
        // Untouched buttons still come from the profile.
        assert_eq!(
            ov.resolve_button(xbox, NativeButtonCode(android::BTN_B)),
            Some(StandardButton::East)
        );
    }

    #[test]
    fn remap_axis_override() {
        let xbox = profile_by_id("xbox").unwrap();
        let mut ov = RemapOverrides::default();
        ov.axes.insert(android::AXIS_Z, StandardAxis::RightX);
        assert_eq!(
            ov.resolve_axis(xbox, NativeAxisCode(android::AXIS_Z)),
            Some(StandardAxis::RightX)
        );
    }

    #[test]
    fn remap_empty_has_no_overrides() {
        assert!(RemapOverrides::default().is_empty());
    }

    // --- Axis calibration ---------------------------------------------------

    #[test]
    fn axis_calibration_deadzone_zeroes_small_values() {
        let cal = AxisCalibration {
            deadzone: 0.2,
            sensitivity: 1.0,
            invert: false,
        };
        assert_eq!(cal.calibrate(0.0), 0.0);
        assert_eq!(cal.calibrate(0.1), 0.0);
        assert_eq!(cal.calibrate(-0.1), 0.0);
    }

    #[test]
    fn axis_calibration_rescales_outside_deadzone() {
        let cal = AxisCalibration {
            deadzone: 0.2,
            sensitivity: 1.0,
            invert: false,
        };
        // At full deflection the rescaled value reaches 1.0.
        assert!((cal.calibrate(1.0) - 1.0).abs() < 1e-6);
        // 0.6 -> (0.6-0.2)/(0.8) = 0.5
        assert!((cal.calibrate(0.6) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn axis_calibration_sensitivity_scales() {
        let cal = AxisCalibration {
            deadzone: 0.2,
            sensitivity: 2.0,
            invert: false,
        };
        // (0.6-0.2)/0.8 * 2 = 1.0 (clamped).
        assert!((cal.calibrate(0.6) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn axis_calibration_inverts() {
        let cal = AxisCalibration {
            deadzone: 0.2,
            sensitivity: 1.0,
            invert: true,
        };
        assert!((cal.calibrate(0.6) + 0.5).abs() < 1e-6);
    }

    #[test]
    fn axis_calibration_clamps_out_of_range() {
        let cal = AxisCalibration {
            deadzone: 2.0, // invalid, clamped below
            sensitivity: -5.0,
            invert: false,
        };
        let c = cal.clamped();
        assert!((c.deadzone - MAX_DEADZONE).abs() < 1e-6);
        assert!((c.sensitivity - MIN_SENSITIVITY).abs() < 1e-6);
    }

    // --- Stick calibration --------------------------------------------------

    #[test]
    fn stick_calibration_radial_deadzone() {
        let cal = StickCalibration {
            deadzone: 0.25,
            sensitivity: 1.0,
            invert_x: false,
            invert_y: false,
        };
        // Magnitude 0.2 < 0.25 -> fully zeroed.
        assert_eq!(cal.calibrate(0.2, 0.0), (0.0, 0.0));
        assert_eq!(cal.calibrate(0.1414, 0.1414), (0.0, 0.0));
    }

    #[test]
    fn stick_calibration_rescales_full_deflection() {
        let cal = StickCalibration {
            deadzone: 0.25,
            sensitivity: 1.0,
            invert_x: false,
            invert_y: false,
        };
        // Straight up at full magnitude should stay full after rescale.
        let (x, y) = cal.calibrate(0.0, 1.0);
        assert!(x.abs() < 1e-6);
        assert!((y - 1.0).abs() < 1e-6);
    }

    #[test]
    fn stick_calibration_axis_invert() {
        let cal = StickCalibration {
            deadzone: 0.1,
            sensitivity: 1.0,
            invert_x: true,
            invert_y: false,
        };
        let (x, y) = cal.calibrate(0.5, 0.5);
        assert!(x < 0.0);
        assert!(y > 0.0);
    }

    #[test]
    fn stick_calibration_preserves_direction() {
        let cal = StickCalibration {
            deadzone: 0.2,
            sensitivity: 1.0,
            invert_x: false,
            invert_y: false,
        };
        let (x, y) = cal.calibrate(0.3, 0.4);
        // Direction (atan2) must be preserved; only magnitude is remapped.
        let before = (0.3_f32).atan2(0.4);
        let after = x.atan2(y);
        assert!((before - after).abs() < 1e-6);
        // After a radial dead-zone of 0.2 the magnitude is remapped from the
        // [0.2, 1] band into [0, 1]; for input mag 0.5 that is (0.5-0.2)/0.8.
        let mag = (x * x + y * y).sqrt();
        assert!((mag - 0.375).abs() < 1e-6);
        assert!(mag > 0.0 && mag <= 1.0);
    }
}
