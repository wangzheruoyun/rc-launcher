//! Screen-orientation adaptation and window size classes (task 9).
//!
//! Android phones rotate; a Minecraft launcher therefore has to answer three
//! very different questions, and this module is the **single source of truth**
//! for all three so the Rust core and the Compose UI can never disagree:
//!
//! 1. **What orientation may the Activity take?** — [`OrientationPolicy`]
//!    (`跟随系统` / `强制横屏` / `强制竖屏`) maps the user's choice onto the
//!    `android:screenOrientation` value the Activity requests
//!    ([`OrientationPolicy::android_screen_orientation`]) *and* onto the game
//!    window geometry ([`OrientationPolicy::orient`]): forcing portrait must not
//!    leave Minecraft with a 1280x720 landscape window letterboxed into a tall
//!    surface.
//! 2. **How should the launcher UI be laid out right now?** —
//!    [`WindowMetrics`] turns the live `width x height` (in *dp*) into
//!    Material-style [`SizeClass`] breakpoints plus the concrete layout
//!    decisions the Compose layer needs (navigation rail vs. bottom bar, grid
//!    column counts, content padding). Kotlin mirrors this table in
//!    `ui/AdaptiveLayout.kt`; `scripts/check_layout_parity.py` fails the build
//!    when the two drift apart.
//! 3. **Did the surface just rotate?** — [`ScreenOrientation::of`] +
//!    [`rotation_flips`] classify a surface-size change. A rotation invalidates
//!    every *in-flight* gesture (the finger is somewhere else now and the
//!    letterbox bars moved), so the AWT session releases held buttons instead of
//!    silently re-mapping the coordinates into the new viewport — which is
//!    exactly the "input lands in the wrong place after rotating" bug.
//!
//! Everything here is integer / branch math with no allocation, so the UI can
//! call it per frame and the tests can cover every breakpoint exhaustively.

use serde::{Deserialize, Serialize};

use crate::launch::options::WindowSize;

// ===========================================================================
// Orientation
// ===========================================================================

/// Orientation of a concrete rectangle (a surface, a window, a desktop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenOrientation {
    /// Wider than tall.
    Landscape,
    /// Taller than wide.
    Portrait,
    /// Exactly square — deliberately its *own* value: a square surface must not
    /// be reported as a rotation when it later becomes landscape or portrait.
    Square,
}

impl ScreenOrientation {
    /// Classify a `width x height` rectangle. A degenerate (zero) axis is
    /// reported as [`ScreenOrientation::Square`] because nothing can be known
    /// about it yet — the surface has not been measured.
    pub fn of(width: u32, height: u32) -> Self {
        if width == 0 || height == 0 {
            return ScreenOrientation::Square;
        }
        match width.cmp(&height) {
            std::cmp::Ordering::Greater => ScreenOrientation::Landscape,
            std::cmp::Ordering::Less => ScreenOrientation::Portrait,
            std::cmp::Ordering::Equal => ScreenOrientation::Square,
        }
    }

    /// Classify a [`WindowSize`].
    pub fn of_window(size: WindowSize) -> Self {
        Self::of(size.width, size.height)
    }

    /// Stable id shared with the Kotlin mirror / the FFI JSON.
    pub fn id(self) -> &'static str {
        match self {
            ScreenOrientation::Landscape => "landscape",
            ScreenOrientation::Portrait => "portrait",
            ScreenOrientation::Square => "square",
        }
    }

    /// Parse [`Self::id`].
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "landscape" => Some(ScreenOrientation::Landscape),
            "portrait" => Some(ScreenOrientation::Portrait),
            "square" => Some(ScreenOrientation::Square),
            _ => None,
        }
    }

    /// `true` for [`ScreenOrientation::Landscape`] only.
    pub fn is_landscape(self) -> bool {
        matches!(self, ScreenOrientation::Landscape)
    }

    /// `true` for [`ScreenOrientation::Portrait`] only.
    pub fn is_portrait(self) -> bool {
        matches!(self, ScreenOrientation::Portrait)
    }

    /// `true` when going from `self` to `other` is a real 90° rotation, i.e.
    /// landscape ⇄ portrait. Growing/shrinking inside one orientation (the soft
    /// keyboard, split screen, a foldable hinge) is *not* a rotation, and neither
    /// is passing through a square/unmeasured state.
    pub fn flips_to(self, other: Self) -> bool {
        matches!(
            (self, other),
            (ScreenOrientation::Landscape, ScreenOrientation::Portrait)
                | (ScreenOrientation::Portrait, ScreenOrientation::Landscape)
        )
    }
}

/// `true` when resizing a surface from `from` to `to` flips the orientation.
///
/// This is the predicate the AWT session and the Compose canvas use to decide
/// whether a resize has to drop the in-flight gesture (see the module docs).
pub fn rotation_flips(from: (u32, u32), to: (u32, u32)) -> bool {
    ScreenOrientation::of(from.0, from.1).flips_to(ScreenOrientation::of(to.0, to.1))
}

/// The screen-orientation strategy the user picked in the settings centre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrientationPolicy {
    /// Follow the system / the user's rotation lock (**default**).
    ///
    /// Serialised as `"system"` — *not* the snake_case variant name — so the JSON
    /// wire form is byte-identical to [`Self::id`] and to the `orientationModeId`
    /// the Kotlin settings persist: the UI can hand its stored id straight to
    /// `LaunchOptions.orientation` with no translation table in between. The old
    /// snake_case spelling stays accepted as an alias so any blob already written
    /// by an earlier build still loads.
    #[default]
    #[serde(rename = "system", alias = "follow_system")]
    FollowSystem,
    /// Always landscape (both sensor directions allowed).
    Landscape,
    /// Always portrait (both sensor directions allowed).
    Portrait,
}

impl OrientationPolicy {
    /// Every policy, in UI order.
    pub const ALL: [OrientationPolicy; 3] = [
        OrientationPolicy::FollowSystem,
        OrientationPolicy::Landscape,
        OrientationPolicy::Portrait,
    ];

    /// Stable id, shared with the Kotlin `OrientationMode.id` and persisted in
    /// the launcher settings — never change these strings.
    pub fn id(self) -> &'static str {
        match self {
            OrientationPolicy::FollowSystem => "system",
            OrientationPolicy::Landscape => "landscape",
            OrientationPolicy::Portrait => "portrait",
        }
    }

    /// Parse [`Self::id`], falling back to the default for anything unknown so a
    /// corrupt settings blob can never wedge the Activity in a broken state.
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "system" => Some(OrientationPolicy::FollowSystem),
            "landscape" => Some(OrientationPolicy::Landscape),
            "portrait" => Some(OrientationPolicy::Portrait),
            _ => None,
        }
    }

    /// Lenient parse used on the settings path.
    pub fn from_id_or_default(id: &str) -> Self {
        Self::from_id(id).unwrap_or_default()
    }

    /// The `android:screenOrientation` / `ActivityInfo.SCREEN_ORIENTATION_*`
    /// name the Activity must request.
    ///
    /// * `user` — honours the device rotation lock, so "follow system" really
    ///   means "follow the *user*", not "spin even when rotation is locked".
    /// * `sensorLandscape` / `sensorPortrait` — pinned axis but *both*
    ///   directions, so a tablet in a landscape dock or a phone held upside
    ///   down still shows an upright picture.
    pub fn android_screen_orientation(self) -> &'static str {
        match self {
            OrientationPolicy::FollowSystem => "user",
            OrientationPolicy::Landscape => "sensorLandscape",
            OrientationPolicy::Portrait => "sensorPortrait",
        }
    }

    /// The orientation this policy pins the screen to, if any.
    pub fn forced(self) -> Option<ScreenOrientation> {
        match self {
            OrientationPolicy::FollowSystem => None,
            OrientationPolicy::Landscape => Some(ScreenOrientation::Landscape),
            OrientationPolicy::Portrait => Some(ScreenOrientation::Portrait),
        }
    }

    /// Whether `orientation` is reachable under this policy.
    pub fn allows(self, orientation: ScreenOrientation) -> bool {
        match self.forced() {
            None => true,
            Some(forced) => orientation == forced || orientation == ScreenOrientation::Square,
        }
    }

    /// Orient a window/desktop size so its longer axis matches the policy,
    /// swapping the axes when needed. Under [`OrientationPolicy::FollowSystem`]
    /// the caller's size is kept verbatim.
    ///
    /// This is what keeps the game window in sync with a forced orientation: a
    /// 1280x720 default becomes 720x1280 in forced portrait instead of being
    /// letterboxed into two fat black bars.
    pub fn orient(self, size: WindowSize) -> WindowSize {
        let needs_swap = match self.forced() {
            None => false,
            Some(ScreenOrientation::Landscape) => size.height > size.width,
            Some(ScreenOrientation::Portrait) => size.width > size.height,
            Some(ScreenOrientation::Square) => false,
        };
        if needs_swap {
            WindowSize {
                width: size.height,
                height: size.width,
            }
        } else {
            size
        }
    }

    /// JSON descriptor for the settings picker (`id` + the Android value), so the
    /// UI catalogue is generated from the core instead of duplicated by hand.
    pub fn to_json(self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id(),
            "android_screen_orientation": self.android_screen_orientation(),
            "forced": self.forced().map(|o| o.id()),
        })
    }
}

// ===========================================================================
// Window size classes
// ===========================================================================

/// Width breakpoint: `< 600dp` is a phone in portrait.
pub const WIDTH_MEDIUM_DP: u32 = 600;
/// Width breakpoint: `>= 840dp` is a tablet / large foldable.
pub const WIDTH_EXPANDED_DP: u32 = 840;
/// Height breakpoint: `< 480dp` is a phone in landscape (very little height).
pub const HEIGHT_MEDIUM_DP: u32 = 480;
/// Height breakpoint: `>= 900dp` is a tall tablet.
pub const HEIGHT_EXPANDED_DP: u32 = 900;

/// Material-style window size class for one axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeClass {
    /// Phone-sized.
    Compact,
    /// Large phone in landscape / small tablet.
    Medium,
    /// Tablet / desktop-class.
    Expanded,
}

impl SizeClass {
    /// Classify a width in dp.
    pub fn of_width_dp(width_dp: u32) -> Self {
        if width_dp < WIDTH_MEDIUM_DP {
            SizeClass::Compact
        } else if width_dp < WIDTH_EXPANDED_DP {
            SizeClass::Medium
        } else {
            SizeClass::Expanded
        }
    }

    /// Classify a height in dp.
    pub fn of_height_dp(height_dp: u32) -> Self {
        if height_dp < HEIGHT_MEDIUM_DP {
            SizeClass::Compact
        } else if height_dp < HEIGHT_EXPANDED_DP {
            SizeClass::Medium
        } else {
            SizeClass::Expanded
        }
    }

    /// Stable id shared with the Kotlin mirror.
    pub fn id(self) -> &'static str {
        match self {
            SizeClass::Compact => "compact",
            SizeClass::Medium => "medium",
            SizeClass::Expanded => "expanded",
        }
    }

    /// Parse [`Self::id`].
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "compact" => Some(SizeClass::Compact),
            "medium" => Some(SizeClass::Medium),
            "expanded" => Some(SizeClass::Expanded),
            _ => None,
        }
    }
}

/// The live launcher window, in **dp**, plus every layout decision derived from
/// it. Cheap to build (two integers), so the UI recomputes it on every
/// configuration change instead of caching stale geometry across a rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowMetrics {
    /// Window width in dp.
    pub width_dp: u32,
    /// Window height in dp.
    pub height_dp: u32,
}

impl Default for WindowMetrics {
    fn default() -> Self {
        // A "reference phone" in portrait (Pixel-class, 392x872dp).
        Self {
            width_dp: 392,
            height_dp: 872,
        }
    }
}

impl WindowMetrics {
    /// Clamp bound so a bogus configuration cannot produce absurd column counts.
    pub const MAX_DP: u32 = 10_000;

    /// Sanitising constructor: zero/huge values are clamped into a usable range.
    pub fn new(width_dp: u32, height_dp: u32) -> Self {
        Self {
            width_dp: width_dp.clamp(1, Self::MAX_DP),
            height_dp: height_dp.clamp(1, Self::MAX_DP),
        }
    }

    /// The same window rotated by 90° (used by the tests and by previews).
    pub fn rotated(self) -> Self {
        Self {
            width_dp: self.height_dp,
            height_dp: self.width_dp,
        }
    }

    /// Orientation of the window.
    pub fn orientation(self) -> ScreenOrientation {
        ScreenOrientation::of(self.width_dp, self.height_dp)
    }

    /// `true` when the window is at least as wide as it is tall — the layout
    /// treats a square window as landscape because the extra width is what the
    /// navigation rail needs.
    pub fn is_landscape(self) -> bool {
        self.width_dp >= self.height_dp
    }

    /// Width size class.
    pub fn width_class(self) -> SizeClass {
        SizeClass::of_width_dp(self.width_dp)
    }

    /// Height size class.
    pub fn height_class(self) -> SizeClass {
        SizeClass::of_height_dp(self.height_dp)
    }

    /// `true` when vertical space is scarce (phone in landscape): the UI then
    /// drops hero sections and *must not* also spend 80dp on a bottom bar.
    pub fn is_short(self) -> bool {
        self.height_class() == SizeClass::Compact
    }

    /// Side navigation rail instead of a bottom navigation bar.
    ///
    /// Landscape phones are the important case: a bottom bar eats the scarce
    /// vertical space *and* sits under the thumbs that hold the device, so every
    /// landscape window and every window at least [`WIDTH_MEDIUM_DP`] wide gets
    /// the rail.
    pub fn uses_navigation_rail(self) -> bool {
        self.is_landscape() || self.width_class() != SizeClass::Compact
    }

    /// Column count for the instance grid (home + instances screens).
    pub fn instance_columns(self) -> u32 {
        match self.width_class() {
            SizeClass::Compact => {
                // A landscape phone is short but wide enough for two cards.
                if self.is_landscape() {
                    2
                } else {
                    1
                }
            }
            SizeClass::Medium => 2,
            SizeClass::Expanded => 3,
        }
    }

    /// Column count for the settings screen's section cards.
    pub fn settings_columns(self) -> u32 {
        match self.width_class() {
            SizeClass::Compact => 1,
            SizeClass::Medium => {
                if self.is_landscape() {
                    2
                } else {
                    1
                }
            }
            SizeClass::Expanded => 2,
        }
    }

    /// Column count for the home dashboard's summary cards.
    pub fn dashboard_columns(self) -> u32 {
        if self.width_class() == SizeClass::Compact && !self.is_landscape() {
            1
        } else {
            2
        }
    }

    /// Screen edge padding in dp.
    pub fn content_padding_dp(self) -> u32 {
        match self.width_class() {
            SizeClass::Compact => 16,
            SizeClass::Medium => 20,
            SizeClass::Expanded => 24,
        }
    }

    /// Upper bound for a text column, so a 1200dp tablet does not render
    /// 200-character-long settings descriptions.
    pub fn max_content_width_dp(self) -> u32 {
        match self.width_class() {
            SizeClass::Compact => Self::MAX_DP,
            SizeClass::Medium => 720,
            SizeClass::Expanded => 1080,
        }
    }

    /// Diagnostics / FFI snapshot.
    pub fn to_json(self) -> serde_json::Value {
        serde_json::json!({
            "width_dp": self.width_dp,
            "height_dp": self.height_dp,
            "orientation": self.orientation().id(),
            "width_class": self.width_class().id(),
            "height_class": self.height_class().id(),
            "landscape": self.is_landscape(),
            "short": self.is_short(),
            "navigation_rail": self.uses_navigation_rail(),
            "instance_columns": self.instance_columns(),
            "settings_columns": self.settings_columns(),
            "dashboard_columns": self.dashboard_columns(),
            "content_padding_dp": self.content_padding_dp(),
            "max_content_width_dp": self.max_content_width_dp(),
        })
    }
}

/// The full orientation catalogue as JSON (consumed by the settings picker).
pub fn orientation_catalog_json() -> serde_json::Value {
    serde_json::Value::Array(OrientationPolicy::ALL.iter().map(|p| p.to_json()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(w: u32, h: u32) -> WindowSize {
        WindowSize {
            width: w,
            height: h,
        }
    }

    #[test]
    fn orientation_of_a_rectangle() {
        assert_eq!(
            ScreenOrientation::of(1920, 1080),
            ScreenOrientation::Landscape
        );
        assert_eq!(
            ScreenOrientation::of(1080, 1920),
            ScreenOrientation::Portrait
        );
        assert_eq!(ScreenOrientation::of(1000, 1000), ScreenOrientation::Square);
        // Unmeasured surfaces are square, never a spurious rotation.
        assert_eq!(ScreenOrientation::of(0, 0), ScreenOrientation::Square);
        assert_eq!(ScreenOrientation::of(1080, 0), ScreenOrientation::Square);
        assert_eq!(
            ScreenOrientation::of_window(win(720, 1280)),
            ScreenOrientation::Portrait
        );
    }

    #[test]
    fn orientation_ids_round_trip() {
        for o in [
            ScreenOrientation::Landscape,
            ScreenOrientation::Portrait,
            ScreenOrientation::Square,
        ] {
            assert_eq!(ScreenOrientation::from_id(o.id()), Some(o));
        }
        assert_eq!(ScreenOrientation::from_id("sideways"), None);
        assert!(ScreenOrientation::Landscape.is_landscape());
        assert!(ScreenOrientation::Portrait.is_portrait());
        assert!(!ScreenOrientation::Square.is_landscape());
        assert!(!ScreenOrientation::Square.is_portrait());
    }

    #[test]
    fn only_a_real_quarter_turn_counts_as_a_rotation() {
        // Rotation: the gesture must be dropped.
        assert!(rotation_flips((1080, 2400), (2400, 1080)));
        assert!(rotation_flips((2400, 1080), (1080, 2400)));
        // The soft keyboard shrinking the surface is not a rotation.
        assert!(!rotation_flips((1080, 2400), (1080, 1400)));
        // Split screen inside one orientation is not a rotation.
        assert!(!rotation_flips((2400, 1080), (1200, 1080)));
        // A square (or not-yet-measured) surface never triggers a release.
        assert!(!rotation_flips((0, 0), (1080, 2400)));
        assert!(!rotation_flips((1000, 1000), (2400, 1080)));
        // Identity.
        assert!(!rotation_flips((1080, 2400), (1080, 2400)));
    }

    #[test]
    fn policy_ids_are_stable_and_round_trip() {
        assert_eq!(
            OrientationPolicy::default(),
            OrientationPolicy::FollowSystem
        );
        assert_eq!(OrientationPolicy::FollowSystem.id(), "system");
        assert_eq!(OrientationPolicy::Landscape.id(), "landscape");
        assert_eq!(OrientationPolicy::Portrait.id(), "portrait");
        for p in OrientationPolicy::ALL {
            assert_eq!(OrientationPolicy::from_id(p.id()), Some(p));
        }
        assert_eq!(OrientationPolicy::from_id("upside-down"), None);
        // A corrupt settings value degrades to "follow system", never to a panic.
        assert_eq!(
            OrientationPolicy::from_id_or_default(""),
            OrientationPolicy::FollowSystem
        );
    }

    #[test]
    fn policy_json_matches_the_settings_id() {
        // The Kotlin `OrientationMode.id` is what the settings persist and what
        // the launch-options JSON carries: the two must be the same string, or a
        // forced orientation would silently degrade to "follow system" on the
        // way through the FFI.
        for p in OrientationPolicy::ALL {
            let json = serde_json::to_value(p).unwrap();
            assert_eq!(json, serde_json::Value::String(p.id().to_string()));
            let back: OrientationPolicy = serde_json::from_value(json).unwrap();
            assert_eq!(back, p);
        }
        // The pre-task-9 snake_case spelling still loads.
        let legacy: OrientationPolicy = serde_json::from_str("\"follow_system\"").unwrap();
        assert_eq!(legacy, OrientationPolicy::FollowSystem);
        // Anything else is a hard error at the serde layer (the lenient path is
        // `from_id_or_default`, used where a corrupt settings blob is expected).
        assert!(serde_json::from_str::<OrientationPolicy>("\"sideways\"").is_err());
    }

    #[test]
    fn policy_maps_onto_android_screen_orientation() {
        // `user` (not `unspecified`) so the device rotation lock is honoured.
        assert_eq!(
            OrientationPolicy::FollowSystem.android_screen_orientation(),
            "user"
        );
        // sensor* so both directions of the pinned axis work.
        assert_eq!(
            OrientationPolicy::Landscape.android_screen_orientation(),
            "sensorLandscape"
        );
        assert_eq!(
            OrientationPolicy::Portrait.android_screen_orientation(),
            "sensorPortrait"
        );
    }

    #[test]
    fn policy_allows_and_forces_the_right_orientations() {
        assert_eq!(OrientationPolicy::FollowSystem.forced(), None);
        assert!(OrientationPolicy::FollowSystem.allows(ScreenOrientation::Portrait));
        assert!(OrientationPolicy::FollowSystem.allows(ScreenOrientation::Landscape));

        assert_eq!(
            OrientationPolicy::Landscape.forced(),
            Some(ScreenOrientation::Landscape)
        );
        assert!(OrientationPolicy::Landscape.allows(ScreenOrientation::Landscape));
        assert!(!OrientationPolicy::Landscape.allows(ScreenOrientation::Portrait));
        // A square window is never "wrong".
        assert!(OrientationPolicy::Landscape.allows(ScreenOrientation::Square));
        assert!(!OrientationPolicy::Portrait.allows(ScreenOrientation::Landscape));
    }

    #[test]
    fn forcing_an_orientation_orients_the_game_window() {
        let landscape = win(1280, 720);
        let portrait = win(720, 1280);
        // Follow-system keeps whatever the user configured.
        assert_eq!(OrientationPolicy::FollowSystem.orient(landscape), landscape);
        assert_eq!(OrientationPolicy::FollowSystem.orient(portrait), portrait);
        // Forcing swaps the axes only when they disagree with the policy.
        assert_eq!(OrientationPolicy::Landscape.orient(portrait), landscape);
        assert_eq!(OrientationPolicy::Landscape.orient(landscape), landscape);
        assert_eq!(OrientationPolicy::Portrait.orient(landscape), portrait);
        assert_eq!(OrientationPolicy::Portrait.orient(portrait), portrait);
        // Square is untouched by either policy, and swapping is idempotent.
        let square = win(1024, 1024);
        assert_eq!(OrientationPolicy::Portrait.orient(square), square);
        assert_eq!(
            OrientationPolicy::Portrait.orient(OrientationPolicy::Portrait.orient(landscape)),
            portrait
        );
    }

    #[test]
    fn size_classes_sit_exactly_on_the_breakpoints() {
        assert_eq!(SizeClass::of_width_dp(599), SizeClass::Compact);
        assert_eq!(SizeClass::of_width_dp(600), SizeClass::Medium);
        assert_eq!(SizeClass::of_width_dp(839), SizeClass::Medium);
        assert_eq!(SizeClass::of_width_dp(840), SizeClass::Expanded);
        assert_eq!(SizeClass::of_height_dp(479), SizeClass::Compact);
        assert_eq!(SizeClass::of_height_dp(480), SizeClass::Medium);
        assert_eq!(SizeClass::of_height_dp(899), SizeClass::Medium);
        assert_eq!(SizeClass::of_height_dp(900), SizeClass::Expanded);
        for c in [SizeClass::Compact, SizeClass::Medium, SizeClass::Expanded] {
            assert_eq!(SizeClass::from_id(c.id()), Some(c));
        }
        assert_eq!(SizeClass::from_id("huge"), None);
        assert!(SizeClass::Compact < SizeClass::Medium);
    }

    #[test]
    fn metrics_are_sanitised_and_rotatable() {
        let m = WindowMetrics::new(0, 0);
        assert_eq!((m.width_dp, m.height_dp), (1, 1));
        let huge = WindowMetrics::new(u32::MAX, u32::MAX);
        assert_eq!(huge.width_dp, WindowMetrics::MAX_DP);
        let phone = WindowMetrics::new(392, 872);
        assert_eq!(phone.rotated(), WindowMetrics::new(872, 392));
        assert_eq!(phone.rotated().rotated(), phone);
        assert_eq!(WindowMetrics::default(), phone);
    }

    #[test]
    fn phone_portrait_uses_a_single_column_and_a_bottom_bar() {
        let m = WindowMetrics::new(392, 872);
        assert_eq!(m.orientation(), ScreenOrientation::Portrait);
        assert_eq!(m.width_class(), SizeClass::Compact);
        assert_eq!(m.height_class(), SizeClass::Medium);
        assert!(!m.is_landscape());
        assert!(!m.is_short());
        assert!(!m.uses_navigation_rail());
        assert_eq!(m.instance_columns(), 1);
        assert_eq!(m.settings_columns(), 1);
        assert_eq!(m.dashboard_columns(), 1);
        assert_eq!(m.content_padding_dp(), 16);
    }

    #[test]
    fn the_same_phone_in_landscape_switches_to_a_rail_and_more_columns() {
        // 392x872dp rotated is 872x392dp: wide enough to be *expanded*, which is
        // why a landscape phone can carry three instance cards per row, and short
        // enough that spending 80dp on a bottom bar would be wrong.
        let m = WindowMetrics::new(392, 872).rotated();
        assert_eq!(m.orientation(), ScreenOrientation::Landscape);
        assert_eq!(m.width_class(), SizeClass::Expanded);
        assert_eq!(m.height_class(), SizeClass::Compact);
        assert!(m.is_landscape());
        assert!(m.is_short());
        assert!(m.uses_navigation_rail());
        assert_eq!(m.instance_columns(), 3);
        assert_eq!(m.settings_columns(), 2);
        assert_eq!(m.dashboard_columns(), 2);

        // A smaller phone in landscape (640x360dp) lands in the medium band and
        // still gets the rail plus two columns.
        let small = WindowMetrics::new(640, 360);
        assert_eq!(small.width_class(), SizeClass::Medium);
        assert_eq!(small.height_class(), SizeClass::Compact);
        assert!(small.uses_navigation_rail());
        assert_eq!(small.instance_columns(), 2);
        assert_eq!(small.settings_columns(), 2);
        assert_eq!(small.content_padding_dp(), 20);
    }

    #[test]
    fn a_narrow_landscape_phone_still_gets_the_rail() {
        // 480dp-wide landscape window (small phone / split screen): compact
        // width but landscape, so the rail wins and two cards still fit.
        let m = WindowMetrics::new(480, 320);
        assert_eq!(m.width_class(), SizeClass::Compact);
        assert!(m.uses_navigation_rail());
        assert_eq!(m.instance_columns(), 2);
        // Settings rows are wide; keep them single-column when width is compact.
        assert_eq!(m.settings_columns(), 1);
    }

    #[test]
    fn tablets_expand_to_three_columns() {
        let m = WindowMetrics::new(1280, 800);
        assert_eq!(m.width_class(), SizeClass::Expanded);
        assert_eq!(m.height_class(), SizeClass::Medium);
        assert!(m.uses_navigation_rail());
        assert_eq!(m.instance_columns(), 3);
        assert_eq!(m.settings_columns(), 2);
        assert_eq!(m.content_padding_dp(), 24);
        assert_eq!(m.max_content_width_dp(), 1080);
        // Portrait tablet: wide enough for the rail, two instance columns.
        let portrait = m.rotated();
        assert_eq!(portrait.width_class(), SizeClass::Medium);
        assert!(portrait.uses_navigation_rail());
        assert_eq!(portrait.instance_columns(), 2);
        // Portrait medium keeps settings single-column (rows are wide).
        assert_eq!(portrait.settings_columns(), 1);
        assert_eq!(portrait.height_class(), SizeClass::Expanded);
    }

    #[test]
    fn every_metric_is_sane_across_the_whole_dp_range() {
        // Fuzz the breakpoints: no combination may produce a zero column count
        // or a padding that would push content off-screen.
        for w in [1u32, 100, 320, 599, 600, 720, 839, 840, 1024, 1600, 4000] {
            for h in [1u32, 100, 320, 479, 480, 640, 899, 900, 1200, 4000] {
                let m = WindowMetrics::new(w, h);
                assert!(m.instance_columns() >= 1 && m.instance_columns() <= 3);
                assert!(m.settings_columns() >= 1 && m.settings_columns() <= 2);
                assert!(m.dashboard_columns() >= 1 && m.dashboard_columns() <= 2);
                assert!(m.content_padding_dp() >= 16 && m.content_padding_dp() <= 24);
                assert!(m.max_content_width_dp() >= 720);
                // Landscape ⇒ rail, always.
                if m.is_landscape() {
                    assert!(m.uses_navigation_rail());
                }
                // The JSON snapshot must always be complete.
                let j = m.to_json();
                assert_eq!(j["width_dp"], w.clamp(1, WindowMetrics::MAX_DP));
                assert!(j["orientation"].is_string());
            }
        }
    }

    #[test]
    fn catalogue_json_is_ui_ready() {
        let arr = orientation_catalog_json();
        let items = arr.as_array().expect("array");
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["id"], "system");
        assert_eq!(items[0]["android_screen_orientation"], "user");
        assert_eq!(items[0]["forced"], serde_json::Value::Null);
        assert_eq!(items[1]["id"], "landscape");
        assert_eq!(items[1]["forced"], "landscape");
        assert_eq!(items[2]["forced"], "portrait");
    }
}
