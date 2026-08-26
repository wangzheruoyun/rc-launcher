//! AWT / Swing compatibility layer + Canvas rendering (task 18, "fakefx").
//!
//! Minecraft — and much more importantly everything *around* it (the Forge /
//! OptiFine installers, the Mojang splash screen, `JOptionPane` crash dialogs,
//! font metrics, `BufferedImage` skin handling) — expects a **desktop AWT** with
//! a real display. Android has no X11, no `libawt_xawt`, no window manager: the
//! moment such code touches `java.awt.Toolkit` the JVM dies with
//! `java.awt.AWTError: Can't connect to X11 window server`.
//!
//! FCL solves this with **caciocavallo** (`FCLCore/fakefx` + the
//! `caciocavallo` / `caciocavallo17` bundles): a re-implementation of the AWT
//! *peers* that renders every window into an off-screen ARGB image instead of a
//! native window. This module is the Rust counterpart, and it covers the whole
//! chain the feature needs:
//!
//! ```text
//!  ┌──────────── game JVM (Android) ─────────────┐        ┌──── this module ────┐
//!  │  Swing / AWT  →  caciocavallo CTC peers     │        │  AwtFrame::decode   │
//!  │        ↓ renders into an int[] ARGB screen  │ frames │        ↓            │
//!  │  cacio "managed screen"  →  bridge          ├───────▶│  AwtCanvas (double  │
//!  │        ▲ injects AWT events                 │◀───────┤  buffered + damage) │
//!  └─────────┼───────────────────────────────────┘ events │        ↓            │
//!            │                                            │  RGBA8888 for a     │
//!            └──── AwtEventRecord (32-byte records) ───────┤  Compose ImageBitmap│
//!                                                         └─────────────────────┘
//! ```
//!
//! | concern | type |
//! |---|---|
//! | which AWT backend a Java version needs | [`AwtBackend`] |
//! | are the cacio jars actually on disk | [`CacioBundle`] |
//! | are the AWT native libs on disk | [`AwtNativeSet`] |
//! | the JVM arguments that activate the bridge | [`AwtBridge::jvm_args`] |
//! | the pixel transport (partial frames, validation) | [`AwtFrame`] |
//! | the off-screen canvas Compose draws | [`AwtCanvas`] |
//! | letterboxing + touch→AWT coordinate mapping | [`Viewport`] |
//! | Compose gestures/keys → AWT events | [`AwtInputTranslator`] |
//!
//! **Robustness first.** Every input that crosses a process boundary (frame
//! headers, damage rectangles, pointer coordinates) is validated: a corrupt or
//! hostile frame yields an [`RcError`], never a panic and never an out-of-bounds
//! write. All geometry is computed in integers, so no NaN can leak into a blit.
//!
//! ## A note on "fakefx"
//!
//! FCL's `fakefx` package is really *two* things: (1) a re-implementation of the
//! JavaFX property/binding API, needed because FCL descends from HMCL (a JavaFX
//! desktop launcher) and (2) the AWT-on-Android adaptation. Our UI is Jetpack
//! Compose with `StateFlow`, which *is* the observable-property layer — so the
//! JavaFX half is intentionally not ported. What remains, and what this module
//! implements, is the AWT/Swing half plus the Canvas that surfaces it inside
//! Compose.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{RcError, RcResult};
use crate::launch::options::WindowSize;
use crate::launch::runtime_assets::{AppRuntime, CACIO_AGENT_JAR};
use crate::runtime::JavaVersion;

// ===========================================================================
// AWT wire constants (mirrors of the real `java.awt` values)
// ===========================================================================

/// `java.awt.event.*Event` ids, so the JVM-side bridge can `new MouseEvent(id,…)`
/// straight from a record without a translation table.
pub mod event_id {
    /// `KeyEvent.KEY_TYPED`
    pub const KEY_TYPED: i32 = 400;
    /// `KeyEvent.KEY_PRESSED`
    pub const KEY_PRESSED: i32 = 401;
    /// `KeyEvent.KEY_RELEASED`
    pub const KEY_RELEASED: i32 = 402;
    /// `MouseEvent.MOUSE_CLICKED`
    pub const MOUSE_CLICKED: i32 = 500;
    /// `MouseEvent.MOUSE_PRESSED`
    pub const MOUSE_PRESSED: i32 = 501;
    /// `MouseEvent.MOUSE_RELEASED`
    pub const MOUSE_RELEASED: i32 = 502;
    /// `MouseEvent.MOUSE_MOVED`
    pub const MOUSE_MOVED: i32 = 503;
    /// `MouseEvent.MOUSE_ENTERED`
    pub const MOUSE_ENTERED: i32 = 504;
    /// `MouseEvent.MOUSE_EXITED`
    pub const MOUSE_EXITED: i32 = 505;
    /// `MouseEvent.MOUSE_DRAGGED`
    pub const MOUSE_DRAGGED: i32 = 506;
    /// `MouseEvent.MOUSE_WHEEL`
    pub const MOUSE_WHEEL: i32 = 507;
    /// `ComponentEvent.COMPONENT_RESIZED` — used for a screen-size change.
    pub const COMPONENT_RESIZED: i32 = 101;
    /// `FocusEvent.FOCUS_GAINED`
    pub const FOCUS_GAINED: i32 = 1004;
    /// `FocusEvent.FOCUS_LOST`
    pub const FOCUS_LOST: i32 = 1005;
}

/// `java.awt.event.InputEvent` extended modifier masks (`*_DOWN_MASK`).
pub mod mask {
    /// `InputEvent.SHIFT_DOWN_MASK`
    pub const SHIFT_DOWN: i32 = 1 << 6;
    /// `InputEvent.CTRL_DOWN_MASK`
    pub const CTRL_DOWN: i32 = 1 << 7;
    /// `InputEvent.META_DOWN_MASK`
    pub const META_DOWN: i32 = 1 << 8;
    /// `InputEvent.ALT_DOWN_MASK`
    pub const ALT_DOWN: i32 = 1 << 9;
    /// `InputEvent.BUTTON1_DOWN_MASK` (left)
    pub const BUTTON1_DOWN: i32 = 1 << 10;
    /// `InputEvent.BUTTON2_DOWN_MASK` (middle)
    pub const BUTTON2_DOWN: i32 = 1 << 11;
    /// `InputEvent.BUTTON3_DOWN_MASK` (right)
    pub const BUTTON3_DOWN: i32 = 1 << 12;
    /// `InputEvent.ALT_GRAPH_DOWN_MASK`
    pub const ALT_GRAPH_DOWN: i32 = 1 << 13;
}

/// The `java.awt.event.KeyEvent.VK_*` codes we can synthesise.
///
/// Only the keys a phone can actually produce (a virtual keyboard, a mapped
/// on-screen button from task 15, or a Bluetooth keyboard) are listed; letters,
/// digits, function and numpad keys are computed in [`vk_for_key`].
pub mod vk {
    /// `VK_BACK_SPACE`
    pub const BACK_SPACE: i32 = 8;
    /// `VK_TAB`
    pub const TAB: i32 = 9;
    /// `VK_ENTER`
    pub const ENTER: i32 = 10;
    /// `VK_SHIFT`
    pub const SHIFT: i32 = 16;
    /// `VK_CONTROL`
    pub const CONTROL: i32 = 17;
    /// `VK_ALT`
    pub const ALT: i32 = 18;
    /// `VK_CAPS_LOCK`
    pub const CAPS_LOCK: i32 = 20;
    /// `VK_ESCAPE`
    pub const ESCAPE: i32 = 27;
    /// `VK_SPACE`
    pub const SPACE: i32 = 32;
    /// `VK_PAGE_UP`
    pub const PAGE_UP: i32 = 33;
    /// `VK_PAGE_DOWN`
    pub const PAGE_DOWN: i32 = 34;
    /// `VK_END`
    pub const END: i32 = 35;
    /// `VK_HOME`
    pub const HOME: i32 = 36;
    /// `VK_LEFT`
    pub const LEFT: i32 = 37;
    /// `VK_UP`
    pub const UP: i32 = 38;
    /// `VK_RIGHT`
    pub const RIGHT: i32 = 39;
    /// `VK_DOWN`
    pub const DOWN: i32 = 40;
    /// `VK_COMMA`
    pub const COMMA: i32 = 44;
    /// `VK_MINUS`
    pub const MINUS: i32 = 45;
    /// `VK_PERIOD`
    pub const PERIOD: i32 = 46;
    /// `VK_SLASH`
    pub const SLASH: i32 = 47;
    /// `VK_SEMICOLON`
    pub const SEMICOLON: i32 = 59;
    /// `VK_EQUALS`
    pub const EQUALS: i32 = 61;
    /// `VK_OPEN_BRACKET`
    pub const OPEN_BRACKET: i32 = 91;
    /// `VK_BACK_SLASH`
    pub const BACK_SLASH: i32 = 92;
    /// `VK_CLOSE_BRACKET`
    pub const CLOSE_BRACKET: i32 = 93;
    /// `VK_DELETE`
    pub const DELETE: i32 = 127;
    /// `VK_INSERT`
    pub const INSERT: i32 = 155;
    /// `VK_BACK_QUOTE` (grave accent)
    pub const BACK_QUOTE: i32 = 192;
    /// `VK_QUOTE` (apostrophe)
    pub const QUOTE: i32 = 222;
    /// `VK_META` (the "super"/"windows" key)
    pub const META: i32 = 157;
    /// `KeyEvent.CHAR_UNDEFINED`
    pub const CHAR_UNDEFINED: u32 = 0xFFFF;
}

/// Mouse buttons, in `java.awt.event.MouseEvent.BUTTON*` numbering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    /// `BUTTON1` — left click / a single-finger tap.
    Left,
    /// `BUTTON2` — middle click.
    Middle,
    /// `BUTTON3` — right click / a long press.
    Right,
}

impl MouseButton {
    /// The AWT button number (1 / 2 / 3).
    pub fn number(self) -> i32 {
        match self {
            MouseButton::Left => 1,
            MouseButton::Middle => 2,
            MouseButton::Right => 3,
        }
    }

    /// The matching `*_DOWN_MASK`.
    pub fn mask(self) -> i32 {
        match self {
            MouseButton::Left => mask::BUTTON1_DOWN,
            MouseButton::Middle => mask::BUTTON2_DOWN,
            MouseButton::Right => mask::BUTTON3_DOWN,
        }
    }

    /// Parse an AWT button number back into a [`MouseButton`].
    pub fn from_number(n: i32) -> Option<MouseButton> {
        match n {
            1 => Some(MouseButton::Left),
            2 => Some(MouseButton::Middle),
            3 => Some(MouseButton::Right),
            _ => None,
        }
    }
}

/// Translate a key *name* into a `KeyEvent.VK_*` code.
///
/// Accepts the Minecraft/GLFW-style names the task-15 control layout stores
/// (`key.keyboard.w`, `key.keyboard.left.shift`), bare names (`w`, `escape`,
/// `f3`), and `_`-separated variants (`LEFT_SHIFT`). Case-insensitive.
///
/// Returns `None` for names AWT has no code for, so callers can degrade to
/// [`AwtEvent::Text`] instead of injecting a bogus key.
pub fn vk_for_key(name: &str) -> Option<i32> {
    let n = name.trim().to_ascii_lowercase().replace('_', ".");
    let n = n.strip_prefix("key.keyboard.").unwrap_or(&n).to_string();
    // single letters / digits
    if n.len() == 1 {
        let c = n.as_bytes()[0];
        if c.is_ascii_lowercase() {
            return Some((c.to_ascii_uppercase()) as i32); // VK_A == 'A' == 65
        }
        if c.is_ascii_digit() {
            return Some(c as i32); // VK_0 == '0' == 48
        }
    }
    // function keys f1..f24
    if let Some(rest) = n.strip_prefix('f') {
        if let Ok(i) = rest.parse::<i32>() {
            if (1..=24).contains(&i) {
                return Some(111 + i); // VK_F1 == 112
            }
        }
    }
    // numpad
    if let Some(rest) = n.strip_prefix("keypad.") {
        if let Ok(i) = rest.parse::<i32>() {
            if (0..=9).contains(&i) {
                return Some(96 + i); // VK_NUMPAD0 == 96
            }
        }
        return match rest {
            "enter" => Some(vk::ENTER),
            "add" | "plus" => Some(107),       // VK_ADD
            "subtract" | "minus" => Some(109), // VK_SUBTRACT
            "multiply" => Some(106),           // VK_MULTIPLY
            "divide" => Some(111),             // VK_DIVIDE
            "decimal" => Some(110),            // VK_DECIMAL
            _ => None,
        };
    }
    Some(match n.as_str() {
        "escape" | "esc" => vk::ESCAPE,
        "space" => vk::SPACE,
        "enter" | "return" => vk::ENTER,
        "tab" => vk::TAB,
        "backspace" | "back.space" => vk::BACK_SPACE,
        "delete" => vk::DELETE,
        "insert" => vk::INSERT,
        "home" => vk::HOME,
        "end" => vk::END,
        "page.up" | "pageup" | "prior" => vk::PAGE_UP,
        "page.down" | "pagedown" | "next" => vk::PAGE_DOWN,
        "left" | "arrow.left" => vk::LEFT,
        "right" | "arrow.right" => vk::RIGHT,
        "up" | "arrow.up" => vk::UP,
        "down" | "arrow.down" => vk::DOWN,
        "shift" | "left.shift" | "shift.left" | "right.shift" | "shift.right" => vk::SHIFT,
        "control" | "ctrl" | "left.control" | "control.left" | "right.control"
        | "control.right" => vk::CONTROL,
        "alt" | "left.alt" | "alt.left" | "right.alt" | "alt.right" => vk::ALT,
        "super" | "meta" | "left.super" | "right.super" => vk::META,
        "caps.lock" | "capslock" => vk::CAPS_LOCK,
        "comma" => vk::COMMA,
        "period" | "dot" => vk::PERIOD,
        "slash" => vk::SLASH,
        "backslash" | "back.slash" => vk::BACK_SLASH,
        "semicolon" => vk::SEMICOLON,
        "equal" | "equals" => vk::EQUALS,
        "minus" => vk::MINUS,
        "left.bracket" | "open.bracket" => vk::OPEN_BRACKET,
        "right.bracket" | "close.bracket" => vk::CLOSE_BRACKET,
        "grave.accent" | "grave" | "back.quote" => vk::BACK_QUOTE,
        "apostrophe" | "quote" => vk::QUOTE,
        _ => return None,
    })
}

/// The `*_DOWN_MASK` a modifier key contributes while it is held.
pub fn modifier_mask_for_vk(code: i32) -> Option<i32> {
    match code {
        vk::SHIFT => Some(mask::SHIFT_DOWN),
        vk::CONTROL => Some(mask::CTRL_DOWN),
        vk::ALT => Some(mask::ALT_DOWN),
        vk::META => Some(mask::META_DOWN),
        _ => None,
    }
}

// ===========================================================================
// Backend selection & on-disk bundle validation
// ===========================================================================

/// Which AWT implementation the game JVM runs with.
///
/// The split follows FCL exactly: Java 8 uses the original caciocavallo build
/// (`cacio-androidnw`, peers + an Android native-window screen), Java 9+ uses the
/// modular `caciocavallo17` build (`cacio-tta` + a java agent that re-opens the
/// sealed `java.desktop` internals).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AwtBackend {
    /// No AWT bridge: `-Djava.awt.headless=true`. Vanilla Minecraft 1.13+ runs
    /// fine like this and it is the cheapest, most robust option.
    Headless,
    /// `app_runtime/caciocavallo/` — Java 8 (`net.java.openjdk.cacio.ctc.*`).
    Cacio8,
    /// `app_runtime/caciocavallo17/` — Java 9+ (`com.github.caciocavallosilano.cacio.ctc.*`).
    Cacio17,
}

impl AwtBackend {
    /// The backend a Java version needs (Java 8 → [`AwtBackend::Cacio8`], else
    /// [`AwtBackend::Cacio17`]).
    pub fn for_java(java: JavaVersion) -> AwtBackend {
        match java {
            JavaVersion::Java8 => AwtBackend::Cacio8,
            _ => AwtBackend::Cacio17,
        }
    }

    /// Stable id for settings / JSON / logs.
    pub fn id(self) -> &'static str {
        match self {
            AwtBackend::Headless => "headless",
            AwtBackend::Cacio8 => "cacio8",
            AwtBackend::Cacio17 => "cacio17",
        }
    }

    /// `-Dawt.toolkit=` value, or `None` when headless.
    pub fn toolkit(self) -> Option<&'static str> {
        match self {
            AwtBackend::Headless => None,
            AwtBackend::Cacio8 => Some(CACIO8_TOOLKIT),
            AwtBackend::Cacio17 => Some(CACIO17_TOOLKIT),
        }
    }

    /// `-Djava.awt.graphicsenv=` value, or `None` when headless.
    pub fn graphics_env(self) -> Option<&'static str> {
        match self {
            AwtBackend::Headless => None,
            AwtBackend::Cacio8 => Some(CACIO8_GRAPHICS_ENV),
            AwtBackend::Cacio17 => Some(CACIO17_GRAPHICS_ENV),
        }
    }

    /// `app_runtime/` sub-directory holding this backend's jars.
    pub fn bundle_dir_name(self) -> Option<&'static str> {
        match self {
            AwtBackend::Headless => None,
            AwtBackend::Cacio8 => Some(crate::launch::runtime_assets::CACIO_DIR),
            AwtBackend::Cacio17 => Some(crate::launch::runtime_assets::CACIO17_DIR),
        }
    }

    /// The artifacts this backend expects inside its bundle directory.
    pub fn artifacts(self) -> &'static [CacioArtifact] {
        match self {
            AwtBackend::Headless => &[],
            AwtBackend::Cacio8 => CACIO8_ARTIFACTS,
            AwtBackend::Cacio17 => CACIO17_ARTIFACTS,
        }
    }

    /// Whether the backend can render AWT windows (i.e. is not headless).
    pub fn is_graphical(self) -> bool {
        self != AwtBackend::Headless
    }
}

/// How a cacio jar has to be handed to the JVM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacioRole {
    /// Appended to `-cp` (the peers, the toolkit, the Android screen).
    Classpath,
    /// Prepended to the *boot* classpath (`-Xbootclasspath/p:`) — it patches
    /// classes the JRE itself loads, so the application classpath is too late.
    BootClasspath,
    /// Loaded with `-javaagent:` (opens the sealed `java.desktop` packages).
    Agent,
}

/// One artifact of a caciocavallo bundle, matched by file-name *prefix* because
/// the shipped jars carry version stamps (`cacio-shared-1.19.1-SNAPSHOT.jar`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacioArtifact {
    /// File-name prefix as shipped by FCL / Zalith.
    pub prefix: &'static str,
    /// Where the jar has to go.
    pub role: CacioRole,
    /// `false` for jars whose absence only degrades the bridge.
    pub required: bool,
    /// One-line explanation used in errors / diagnostics.
    pub purpose: &'static str,
}

/// Java-8 bundle (`assets/app_runtime/caciocavallo/`, from the FCL APK catalogue).
pub const CACIO8_ARTIFACTS: &[CacioArtifact] = &[
    CacioArtifact {
        prefix: "cacio-shared",
        role: CacioRole::Classpath,
        required: true,
        purpose: "AWT peer implementation (windows, graphics, focus, event pump)",
    },
    CacioArtifact {
        prefix: "cacio-androidnw",
        role: CacioRole::Classpath,
        required: true,
        purpose: "Android native-window screen: renders the AWT desktop into an ARGB buffer",
    },
    CacioArtifact {
        prefix: "ResConfHack",
        role: CacioRole::BootClasspath,
        required: false,
        purpose: "patches the AWT resource bundles the Android JRE ships without",
    },
];

/// Java-17+ bundle (`assets/app_runtime/caciocavallo17/`).
pub const CACIO17_ARTIFACTS: &[CacioArtifact] = &[
    CacioArtifact {
        prefix: "cacio-shared",
        role: CacioRole::Classpath,
        required: true,
        purpose: "AWT peer implementation (windows, graphics, focus, event pump)",
    },
    CacioArtifact {
        prefix: "cacio-tta",
        role: CacioRole::Classpath,
        required: true,
        purpose: "the CTC toolkit + off-screen graphics environment",
    },
    CacioArtifact {
        prefix: "cacio-agent",
        role: CacioRole::Agent,
        required: false,
        purpose: "java agent that opens the sealed java.desktop internals",
    },
];

/// A scanned caciocavallo bundle: which artifacts are on disk and which are not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacioBundle {
    /// Backend the bundle belongs to.
    pub backend: AwtBackend,
    /// `app_runtime/caciocavallo{,17}/`.
    pub dir: PathBuf,
    /// Artifacts found on disk, paired with their resolved path.
    pub present: Vec<(CacioArtifact, PathBuf)>,
    /// Expected artifacts that are missing (optional ones included).
    pub missing: Vec<CacioArtifact>,
}

impl CacioBundle {
    /// Scan the bundle directory. Never fails — a missing directory simply means
    /// "everything is missing", which the caller grades via
    /// [`CacioBundle::is_usable`] or [`CacioBundle::discover`].
    pub fn scan(app_runtime: &AppRuntime, backend: AwtBackend) -> Self {
        let dir = match backend.bundle_dir_name() {
            Some(name) => app_runtime.root().join(name),
            None => app_runtime.root().to_path_buf(),
        };
        let jars = list_jars_sorted(&dir);
        let mut present = Vec::new();
        let mut missing = Vec::new();
        for artifact in backend.artifacts() {
            match jars.iter().find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(artifact.prefix))
                    .unwrap_or(false)
            }) {
                Some(p) => present.push((*artifact, p.clone())),
                None => missing.push(*artifact),
            }
        }
        Self {
            backend,
            dir,
            present,
            missing,
        }
    }

    /// Scan and *require* every mandatory artifact, returning an actionable
    /// error listing what to re-extract. Used by the launch preflight so a
    /// half-extracted bundle fails before a JVM is spawned.
    pub fn discover(app_runtime: &AppRuntime, backend: AwtBackend) -> RcResult<Self> {
        let bundle = Self::scan(app_runtime, backend);
        let required: Vec<&CacioArtifact> = bundle.missing.iter().filter(|a| a.required).collect();
        if !required.is_empty() {
            let names: Vec<String> = required
                .iter()
                .map(|a| format!("{}*.jar ({})", a.prefix, a.purpose))
                .collect();
            return Err(RcError::MissingFile(format!(
                "incomplete AWT bridge ({}) in {}: missing {} — re-extract the \
                 app_runtime/{} bundle or disable the AWT bridge",
                backend.id(),
                bundle.dir.display(),
                names.join(", "),
                backend.bundle_dir_name().unwrap_or("caciocavallo"),
            )));
        }
        Ok(bundle)
    }

    /// `true` when every *required* artifact is present.
    pub fn is_usable(&self) -> bool {
        !self.missing.iter().any(|a| a.required)
    }

    /// `true` when even the optional artifacts are present.
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }

    /// Jars for `-cp`, in bundle order (deterministic).
    pub fn classpath_jars(&self) -> Vec<PathBuf> {
        self.paths_with_role(CacioRole::Classpath)
    }

    /// Jars for `-Xbootclasspath/p:` (Java 8 `ResConfHack.jar`).
    pub fn boot_classpath_jars(&self) -> Vec<PathBuf> {
        self.paths_with_role(CacioRole::BootClasspath)
    }

    /// The `-javaagent:` jar, when the bundle ships one.
    pub fn agent_jar(&self) -> Option<PathBuf> {
        self.paths_with_role(CacioRole::Agent).into_iter().next()
    }

    fn paths_with_role(&self, role: CacioRole) -> Vec<PathBuf> {
        self.present
            .iter()
            .filter(|(a, _)| a.role == role)
            .map(|(_, p)| p.clone())
            .collect()
    }

    /// A one-line diagnostic summary for the launch log header.
    pub fn describe(&self) -> String {
        format!(
            "awt={} dir={} present={} missing={}",
            self.backend.id(),
            self.dir.display(),
            self.present.len(),
            self.missing.len()
        )
    }
}

/// Sorted `*.jar` listing that tolerates a missing directory (mirrors
/// `runtime_assets::list_jars`, kept private there).
fn list_jars_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .map(|e| e.eq_ignore_ascii_case("jar"))
                    .unwrap_or(false)
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

// ===========================================================================
// AWT native libraries
// ===========================================================================

/// A native library the AWT stack needs on Android.
///
/// Taken from the FCL APK (`FCL_NATIVE_LIBRARIES.md`): `libawt_headless.so` /
/// `libawt_xawt.so` are the JRE's AWT back-ends (the `xawt` one is FCL/Zalith's
/// *fake* X11 build — `xawt_fake.c` — whose only job is to let `java.awt` link),
/// and `libpojavexec_awt.so` is the frame bridge that publishes the off-screen
/// desktop to the Android side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwtNativeLib {
    /// `lib*.so` file name.
    pub file_name: &'static str,
    /// `true` when AWT cannot initialise without it.
    pub required: bool,
    /// What it is for (used in diagnostics).
    pub purpose: &'static str,
}

/// The AWT natives an Android launcher ships or expects inside the JRE.
pub const AWT_NATIVES: &[AwtNativeLib] = &[
    AwtNativeLib {
        file_name: "libawt.so",
        required: true,
        purpose: "core java.awt native support (part of the JRE)",
    },
    AwtNativeLib {
        file_name: "libawt_headless.so",
        required: false,
        purpose: "headless AWT back-end (used when the bridge is disabled)",
    },
    AwtNativeLib {
        file_name: "libawt_xawt.so",
        required: false,
        purpose: "fake X11 AWT back-end: lets java.desktop link without an X server",
    },
    AwtNativeLib {
        file_name: "libpojavexec_awt.so",
        required: false,
        purpose: "frame bridge that publishes the off-screen AWT desktop to Android",
    },
];

/// The result of scanning the native-library search path for [`AWT_NATIVES`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwtNativeSet {
    /// Directories that were scanned, in search order.
    pub search_dirs: Vec<PathBuf>,
    /// Libraries found, with the path they were found at.
    pub present: Vec<(AwtNativeLib, PathBuf)>,
    /// Libraries not found anywhere.
    pub missing: Vec<AwtNativeLib>,
}

impl AwtNativeSet {
    /// Scan `search_dirs` (typically `nativeLibraryDir` + `<jre>/lib` +
    /// `<jre>/lib/<abi>`) for the AWT natives. Never fails.
    pub fn scan<P: AsRef<Path>>(search_dirs: &[P]) -> Self {
        let dirs: Vec<PathBuf> = search_dirs
            .iter()
            .map(|p| p.as_ref().to_path_buf())
            .collect();
        let mut present = Vec::new();
        let mut missing = Vec::new();
        for lib in AWT_NATIVES {
            match dirs
                .iter()
                .map(|d| d.join(lib.file_name))
                .find(|p| p.is_file())
            {
                Some(p) => present.push((*lib, p)),
                None => missing.push(*lib),
            }
        }
        Self {
            search_dirs: dirs,
            present,
            missing,
        }
    }

    /// `true` when every *required* native was found.
    pub fn is_usable(&self) -> bool {
        !self.missing.iter().any(|l| l.required)
    }

    /// Whether a specific library was found.
    pub fn has(&self, file_name: &str) -> bool {
        self.present.iter().any(|(l, _)| l.file_name == file_name)
    }

    /// Human-readable warnings about what is missing (never an error: the JRE
    /// may satisfy AWT differently, and a missing *optional* native only means a
    /// degraded bridge).
    pub fn warnings(&self) -> Vec<String> {
        self.missing
            .iter()
            .map(|l| {
                format!(
                    "AWT native {} not found ({}){}",
                    l.file_name,
                    l.purpose,
                    if l.required {
                        ": AWT windows will fail to initialise"
                    } else {
                        ""
                    }
                )
            })
            .collect()
    }
}

// ===========================================================================
// The bridge: the JVM arguments that activate AWT on Android
// ===========================================================================

/// Java-8 caciocavallo AWT toolkit (`app_runtime/caciocavallo/`).
pub const CACIO8_TOOLKIT: &str = "net.java.openjdk.cacio.ctc.CTCToolkit";
/// Java-8 caciocavallo graphics environment.
pub const CACIO8_GRAPHICS_ENV: &str = "net.java.openjdk.cacio.ctc.CTCGraphicsEnvironment";
/// Java-17+ caciocavallo AWT toolkit (`app_runtime/caciocavallo17/`).
pub const CACIO17_TOOLKIT: &str = "com.github.caciocavallosilano.cacio.ctc.CTCToolkit";
/// Java-17+ caciocavallo graphics environment.
pub const CACIO17_GRAPHICS_ENV: &str =
    "com.github.caciocavallosilano.cacio.ctc.CTCGraphicsEnvironment";

/// `--add-exports` / `--add-opens` flags the Java-17+ caciocavallo build needs.
///
/// caciocavallo re-implements the AWT peers, which live in `java.desktop`
/// internals that the module system seals off since Java 9. Without these the
/// bridge dies with `IllegalAccessError` the first time the game touches AWT
/// (Forge installers, the Mojang splash screen, font metrics, ...).
pub const CACIO17_MODULE_FLAGS: &[&str] = &[
    "--add-exports=java.base/sun.security.action=ALL-UNNAMED",
    "--add-exports=java.desktop/java.awt=ALL-UNNAMED",
    "--add-exports=java.desktop/java.awt.dnd.peer=ALL-UNNAMED",
    "--add-exports=java.desktop/java.awt.peer=ALL-UNNAMED",
    "--add-exports=java.desktop/sun.awt=ALL-UNNAMED",
    "--add-exports=java.desktop/sun.awt.dnd=ALL-UNNAMED",
    "--add-exports=java.desktop/sun.awt.image=ALL-UNNAMED",
    "--add-exports=java.desktop/sun.font=ALL-UNNAMED",
    "--add-exports=java.desktop/sun.java2d=ALL-UNNAMED",
    "--add-opens=java.base/java.lang.reflect=ALL-UNNAMED",
    "--add-opens=java.base/java.util=ALL-UNNAMED",
    "--add-opens=java.desktop/sun.font=ALL-UNNAMED",
];

/// Java-8 cacio font plumbing: the Android JRE has no fontconfig, so cacio is
/// told which font manager / scaler to instantiate (same values FCL uses).
pub const CACIO8_FONT_MANAGER: &str = "sun.awt.X11FontManager";
/// Java-8 cacio font scaler (FreeType, which the Android JRE does ship).
pub const CACIO8_FONT_SCALER: &str = "sun.font.FreetypeFontScaler";
/// Swing look-and-feel that has no native dependencies at all.
pub const SWING_LAF: &str = "javax.swing.plaf.metal.MetalLookAndFeel";

// ===========================================================================
// JVM-side transport: where the frames and events actually flow
// ===========================================================================

/// Wire-protocol id handed to the JVM side, so a bridge built against another
/// revision refuses the channel instead of misinterpreting bytes: `"rcaf1"` =
/// [`AwtFrame`] frames (`"RCAF"` header) + 32-byte [`AwtEventRecord`] batches.
pub const AWT_TRANSPORT_PROTOCOL: &str = "rcaf1";
/// System property naming the frame channel (JVM writes, launcher reads).
pub const AWT_PROP_FRAMES: &str = "rc.awt.bridge.frames";
/// System property naming the event channel (launcher writes, JVM reads).
pub const AWT_PROP_EVENTS: &str = "rc.awt.bridge.events";
/// System property naming the wire protocol.
pub const AWT_PROP_PROTOCOL: &str = "rc.awt.bridge.protocol";
/// Default file name of the frame channel inside a transport directory.
pub const AWT_FRAMES_CHANNEL: &str = "awt-frames.rcaf";
/// Default file name of the event channel inside a transport directory.
pub const AWT_EVENTS_CHANNEL: &str = "awt-events.rcae";

/// The two channels (named pipes on Android) that carry the live AWT session
/// between the game JVM and the launcher.
///
/// ```text
///   frames : JVM  --[AwtFrame]-------->  launcher  (AwtSession::submit_frame)
///   events : JVM  <--[AwtEventRecord]--  launcher  (AwtSession::drain_events)
/// ```
///
/// Both paths are passed to the JVM as system properties ([`AwtTransport::jvm_args`]),
/// which is all the JVM-side bridge (the caciocavallo hook / agent) needs to find
/// them. Nothing here opens a file: the launcher side is created and pumped by
/// [`crate::launch::awt_host`], so a launch that never opens a session simply
/// never advertises the properties (a FIFO with no reader would block the JVM).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwtTransport {
    /// Frame channel (JVM -> launcher).
    pub frames: PathBuf,
    /// Event channel (launcher -> JVM).
    pub events: PathBuf,
}

impl AwtTransport {
    /// Explicit channel paths.
    pub fn new(frames: impl Into<PathBuf>, events: impl Into<PathBuf>) -> Self {
        Self {
            frames: frames.into(),
            events: events.into(),
        }
    }

    /// The conventional pair inside `dir` ([`AWT_FRAMES_CHANNEL`] /
    /// [`AWT_EVENTS_CHANNEL`]) — what the launcher creates per game session.
    pub fn in_dir<P: AsRef<Path>>(dir: P) -> Self {
        let dir = dir.as_ref();
        Self {
            frames: dir.join(AWT_FRAMES_CHANNEL),
            events: dir.join(AWT_EVENTS_CHANNEL),
        }
    }

    /// The directory holding the channels (when both share one).
    pub fn dir(&self) -> Option<&Path> {
        self.frames.parent()
    }

    /// The `-D` properties that tell the JVM-side bridge where to write/read.
    pub fn jvm_args(&self) -> Vec<String> {
        vec![
            format!("-D{AWT_PROP_PROTOCOL}={AWT_TRANSPORT_PROTOCOL}"),
            format!("-D{AWT_PROP_FRAMES}={}", self.frames.to_string_lossy()),
            format!("-D{AWT_PROP_EVENTS}={}", self.events.to_string_lossy()),
        ]
    }

    /// JSON snapshot (FFI / diagnostics).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "protocol": AWT_TRANSPORT_PROTOCOL,
            "frames": self.frames.to_string_lossy(),
            "events": self.events.to_string_lossy(),
        })
    }
}

/// The complete, validated AWT-on-Android setup for one launch.
///
/// Combines *what* backend to use, *whether its jars/natives are on disk* and
/// the *JVM arguments* that activate it. The launch engine builds one during
/// preflight; [`crate::launch::CommandBuilder`] asks it for the arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwtBridge {
    /// The selected backend.
    pub backend: AwtBackend,
    /// Size of the virtual AWT desktop (`-Dcacio.managed.screensize`).
    pub screen: WindowSize,
    /// The scanned cacio bundle (`None` when headless / no `app_runtime`).
    pub bundle: Option<CacioBundle>,
    /// The scanned AWT natives (`None` when not scanned).
    pub natives: Option<AwtNativeSet>,
    /// The live session channels, when the launcher hosts one (task 18). `None`
    /// keeps the bridge purely off-line: cacio still renders into its own ARGB
    /// screen, but nothing is published to the Android side.
    pub transport: Option<AwtTransport>,
}

impl AwtBridge {
    /// A bridge that keeps AWT headless (no cacio, no canvas).
    pub fn headless() -> Self {
        Self {
            backend: AwtBackend::Headless,
            screen: WindowSize::default(),
            bundle: None,
            natives: None,
            transport: None,
        }
    }

    /// A bridge for an explicit backend and virtual screen size.
    pub fn new(backend: AwtBackend, screen: WindowSize) -> Self {
        Self {
            backend,
            screen,
            bundle: None,
            natives: None,
            transport: None,
        }
    }

    /// The backend a Java version needs, with the given virtual screen size.
    pub fn for_java(java: JavaVersion, screen: WindowSize) -> Self {
        Self::new(AwtBackend::for_java(java), screen)
    }

    /// Attach an already scanned bundle.
    pub fn with_bundle(mut self, bundle: CacioBundle) -> Self {
        self.bundle = Some(bundle);
        self
    }

    /// Attach an already scanned native set.
    pub fn with_natives(mut self, natives: AwtNativeSet) -> Self {
        self.natives = Some(natives);
        self
    }

    /// Advertise the live session channels to the JVM (task 18).
    ///
    /// Only meaningful for a *graphical* backend and only when the launcher has
    /// actually opened + pumped the channels
    /// ([`crate::launch::awt_host::AwtHost::attach_transport`]); a channel with
    /// no reader on the launcher side would block the JVM's first repaint.
    pub fn with_transport(mut self, transport: AwtTransport) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Scan `app_runtime/` + the native search dirs and build the bridge (I/O).
    ///
    /// Never fails: a missing bundle shows up in [`AwtBridge::warnings`] and in
    /// [`CacioBundle::is_usable`]; use [`AwtBridge::discover`] when the launch
    /// must abort instead.
    pub fn scan(
        java: JavaVersion,
        screen: WindowSize,
        app_runtime: Option<&AppRuntime>,
        native_search_dirs: &[PathBuf],
    ) -> Self {
        let mut bridge = Self::for_java(java, screen);
        if let Some(rt) = app_runtime {
            bridge.bundle = Some(CacioBundle::scan(rt, bridge.backend));
        }
        if !native_search_dirs.is_empty() {
            bridge.natives = Some(AwtNativeSet::scan(native_search_dirs));
        }
        bridge
    }

    /// Like [`AwtBridge::scan`] but *requires* the cacio bundle to be complete
    /// (launch preflight: fail before spawning a JVM that would die on the first
    /// `Toolkit.getDefaultToolkit()`).
    pub fn discover(
        java: JavaVersion,
        screen: WindowSize,
        app_runtime: &AppRuntime,
        native_search_dirs: &[PathBuf],
    ) -> RcResult<Self> {
        let backend = AwtBackend::for_java(java);
        let bundle = CacioBundle::discover(app_runtime, backend)?;
        let mut bridge = Self::new(backend, screen).with_bundle(bundle);
        if !native_search_dirs.is_empty() {
            bridge.natives = Some(AwtNativeSet::scan(native_search_dirs));
        }
        Ok(bridge)
    }

    /// Jars that must join the application classpath.
    pub fn classpath_jars(&self) -> Vec<PathBuf> {
        match (&self.bundle, self.backend.is_graphical()) {
            (Some(b), true) => b.classpath_jars(),
            _ => Vec::new(),
        }
    }

    /// `-Xbootclasspath/p:<jars>` when the bundle ships boot-classpath patches
    /// (Java 8 `ResConfHack.jar`).
    pub fn boot_classpath_arg(&self) -> Option<String> {
        let jars = self.bundle.as_ref()?.boot_classpath_jars();
        if jars.is_empty() || !self.backend.is_graphical() {
            return None;
        }
        let joined = jars
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(crate::launch::env::PATH_SEP);
        Some(format!("-Xbootclasspath/p:{joined}"))
    }

    /// `-javaagent:<cacio-agent.jar>` when the bundle ships the agent.
    pub fn agent_arg(&self) -> Option<String> {
        if !self.backend.is_graphical() {
            return None;
        }
        let agent = self.bundle.as_ref()?.agent_jar()?;
        Some(format!("-javaagent:{}", agent.to_string_lossy()))
    }

    /// The JVM arguments that switch AWT to the off-screen bridge.
    ///
    /// Anything the bridge *wanted* but could not find is appended to `notes`
    /// (and surfaced to the UI) rather than failing the launch: a missing
    /// `cacio-agent.jar` degrades AWT, it does not break the game itself.
    pub fn jvm_args(&self, notes: &mut Vec<String>) -> Vec<String> {
        let mut args = Vec::new();
        if !self.backend.is_graphical() {
            args.push("-Djava.awt.headless=true".to_string());
            return args;
        }
        // 1. AWT must not be headless, and cacio needs to know how big the
        //    virtual desktop is (it allocates the ARGB screen from this).
        args.push("-Djava.awt.headless=false".to_string());
        args.push(format!(
            "-Dcacio.managed.screensize={}",
            self.screen.as_screen_size()
        ));
        // 2. A pure-Java look-and-feel: every native LAF needs a real desktop.
        args.push(format!("-Dswing.defaultlaf={SWING_LAF}"));
        // 3. Replace the AWT toolkit + graphics environment with cacio's.
        if let Some(toolkit) = self.backend.toolkit() {
            args.push(format!("-Dawt.toolkit={toolkit}"));
        }
        if let Some(genv) = self.backend.graphics_env() {
            args.push(format!("-Djava.awt.graphicsenv={genv}"));
        }
        // 4. Java2D must stay on the software pipeline: there is no desktop GL
        //    (and the GL4ES/ANGLE translation layer belongs to LWJGL, not AWT).
        args.push("-Dsun.java2d.opengl=false".to_string());
        match self.backend {
            AwtBackend::Cacio8 => {
                // Java 8 has no fontconfig on Android: name the manager/scaler.
                args.push(format!("-Dcacio.font.fontmanager={CACIO8_FONT_MANAGER}"));
                args.push(format!("-Dcacio.font.fontscaler={CACIO8_FONT_SCALER}"));
                match self.boot_classpath_arg() {
                    Some(arg) => args.push(arg),
                    None => notes.push(
                        "caciocavallo ResConfHack.jar not found: AWT resource bundles may be \
                         missing (dialog labels can render empty)"
                            .to_string(),
                    ),
                }
            }
            AwtBackend::Cacio17 => {
                // Java 9+ seals java.desktop: re-open what cacio reaches into.
                for flag in CACIO17_MODULE_FLAGS {
                    args.push((*flag).to_string());
                }
                match self.agent_arg() {
                    Some(arg) => args.push(arg),
                    None => notes.push(format!(
                        "caciocavallo17 agent ({CACIO_AGENT_JAR}) not found: AWT may be unavailable"
                    )),
                }
            }
            AwtBackend::Headless => unreachable!("handled above"),
        }
        // 5. Where the live session lives, when the launcher hosts one. Without
        //    a transport cacio still paints into its own ARGB screen (dialogs
        //    work, nothing is displayed); with one, every repaint reaches the
        //    Compose canvas and every touch reaches the AWT event queue.
        if let Some(t) = &self.transport {
            args.extend(t.jvm_args());
        }
        notes.extend(self.warnings());
        args
    }

    /// Non-fatal problems with this setup (missing optional jars / natives).
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.backend.is_graphical() {
            return out;
        }
        if let Some(b) = &self.bundle {
            for missing in b.missing.iter().filter(|a| !a.required) {
                out.push(format!(
                    "AWT bridge: optional {}*.jar missing ({})",
                    missing.prefix, missing.purpose
                ));
            }
        }
        if let Some(n) = &self.natives {
            out.extend(n.warnings());
        }
        out
    }

    /// A canvas sized to the virtual AWT desktop (what Compose will draw).
    pub fn canvas(&self) -> RcResult<AwtCanvas> {
        AwtCanvas::new(self.screen.width, self.screen.height)
    }

    /// One-line summary for the launch log header.
    pub fn describe(&self) -> String {
        format!(
            "awt backend={} screen={} bundle={} natives={}",
            self.backend.id(),
            self.screen.as_screen_size(),
            self.bundle
                .as_ref()
                .map(|b| if b.is_complete() {
                    "complete".to_string()
                } else if b.is_usable() {
                    format!("usable ({} optional missing)", b.missing.len())
                } else {
                    format!("INCOMPLETE ({} required missing)", b.missing.len())
                })
                .unwrap_or_else(|| "not scanned".to_string()),
            self.natives
                .as_ref()
                .map(|n| format!("{}/{}", n.present.len(), AWT_NATIVES.len()))
                .unwrap_or_else(|| "not scanned".to_string()),
        )
    }

    /// JSON snapshot for the FFI layer / a settings screen.
    pub fn to_json(&self) -> serde_json::Value {
        let mut notes = Vec::new();
        let args = self.jvm_args(&mut notes);
        serde_json::json!({
            "backend": self.backend.id(),
            "graphical": self.backend.is_graphical(),
            "toolkit": self.backend.toolkit(),
            "graphics_env": self.backend.graphics_env(),
            "screen": { "width": self.screen.width, "height": self.screen.height },
            "jvm_args": args,
            "classpath_jars": self.classpath_jars()
                .iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>(),
            "bundle_dir": self.bundle.as_ref().map(|b| b.dir.to_string_lossy().to_string()),
            "usable": self.bundle.as_ref().map(|b| b.is_usable()),
            "transport": self.transport.as_ref().map(|t| t.to_json()),
            "notes": notes,
        })
    }
}

// ===========================================================================
// Canvas rendering: geometry
// ===========================================================================

/// Hard upper bound for a canvas edge. Guards against a corrupt frame header
/// asking us to allocate gigabytes (`8192 * 8192 * 4 B` = 256 MiB is already far
/// beyond any phone screen; anything larger is a bug or an attack).
pub const MAX_CANVAS_DIM: u32 = 8192;

/// An axis-aligned, non-negative rectangle in canvas pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width in pixels (`0` = empty).
    pub width: u32,
    /// Height in pixels (`0` = empty).
    pub height: u32,
}

impl Rect {
    /// A rectangle at `(x, y)` sized `width × height`.
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The rectangle covering a whole `width × height` surface.
    pub const fn whole(width: u32, height: u32) -> Self {
        Self::new(0, 0, width, height)
    }

    /// `true` when the rectangle has no area.
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Right edge (exclusive), saturating.
    pub const fn right(&self) -> u32 {
        self.x.saturating_add(self.width)
    }

    /// Bottom edge (exclusive), saturating.
    pub const fn bottom(&self) -> u32 {
        self.y.saturating_add(self.height)
    }

    /// Pixel count (as `u64`, so a huge rectangle cannot overflow).
    pub const fn area(&self) -> u64 {
        (self.width as u64) * (self.height as u64)
    }

    /// The smallest rectangle containing both (empty operands are ignored).
    pub fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Rect::new(x, y, right - x, bottom - y)
    }

    /// The overlapping part of both rectangles (empty when they do not overlap).
    pub fn intersect(self, other: Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= x || bottom <= y {
            Rect::default()
        } else {
            Rect::new(x, y, right - x, bottom - y)
        }
    }

    /// Clip to a `width × height` surface.
    pub fn clamp_to(self, width: u32, height: u32) -> Rect {
        self.intersect(Rect::whole(width, height))
    }

    /// `true` when `self` fits completely inside a `width × height` surface.
    pub fn fits_in(&self, width: u32, height: u32) -> bool {
        self.right() <= width && self.bottom() <= height
    }
}

/// Accumulated damage ("dirty region") as a single coalesced rectangle.
///
/// A rectangle list would upload less, but on Android the frame ends up in one
/// `Bitmap` anyway, so a coalesced bounding box is both cheaper to maintain and
/// what the upload path can actually exploit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Damage {
    rect: Rect,
}

impl Damage {
    /// No damage.
    pub const fn none() -> Self {
        Self {
            rect: Rect::new(0, 0, 0, 0),
        }
    }

    /// Everything is damaged.
    pub const fn full(width: u32, height: u32) -> Self {
        Self {
            rect: Rect::whole(width, height),
        }
    }

    /// Merge a damaged rectangle in.
    pub fn add(&mut self, rect: Rect) {
        self.rect = self.rect.union(rect);
    }

    /// `true` when nothing is damaged.
    pub const fn is_empty(&self) -> bool {
        self.rect.is_empty()
    }

    /// The coalesced damage rectangle (`None` when clean).
    pub fn rect(&self) -> Option<Rect> {
        (!self.is_empty()).then_some(self.rect)
    }

    /// Take the damage and reset to clean (consumer side).
    pub fn take(&mut self) -> Option<Rect> {
        let r = self.rect();
        self.rect = Rect::default();
        r
    }

    /// Reset to clean.
    pub fn clear(&mut self) {
        self.rect = Rect::default();
    }
}

// ===========================================================================
// Canvas rendering: the frame transport
// ===========================================================================

/// Pixel layout of a frame payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    /// `BufferedImage.TYPE_INT_ARGB` — what cacio's managed screen hands out.
    IntArgb,
    /// `BufferedImage.TYPE_INT_RGB` — no alpha channel; treated as opaque.
    IntRgb,
}

impl PixelFormat {
    /// Wire code.
    pub fn code(self) -> u16 {
        match self {
            PixelFormat::IntArgb => 0,
            PixelFormat::IntRgb => 1,
        }
    }

    /// Parse a wire code.
    pub fn from_code(code: u16) -> Option<PixelFormat> {
        match code {
            0 => Some(PixelFormat::IntArgb),
            1 => Some(PixelFormat::IntRgb),
            _ => None,
        }
    }
}

/// `"RCAF"` — RC launcher AWT frame.
pub const FRAME_MAGIC: u32 = 0x5243_4146;
/// Current frame wire version.
pub const FRAME_VERSION: u16 = 1;
/// Size of the fixed frame header in bytes.
pub const FRAME_HEADER_LEN: usize = 32;

/// One frame produced by the JVM-side AWT bridge.
///
/// The payload holds **only the damaged rectangle**, tightly packed row-major —
/// a Swing caret blink then costs a handful of pixels instead of a full 1280×720
/// upload. The wire format is a fixed 32-byte little-endian header:
///
/// ```text
/// 0  u32 magic ("RCAF")  4  u16 version  6  u16 format
/// 8  u32 seq            12  u16 width    14 u16 height
/// 16 u16 damage.x       18  u16 damage.y 20 u16 damage.w  22 u16 damage.h
/// 24 u32 payload_len (bytes)             28 u32 flags (bit0 = full frame)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwtFrame {
    /// Monotonic frame counter from the producer (used for drop detection).
    pub seq: u32,
    /// Full width of the virtual AWT desktop this frame belongs to.
    pub width: u32,
    /// Full height of the virtual AWT desktop this frame belongs to.
    pub height: u32,
    /// Payload layout.
    pub format: PixelFormat,
    /// The region the payload covers.
    pub damage: Rect,
    /// `damage.width * damage.height` pixels, row-major.
    pub pixels: Vec<u32>,
}

impl AwtFrame {
    /// A full-screen frame.
    pub fn full(seq: u32, width: u32, height: u32, pixels: Vec<u32>) -> RcResult<Self> {
        Self::partial(seq, width, height, Rect::whole(width, height), pixels)
    }

    /// A partial (damage-only) frame.
    pub fn partial(
        seq: u32,
        width: u32,
        height: u32,
        damage: Rect,
        pixels: Vec<u32>,
    ) -> RcResult<Self> {
        validate_dims(width, height)?;
        if damage.is_empty() {
            return Err(RcError::Launch(
                "AWT frame has an empty damage rectangle".to_string(),
            ));
        }
        if !damage.fits_in(width, height) {
            return Err(RcError::Launch(format!(
                "AWT frame damage {}x{}+{}+{} exceeds the {}x{} screen",
                damage.width, damage.height, damage.x, damage.y, width, height
            )));
        }
        if pixels.len() as u64 != damage.area() {
            return Err(RcError::Launch(format!(
                "AWT frame payload has {} pixels, expected {}",
                pixels.len(),
                damage.area()
            )));
        }
        Ok(Self {
            seq,
            width,
            height,
            format: PixelFormat::IntArgb,
            damage,
            pixels,
        })
    }

    /// Override the payload layout (defaults to [`PixelFormat::IntArgb`]).
    pub fn with_format(mut self, format: PixelFormat) -> Self {
        self.format = format;
        self
    }

    /// `true` when the damage covers the whole screen.
    pub fn is_full(&self) -> bool {
        self.damage == Rect::whole(self.width, self.height)
    }

    /// Serialise header + payload.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(FRAME_HEADER_LEN + self.pixels.len() * 4);
        out.extend_from_slice(&FRAME_MAGIC.to_le_bytes());
        out.extend_from_slice(&FRAME_VERSION.to_le_bytes());
        out.extend_from_slice(&self.format.code().to_le_bytes());
        out.extend_from_slice(&self.seq.to_le_bytes());
        out.extend_from_slice(&(self.width as u16).to_le_bytes());
        out.extend_from_slice(&(self.height as u16).to_le_bytes());
        out.extend_from_slice(&(self.damage.x as u16).to_le_bytes());
        out.extend_from_slice(&(self.damage.y as u16).to_le_bytes());
        out.extend_from_slice(&(self.damage.width as u16).to_le_bytes());
        out.extend_from_slice(&(self.damage.height as u16).to_le_bytes());
        out.extend_from_slice(&((self.pixels.len() * 4) as u32).to_le_bytes());
        out.extend_from_slice(&(if self.is_full() { 1u32 } else { 0u32 }).to_le_bytes());
        for px in &self.pixels {
            out.extend_from_slice(&px.to_le_bytes());
        }
        out
    }

    /// Parse a frame, validating *every* field.
    ///
    /// Truncated buffers, bogus magic/version, zero or oversized dimensions, a
    /// damage rectangle outside the screen and a payload length that disagrees
    /// with the damage all produce an [`RcError`] — never a panic, never an
    /// out-of-bounds read.
    pub fn decode(bytes: &[u8]) -> RcResult<Self> {
        if bytes.len() < FRAME_HEADER_LEN {
            return Err(RcError::Launch(format!(
                "AWT frame truncated: {} bytes < {} byte header",
                bytes.len(),
                FRAME_HEADER_LEN
            )));
        }
        let u32_at = |o: usize| -> u32 {
            u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
        };
        let u16_at = |o: usize| -> u32 { u16::from_le_bytes([bytes[o], bytes[o + 1]]) as u32 };

        let magic = u32_at(0);
        if magic != FRAME_MAGIC {
            return Err(RcError::Launch(format!(
                "AWT frame has bad magic 0x{magic:08X} (expected 0x{FRAME_MAGIC:08X})"
            )));
        }
        let version = u16_at(4) as u16;
        if version != FRAME_VERSION {
            return Err(RcError::Launch(format!(
                "unsupported AWT frame version {version} (expected {FRAME_VERSION})"
            )));
        }
        let format = PixelFormat::from_code(u16_at(6) as u16).ok_or_else(|| {
            RcError::Launch(format!("unknown AWT frame pixel format {}", u16_at(6)))
        })?;
        let seq = u32_at(8);
        let width = u16_at(12);
        let height = u16_at(14);
        let damage = Rect::new(u16_at(16), u16_at(18), u16_at(20), u16_at(22));
        let payload_len = u32_at(24) as usize;
        // `flags` (offset 28) is redundant with the damage rectangle today; it is
        // read (and ignored) so old producers stay compatible.
        let available = bytes.len() - FRAME_HEADER_LEN;
        if payload_len > available {
            return Err(RcError::Launch(format!(
                "AWT frame truncated: header claims {payload_len} payload bytes, {available} present"
            )));
        }
        if !payload_len.is_multiple_of(4) {
            return Err(RcError::Launch(format!(
                "AWT frame payload length {payload_len} is not a multiple of 4"
            )));
        }
        let mut pixels = Vec::with_capacity(payload_len / 4);
        for chunk in bytes[FRAME_HEADER_LEN..FRAME_HEADER_LEN + payload_len].chunks(4) {
            pixels.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
        Ok(Self::partial(seq, width, height, damage, pixels)?.with_format(format))
    }
}

fn validate_dims(width: u32, height: u32) -> RcResult<()> {
    if width == 0 || height == 0 {
        return Err(RcError::Launch(format!(
            "invalid AWT canvas size {width}x{height}: both edges must be > 0"
        )));
    }
    if width > MAX_CANVAS_DIM || height > MAX_CANVAS_DIM {
        return Err(RcError::Launch(format!(
            "AWT canvas size {width}x{height} exceeds the {MAX_CANVAS_DIM} px limit"
        )));
    }
    Ok(())
}

// ===========================================================================
// Canvas rendering: the off-screen surface Compose draws
// ===========================================================================

/// Frame statistics for the HUD / diagnostics (task 12's overlay, task 25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CanvasStats {
    /// Frames written into the back buffer.
    pub frames_submitted: u64,
    /// Frames published to the front buffer.
    pub frames_presented: u64,
    /// Frames published *over* a frame the UI never consumed (UI is behind).
    pub frames_dropped: u64,
    /// Pixel bytes actually blitted (damage-limited, so far below `w*h*4*frames`).
    pub bytes_blitted: u64,
    /// How often the virtual desktop was resized.
    pub resizes: u64,
    /// `seq` of the most recently presented frame.
    pub last_seq: u32,
    /// Timestamp (ms) of the most recent present.
    pub last_present_ms: u64,
}

/// Number of present timestamps kept for the FPS estimate.
const FPS_WINDOW: usize = 60;

/// A double-buffered, damage-tracking ARGB surface: the Android-side end of the
/// AWT bridge.
///
/// * **Producer** (the JVM bridge, via [`AwtCanvas::submit`]) writes damaged
///   regions into the *back* buffer and publishes with [`AwtCanvas::present`].
/// * **Consumer** (Compose, via [`AwtCanvas::copy_rgba_into`]) reads the *front*
///   buffer and calls [`AwtCanvas::take_dirty`] to learn what changed, so it can
///   upload just that rectangle into its `Bitmap`.
///
/// Publishing never blocks the producer and never tears for the consumer: the
/// front buffer only changes inside `present`, and only the damaged rows are
/// copied, which keeps a partial 60 fps AWT repaint essentially free.
#[derive(Debug, Clone)]
pub struct AwtCanvas {
    width: u32,
    height: u32,
    back: Vec<u32>,
    front: Vec<u32>,
    /// Damage written into `back` but not yet published.
    pending: Damage,
    /// Damage published into `front` but not yet consumed by the UI.
    dirty: Damage,
    stats: CanvasStats,
    present_times: VecDeque<u64>,
}

impl AwtCanvas {
    /// An opaque black canvas of `width × height` pixels.
    pub fn new(width: u32, height: u32) -> RcResult<Self> {
        validate_dims(width, height)?;
        let len = (width as usize) * (height as usize);
        Ok(Self {
            width,
            height,
            back: vec![OPAQUE_BLACK; len],
            front: vec![OPAQUE_BLACK; len],
            pending: Damage::none(),
            dirty: Damage::full(width, height),
            stats: CanvasStats::default(),
            present_times: VecDeque::with_capacity(FPS_WINDOW),
        })
    }

    /// Canvas width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Canvas height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// `(width, height)`.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Bytes an RGBA8888 copy of the whole canvas needs.
    pub fn rgba_len(&self) -> usize {
        (self.width as usize) * (self.height as usize) * 4
    }

    /// Resize the virtual desktop (the user rotated the phone / changed the
    /// resolution). Contents are reset to opaque black and fully re-damaged, so
    /// the UI cannot show stale pixels at the new size.
    pub fn resize(&mut self, width: u32, height: u32) -> RcResult<()> {
        validate_dims(width, height)?;
        if (width, height) == (self.width, self.height) {
            return Ok(());
        }
        let len = (width as usize) * (height as usize);
        self.width = width;
        self.height = height;
        self.back = vec![OPAQUE_BLACK; len];
        self.front = vec![OPAQUE_BLACK; len];
        self.pending = Damage::none();
        self.dirty = Damage::full(width, height);
        self.stats.resizes += 1;
        Ok(())
    }

    /// Write a frame into the back buffer (no publish yet).
    ///
    /// Rejects a frame whose screen size differs from the canvas: the producer
    /// and the consumer must agree on the desktop size, and silently stretching
    /// would corrupt every subsequent partial update.
    pub fn submit(&mut self, frame: &AwtFrame) -> RcResult<()> {
        if (frame.width, frame.height) != (self.width, self.height) {
            return Err(RcError::Launch(format!(
                "AWT frame is for a {}x{} screen but the canvas is {}x{} (resize first)",
                frame.width, frame.height, self.width, self.height
            )));
        }
        // `AwtFrame`'s constructors validate this, but the struct is public: a
        // hand-built frame must not be able to index out of bounds below.
        if frame.pixels.len() as u64 != frame.damage.area() {
            return Err(RcError::Launch(format!(
                "AWT frame payload has {} pixels, expected {} for a {}x{} damage",
                frame.pixels.len(),
                frame.damage.area(),
                frame.damage.width,
                frame.damage.height
            )));
        }
        // The damage must lie *completely* inside the canvas. Clamping instead
        // would keep us memory-safe but silently blit the wrong pixels (the
        // payload rows are strided by `frame.damage.width`, so a clipped origin
        // shifts every row), and a producer that disagrees with us about the
        // desktop size is a bug we want to see, not to paper over.
        if frame.damage.is_empty() || !frame.damage.fits_in(self.width, self.height) {
            return Err(RcError::Launch(format!(
                "AWT frame damage {}x{}+{}+{} is not inside the {}x{} canvas",
                frame.damage.width,
                frame.damage.height,
                frame.damage.x,
                frame.damage.y,
                self.width,
                self.height
            )));
        }
        let damage = frame.damage;
        let opaque = frame.format == PixelFormat::IntRgb;
        for row in 0..damage.height {
            let src_start = (row as usize) * (frame.damage.width as usize);
            let src = &frame.pixels[src_start..src_start + damage.width as usize];
            let dst_start =
                ((damage.y + row) as usize) * (self.width as usize) + (damage.x as usize);
            let dst = &mut self.back[dst_start..dst_start + damage.width as usize];
            if opaque {
                for (d, s) in dst.iter_mut().zip(src) {
                    *d = s | 0xFF00_0000;
                }
            } else {
                dst.copy_from_slice(src);
            }
        }
        self.pending.add(damage);
        self.stats.frames_submitted += 1;
        self.stats.bytes_blitted += damage.area() * 4;
        self.stats.last_seq = frame.seq;
        Ok(())
    }

    /// Publish the back buffer's pending damage to the front buffer.
    ///
    /// Returns the published rectangle, or `None` when nothing changed since the
    /// last present (an idle AWT desktop costs nothing).
    pub fn present(&mut self) -> Option<Rect> {
        self.present_at(now_millis())
    }

    /// [`AwtCanvas::present`] with an explicit timestamp (deterministic tests).
    pub fn present_at(&mut self, now_ms: u64) -> Option<Rect> {
        let rect = self.pending.take()?;
        if !self.dirty.is_empty() {
            // The UI has not consumed the previous frame yet: it will now see
            // the newer pixels, so the older frame effectively never displayed.
            self.stats.frames_dropped += 1;
        }
        for row in 0..rect.height {
            let start = ((rect.y + row) as usize) * (self.width as usize) + (rect.x as usize);
            let end = start + rect.width as usize;
            self.front[start..end].copy_from_slice(&self.back[start..end]);
        }
        self.dirty.add(rect);
        self.stats.frames_presented += 1;
        self.stats.last_present_ms = now_ms;
        if self.present_times.len() == FPS_WINDOW {
            self.present_times.pop_front();
        }
        self.present_times.push_back(now_ms);
        Some(rect)
    }

    /// [`AwtCanvas::submit`] + [`AwtCanvas::present`] in one call.
    pub fn submit_and_present(&mut self, frame: &AwtFrame) -> RcResult<Option<Rect>> {
        self.submit(frame)?;
        Ok(self.present())
    }

    /// The published damage rectangle, without consuming it.
    pub fn dirty_rect(&self) -> Option<Rect> {
        self.dirty.rect()
    }

    /// Take the published damage rectangle and mark the canvas clean.
    pub fn take_dirty(&mut self) -> Option<Rect> {
        self.dirty.take()
    }

    /// The front buffer (ARGB, row-major) — what the UI must display.
    pub fn front_pixels(&self) -> &[u32] {
        &self.front
    }

    /// A single front-buffer pixel (`None` when out of bounds).
    pub fn pixel(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(self.front[(y as usize) * (self.width as usize) + (x as usize)])
    }

    /// Convert the whole front buffer into RGBA8888 bytes — the memory layout an
    /// Android `Bitmap.Config.ARGB_8888` expects (R,G,B,A per pixel).
    ///
    /// Returns the number of bytes written. `dst` must be at least
    /// [`AwtCanvas::rgba_len`] bytes.
    pub fn copy_rgba_into(&self, dst: &mut [u8]) -> RcResult<usize> {
        self.copy_region_rgba_into(Rect::whole(self.width, self.height), dst)
    }

    /// Convert one region of the front buffer into tightly packed RGBA8888 bytes
    /// (`rect.width * 4` bytes per row) — used for damage-limited uploads.
    pub fn copy_region_rgba_into(&self, rect: Rect, dst: &mut [u8]) -> RcResult<usize> {
        let rect = rect.clamp_to(self.width, self.height);
        if rect.is_empty() {
            return Ok(0);
        }
        let needed = (rect.area() * 4) as usize;
        if dst.len() < needed {
            return Err(RcError::Launch(format!(
                "RGBA destination too small: {} bytes < {needed} needed for {}x{}",
                dst.len(),
                rect.width,
                rect.height
            )));
        }
        let mut o = 0usize;
        for row in 0..rect.height {
            let start = ((rect.y + row) as usize) * (self.width as usize) + (rect.x as usize);
            for &px in &self.front[start..start + rect.width as usize] {
                dst[o] = ((px >> 16) & 0xFF) as u8; // R
                dst[o + 1] = ((px >> 8) & 0xFF) as u8; // G
                dst[o + 2] = (px & 0xFF) as u8; // B
                dst[o + 3] = ((px >> 24) & 0xFF) as u8; // A
                o += 4;
            }
        }
        Ok(needed)
    }

    /// Convert one region of the front buffer into an RGBA8888 **framebuffer**.
    ///
    /// Unlike [`AwtCanvas::copy_region_rgba_into`] (which packs the region
    /// tightly), `dst` here holds the *whole* desktop ([`AwtCanvas::rgba_len`]
    /// bytes) and only `rect`'s rows are rewritten, each at its canvas-relative
    /// offset. That is exactly what the Android upload path wants: the
    /// persistent direct `ByteBuffer` behind a `Bitmap.Config.ARGB_8888` stays a
    /// complete image, so `Bitmap.copyPixelsFromBuffer` can memcpy it wholesale
    /// while the (per-pixel, therefore expensive) ARGB->RGBA conversion stays
    /// limited to the damaged rectangle.
    ///
    /// Returns the number of bytes rewritten (0 for an empty region).
    pub fn copy_region_into_framebuffer(&self, rect: Rect, dst: &mut [u8]) -> RcResult<usize> {
        let rect = rect.clamp_to(self.width, self.height);
        if rect.is_empty() {
            return Ok(0);
        }
        let needed = self.rgba_len();
        if dst.len() < needed {
            return Err(RcError::Launch(format!(
                "RGBA framebuffer too small: {} bytes < {needed} needed for {}x{}",
                dst.len(),
                self.width,
                self.height
            )));
        }
        let stride = (self.width as usize) * 4;
        let mut written = 0usize;
        for row in 0..rect.height {
            let src = ((rect.y + row) as usize) * (self.width as usize) + (rect.x as usize);
            let mut o = ((rect.y + row) as usize) * stride + (rect.x as usize) * 4;
            for &px in &self.front[src..src + rect.width as usize] {
                dst[o] = ((px >> 16) & 0xFF) as u8; // R
                dst[o + 1] = ((px >> 8) & 0xFF) as u8; // G
                dst[o + 2] = (px & 0xFF) as u8; // B
                dst[o + 3] = ((px >> 24) & 0xFF) as u8; // A
                o += 4;
                written += 4;
            }
        }
        Ok(written)
    }

    /// Fill both buffers with one ARGB colour and damage everything (used for a
    /// clean "AWT is starting" / "AWT exited" state).
    pub fn fill(&mut self, argb: u32) {
        self.back.iter_mut().for_each(|p| *p = argb);
        self.front.copy_from_slice(&self.back);
        self.pending.clear();
        self.dirty = Damage::full(self.width, self.height);
    }

    /// Frame statistics.
    pub fn stats(&self) -> CanvasStats {
        self.stats
    }

    /// Presented frames per second over the last [`FPS_WINDOW`] presents.
    ///
    /// Returns `0.0` until at least two frames have been presented, and never a
    /// NaN/∞ (a zero-length window degrades to `0.0`).
    pub fn fps(&self) -> f32 {
        if self.present_times.len() < 2 {
            return 0.0;
        }
        let first = *self.present_times.front().unwrap();
        let last = *self.present_times.back().unwrap();
        let span = last.saturating_sub(first);
        if span == 0 {
            return 0.0;
        }
        ((self.present_times.len() - 1) as f32) * 1000.0 / (span as f32)
    }

    /// JSON snapshot for the FFI / HUD.
    pub fn stats_json(&self) -> serde_json::Value {
        let dirty = self.dirty_rect();
        serde_json::json!({
            "width": self.width,
            "height": self.height,
            "fps": self.fps(),
            "frames_submitted": self.stats.frames_submitted,
            "frames_presented": self.stats.frames_presented,
            "frames_dropped": self.stats.frames_dropped,
            "bytes_blitted": self.stats.bytes_blitted,
            "resizes": self.stats.resizes,
            "last_seq": self.stats.last_seq,
            "dirty": dirty.map(|r| serde_json::json!({
                "x": r.x, "y": r.y, "width": r.width, "height": r.height
            })),
        })
    }
}

/// Fully opaque black, the colour an AWT desktop starts as.
pub const OPAQUE_BLACK: u32 = 0xFF00_0000;

/// Milliseconds since the Unix epoch (monotonic enough for FPS; a clock jump can
/// only distort one window, and `saturating_sub` keeps it non-negative).
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ===========================================================================
// Canvas rendering: viewport (letterboxing + touch mapping)
// ===========================================================================

/// How the virtual AWT desktop is fitted into the Compose surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScaleMode {
    /// Fill the surface exactly, ignoring the aspect ratio.
    Stretch,
    /// Preserve the aspect ratio, letterboxing the remainder. **Default** —
    /// Swing dialogs stay readable and un-squashed.
    #[default]
    Fit,
    /// Preserve the aspect ratio and cover the surface, cropping the overflow.
    FillCrop,
    /// No scaling (1 canvas px = 1 surface px), centred.
    Center,
}

/// Where the desktop lands inside the surface (may extend past the edges for
/// [`ScaleMode::FillCrop`] / [`ScaleMode::Center`], hence the signed origin).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    /// Left edge in surface pixels (can be negative).
    pub x: i32,
    /// Top edge in surface pixels (can be negative).
    pub y: i32,
    /// Width in surface pixels.
    pub width: u32,
    /// Height in surface pixels.
    pub height: u32,
}

/// Maps between the virtual AWT desktop and the Compose surface it is drawn on.
///
/// This is what makes a *touch* land on the right Swing button: the UI reports a
/// position in surface pixels, and the AWT peers only understand desktop pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    /// Virtual AWT desktop size (the canvas).
    pub screen: (u32, u32),
    /// The Compose surface size, in pixels.
    pub surface: (u32, u32),
    /// Fitting policy.
    pub mode: ScaleMode,
}

impl Viewport {
    /// A viewport with the default [`ScaleMode::Fit`] policy.
    pub fn new(screen: (u32, u32), surface: (u32, u32)) -> Self {
        Self {
            screen,
            surface,
            mode: ScaleMode::default(),
        }
    }

    /// A viewport with an explicit scale mode.
    pub fn with_mode(mut self, mode: ScaleMode) -> Self {
        self.mode = mode;
        self
    }

    /// Where the desktop is drawn inside the surface.
    pub fn placement(&self) -> Placement {
        let (cw, ch) = self.screen;
        let (sw, sh) = self.surface;
        if cw == 0 || ch == 0 || sw == 0 || sh == 0 {
            return Placement {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            };
        }
        let (w, h) = match self.mode {
            ScaleMode::Stretch => (sw, sh),
            ScaleMode::Center => (cw, ch),
            ScaleMode::Fit | ScaleMode::FillCrop => {
                // Integer arithmetic only: no NaN can reach a blit or a touch.
                let by_width = (sw as u64) * (ch as u64); // compare sw/cw vs sh/ch
                let by_height = (sh as u64) * (cw as u64);
                let height_is_limit = if self.mode == ScaleMode::Fit {
                    by_width > by_height
                } else {
                    by_width < by_height
                };
                if height_is_limit {
                    let w = ((cw as u64) * (sh as u64) / (ch as u64)).max(1) as u32;
                    (w, sh)
                } else {
                    let h = ((ch as u64) * (sw as u64) / (cw as u64)).max(1) as u32;
                    (sw, h)
                }
            }
        };
        Placement {
            x: ((sw as i64 - w as i64) / 2) as i32,
            y: ((sh as i64 - h as i64) / 2) as i32,
            width: w,
            height: h,
        }
    }

    /// Uniform-ish scale factors `(x, y)` from desktop to surface pixels.
    pub fn scale(&self) -> (f32, f32) {
        let p = self.placement();
        let (cw, ch) = self.screen;
        if cw == 0 || ch == 0 || p.width == 0 || p.height == 0 {
            return (1.0, 1.0);
        }
        (p.width as f32 / cw as f32, p.height as f32 / ch as f32)
    }

    /// Map a surface position to a desktop pixel.
    ///
    /// Returns `None` when the position is outside the drawn area (a tap on the
    /// letterbox bars must not be forwarded as a click at the edge) or when the
    /// input is not finite.
    pub fn map_pointer(&self, surface_x: f32, surface_y: f32) -> Option<(u32, u32)> {
        if !surface_x.is_finite() || !surface_y.is_finite() {
            return None;
        }
        let p = self.placement();
        if p.width == 0 || p.height == 0 {
            return None;
        }
        let rel_x = surface_x - p.x as f32;
        let rel_y = surface_y - p.y as f32;
        if rel_x < 0.0 || rel_y < 0.0 || rel_x >= p.width as f32 || rel_y >= p.height as f32 {
            return None;
        }
        let (cw, ch) = self.screen;
        let x = (rel_x * cw as f32 / p.width as f32) as u32;
        let y = (rel_y * ch as f32 / p.height as f32) as u32;
        Some((x.min(cw.saturating_sub(1)), y.min(ch.saturating_sub(1))))
    }

    /// Like [`Viewport::map_pointer`] but clamps into the desktop instead of
    /// rejecting: used while *dragging*, where a finger leaving the letterbox
    /// must keep dragging the Swing scrollbar rather than dropping it.
    pub fn map_pointer_clamped(&self, surface_x: f32, surface_y: f32) -> (u32, u32) {
        let (cw, ch) = self.screen;
        let max = (cw.saturating_sub(1), ch.saturating_sub(1));
        if !surface_x.is_finite() || !surface_y.is_finite() {
            return (0, 0);
        }
        let p = self.placement();
        if p.width == 0 || p.height == 0 {
            return (0, 0);
        }
        let rel_x = (surface_x - p.x as f32).max(0.0);
        let rel_y = (surface_y - p.y as f32).max(0.0);
        let x = (rel_x * cw as f32 / p.width as f32) as u32;
        let y = (rel_y * ch as f32 / p.height as f32) as u32;
        (x.min(max.0), y.min(max.1))
    }

    /// Map a desktop pixel back to a surface position (cursor overlays).
    pub fn map_to_surface(&self, x: u32, y: u32) -> (f32, f32) {
        let p = self.placement();
        let (cw, ch) = self.screen;
        if cw == 0 || ch == 0 {
            return (p.x as f32, p.y as f32);
        }
        (
            p.x as f32 + x as f32 * p.width as f32 / cw as f32,
            p.y as f32 + y as f32 * p.height as f32 / ch as f32,
        )
    }
}

// ===========================================================================
// Input: Compose gestures / keys -> AWT events
// ===========================================================================

/// A high-level input event coming from the Compose layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwtEvent {
    /// Pointer moved to a desktop pixel (drag when a button is held).
    PointerMove {
        /// Desktop x.
        x: u32,
        /// Desktop y.
        y: u32,
    },
    /// A button went down at a desktop pixel.
    PointerDown {
        /// Desktop x.
        x: u32,
        /// Desktop y.
        y: u32,
        /// Which button.
        button: MouseButton,
    },
    /// A button was released at a desktop pixel.
    PointerUp {
        /// Desktop x.
        x: u32,
        /// Desktop y.
        y: u32,
        /// Which button.
        button: MouseButton,
    },
    /// Wheel scroll (`ticks` > 0 scrolls down/away, as in AWT).
    Scroll {
        /// Desktop x.
        x: u32,
        /// Desktop y.
        y: u32,
        /// Notches scrolled.
        ticks: i32,
    },
    /// A key went down (`VK_*` code, see [`vk_for_key`]).
    KeyDown {
        /// `KeyEvent.VK_*`.
        code: i32,
    },
    /// A key was released.
    KeyUp {
        /// `KeyEvent.VK_*`.
        code: i32,
    },
    /// A typed character (IME / soft keyboard): becomes `KEY_TYPED`.
    Text {
        /// The character.
        ch: char,
    },
    /// The canvas gained / lost focus.
    Focus {
        /// `true` = gained.
        gained: bool,
    },
    /// The virtual desktop was resized.
    Resize {
        /// New width.
        width: u32,
        /// New height.
        height: u32,
    },
}

/// One wire record the JVM-side bridge turns into a real `java.awt.event.*Event`.
///
/// Fixed 32-byte little-endian layout (8 × `i32`), in field order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AwtEventRecord {
    /// `java.awt.event.*Event` id (see [`event_id`]).
    pub id: i32,
    /// Desktop x (or the new width for `COMPONENT_RESIZED`).
    pub x: i32,
    /// Desktop y (or the new height for `COMPONENT_RESIZED`).
    pub y: i32,
    /// AWT button number (0 = none).
    pub button: i32,
    /// `KeyEvent.VK_*` (0 = `VK_UNDEFINED`).
    pub key_code: i32,
    /// UTF-32 character for `KEY_TYPED` ([`vk::CHAR_UNDEFINED`] otherwise).
    pub key_char: u32,
    /// `getModifiersEx()` value (see [`mask`]).
    pub modifiers: i32,
    /// Wheel notches for `MOUSE_WHEEL`.
    pub wheel: i32,
}

/// Size of one encoded [`AwtEventRecord`].
pub const EVENT_RECORD_LEN: usize = 32;

impl AwtEventRecord {
    /// Serialise to 32 little-endian bytes.
    pub fn encode(&self) -> [u8; EVENT_RECORD_LEN] {
        let mut out = [0u8; EVENT_RECORD_LEN];
        out[0..4].copy_from_slice(&self.id.to_le_bytes());
        out[4..8].copy_from_slice(&self.x.to_le_bytes());
        out[8..12].copy_from_slice(&self.y.to_le_bytes());
        out[12..16].copy_from_slice(&self.button.to_le_bytes());
        out[16..20].copy_from_slice(&self.key_code.to_le_bytes());
        out[20..24].copy_from_slice(&self.key_char.to_le_bytes());
        out[24..28].copy_from_slice(&self.modifiers.to_le_bytes());
        out[28..32].copy_from_slice(&self.wheel.to_le_bytes());
        out
    }

    /// Parse one record (rejects a short buffer).
    pub fn decode(bytes: &[u8]) -> RcResult<Self> {
        if bytes.len() < EVENT_RECORD_LEN {
            return Err(RcError::Launch(format!(
                "AWT event record truncated: {} bytes < {EVENT_RECORD_LEN}",
                bytes.len()
            )));
        }
        let i32_at =
            |o: usize| i32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        let u32_at =
            |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        Ok(Self {
            id: i32_at(0),
            x: i32_at(4),
            y: i32_at(8),
            button: i32_at(12),
            key_code: i32_at(16),
            key_char: u32_at(20),
            modifiers: i32_at(24),
            wheel: i32_at(28),
        })
    }

    /// Whether this record is a *control* record ([`CONTROL_EVENT_ID`]) rather
    /// than an AWT input event.
    ///
    /// The JVM-side reader dispatches on this: a control record must never be
    /// handed to `EventQueue.postEvent`, and load shedding
    /// (`AwtSession::shed_one`) must never drop one, because a chunked reply is
    /// only meaningful as a whole run.
    pub fn is_control(&self) -> bool {
        self.id == CONTROL_EVENT_ID
    }

    /// Serialise a batch (one `write()` per UI frame instead of per event).
    pub fn encode_batch(records: &[AwtEventRecord]) -> Vec<u8> {
        let mut out = Vec::with_capacity(records.len() * EVENT_RECORD_LEN);
        for r in records {
            out.extend_from_slice(&r.encode());
        }
        out
    }

    /// Parse a batch; rejects a buffer whose length is not a record multiple.
    pub fn decode_batch(bytes: &[u8]) -> RcResult<Vec<AwtEventRecord>> {
        if !bytes.len().is_multiple_of(EVENT_RECORD_LEN) {
            return Err(RcError::Launch(format!(
                "AWT event batch of {} bytes is not a multiple of {EVENT_RECORD_LEN}",
                bytes.len()
            )));
        }
        bytes
            .chunks(EVENT_RECORD_LEN)
            .map(AwtEventRecord::decode)
            .collect()
    }
}

/// Turns Compose input into AWT event records, keeping the button / modifier
/// state AWT's `getModifiersEx()` contract requires.
///
/// It also implements the two behaviours a naive translation gets wrong:
/// * a *move with a button held* is `MOUSE_DRAGGED`, not `MOUSE_MOVED` (Swing
///   scrollbars and text selection depend on it);
/// * `MOUSE_CLICKED` is synthesised after a release that did not travel further
///   than [`AwtInputTranslator::click_slop`] pixels — buttons in Swing react to
///   `mouseClicked`, and a finger always jitters a little.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwtInputTranslator {
    modifiers: i32,
    buttons: i32,
    pointer: (i32, i32),
    press_origin: Option<(i32, i32, MouseButton)>,
    click_slop: u32,
}

impl Default for AwtInputTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl AwtInputTranslator {
    /// A translator with a 12 px click tolerance (finger-friendly).
    pub fn new() -> Self {
        Self {
            modifiers: 0,
            buttons: 0,
            pointer: (0, 0),
            press_origin: None,
            click_slop: 12,
        }
    }

    /// Override the click tolerance in desktop pixels (0 = exact position).
    pub fn with_click_slop(mut self, slop: u32) -> Self {
        self.click_slop = slop;
        self
    }

    /// Current click tolerance.
    pub fn click_slop(&self) -> u32 {
        self.click_slop
    }

    /// Current `getModifiersEx()` value (modifier keys + held buttons).
    pub fn modifiers(&self) -> i32 {
        self.modifiers | self.buttons
    }

    /// Last known pointer position in desktop pixels.
    pub fn pointer(&self) -> (i32, i32) {
        self.pointer
    }

    /// Translate one event into zero or more AWT records.
    pub fn translate(&mut self, event: AwtEvent) -> Vec<AwtEventRecord> {
        match event {
            AwtEvent::PointerMove { x, y } => {
                self.pointer = (x as i32, y as i32);
                let id = if self.buttons != 0 {
                    event_id::MOUSE_DRAGGED
                } else {
                    event_id::MOUSE_MOVED
                };
                vec![self.mouse_record(id, 0, 0)]
            }
            AwtEvent::PointerDown { x, y, button } => {
                self.pointer = (x as i32, y as i32);
                self.buttons |= button.mask();
                self.press_origin = Some((x as i32, y as i32, button));
                vec![self.mouse_record(event_id::MOUSE_PRESSED, button.number(), 0)]
            }
            AwtEvent::PointerUp { x, y, button } => {
                self.pointer = (x as i32, y as i32);
                self.buttons &= !button.mask();
                let mut out = vec![self.mouse_record(event_id::MOUSE_RELEASED, button.number(), 0)];
                if let Some((px, py, pb)) = self.press_origin.take() {
                    let travelled = (px - x as i32)
                        .unsigned_abs()
                        .max((py - y as i32).unsigned_abs());
                    if pb == button && travelled <= self.click_slop {
                        out.push(self.mouse_record(event_id::MOUSE_CLICKED, button.number(), 0));
                    }
                }
                out
            }
            AwtEvent::Scroll { x, y, ticks } => {
                self.pointer = (x as i32, y as i32);
                vec![self.mouse_record(event_id::MOUSE_WHEEL, 0, ticks)]
            }
            AwtEvent::KeyDown { code } => {
                if let Some(m) = modifier_mask_for_vk(code) {
                    self.modifiers |= m;
                }
                vec![self.key_record(event_id::KEY_PRESSED, code, vk::CHAR_UNDEFINED)]
            }
            AwtEvent::KeyUp { code } => {
                if let Some(m) = modifier_mask_for_vk(code) {
                    self.modifiers &= !m;
                }
                vec![self.key_record(event_id::KEY_RELEASED, code, vk::CHAR_UNDEFINED)]
            }
            AwtEvent::Text { ch } => {
                vec![self.key_record(event_id::KEY_TYPED, 0, ch as u32)]
            }
            AwtEvent::Focus { gained } => {
                let mut out = Vec::new();
                if !gained {
                    // Release everything so nothing stays stuck while the app is
                    // in the background (a held Shift would break the next input).
                    out.extend(self.release_all());
                }
                out.push(AwtEventRecord {
                    id: if gained {
                        event_id::FOCUS_GAINED
                    } else {
                        event_id::FOCUS_LOST
                    },
                    key_char: vk::CHAR_UNDEFINED,
                    modifiers: self.modifiers(),
                    ..Default::default()
                });
                out
            }
            AwtEvent::Resize { width, height } => vec![AwtEventRecord {
                id: event_id::COMPONENT_RESIZED,
                x: width as i32,
                y: height as i32,
                key_char: vk::CHAR_UNDEFINED,
                modifiers: self.modifiers(),
                ..Default::default()
            }],
        }
    }

    /// Translate a sequence of events (batched UI frame).
    pub fn translate_all<I: IntoIterator<Item = AwtEvent>>(
        &mut self,
        events: I,
    ) -> Vec<AwtEventRecord> {
        events.into_iter().flat_map(|e| self.translate(e)).collect()
    }

    /// Type a whole string (soft keyboard / IME commit).
    pub fn type_str(&mut self, text: &str) -> Vec<AwtEventRecord> {
        text.chars()
            .map(|ch| self.key_record(event_id::KEY_TYPED, 0, ch as u32))
            .collect()
    }

    /// Press a named key (see [`vk_for_key`]) — `None` when the name is unknown,
    /// so the caller can fall back to [`AwtInputTranslator::type_str`].
    pub fn press_named(&mut self, name: &str) -> Option<Vec<AwtEventRecord>> {
        let code = vk_for_key(name)?;
        Some(self.translate(AwtEvent::KeyDown { code }))
    }

    /// Release a named key.
    pub fn release_named(&mut self, name: &str) -> Option<Vec<AwtEventRecord>> {
        let code = vk_for_key(name)?;
        Some(self.translate(AwtEvent::KeyUp { code }))
    }

    /// A pointer event given in *surface* coordinates, mapped through a
    /// [`Viewport`]. Returns an empty vector when the position is outside the
    /// drawn desktop and no button is held.
    pub fn pointer_from_surface(
        &mut self,
        viewport: &Viewport,
        surface_x: f32,
        surface_y: f32,
        phase: PointerPhase,
        button: MouseButton,
    ) -> Vec<AwtEventRecord> {
        // While dragging we clamp (the finger may leave the letterbox), otherwise
        // a tap outside the desktop is simply not an AWT event.
        let dragging = self.buttons != 0 || phase == PointerPhase::Up;
        let mapped = if dragging {
            Some(viewport.map_pointer_clamped(surface_x, surface_y))
        } else {
            viewport.map_pointer(surface_x, surface_y)
        };
        let Some((x, y)) = mapped else {
            return Vec::new();
        };
        self.translate(match phase {
            PointerPhase::Down => AwtEvent::PointerDown { x, y, button },
            PointerPhase::Move => AwtEvent::PointerMove { x, y },
            PointerPhase::Up => AwtEvent::PointerUp { x, y, button },
        })
    }

    /// Release every held button and modifier, returning the records that do it.
    pub fn release_all(&mut self) -> Vec<AwtEventRecord> {
        let mut out = Vec::new();
        for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
            if self.buttons & button.mask() != 0 {
                self.buttons &= !button.mask();
                out.push(self.mouse_record(event_id::MOUSE_RELEASED, button.number(), 0));
            }
        }
        for (m, code) in [
            (mask::SHIFT_DOWN, vk::SHIFT),
            (mask::CTRL_DOWN, vk::CONTROL),
            (mask::ALT_DOWN, vk::ALT),
            (mask::META_DOWN, vk::META),
        ] {
            if self.modifiers & m != 0 {
                self.modifiers &= !m;
                out.push(self.key_record(event_id::KEY_RELEASED, code, vk::CHAR_UNDEFINED));
            }
        }
        self.press_origin = None;
        out
    }

    /// Forget all state (new game session / bridge restart).
    pub fn reset(&mut self) {
        self.modifiers = 0;
        self.buttons = 0;
        self.pointer = (0, 0);
        self.press_origin = None;
    }

    fn mouse_record(&self, id: i32, button: i32, wheel: i32) -> AwtEventRecord {
        AwtEventRecord {
            id,
            x: self.pointer.0,
            y: self.pointer.1,
            button,
            key_code: 0,
            key_char: vk::CHAR_UNDEFINED,
            modifiers: self.modifiers(),
            wheel,
        }
    }

    fn key_record(&self, id: i32, code: i32, ch: u32) -> AwtEventRecord {
        AwtEventRecord {
            id,
            x: self.pointer.0,
            y: self.pointer.1,
            button: 0,
            key_code: code,
            key_char: ch,
            modifiers: self.modifiers(),
            wheel: 0,
        }
    }
}

/// Phase of a pointer gesture coming from Compose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerPhase {
    /// Finger / mouse went down.
    Down,
    /// Moved (drag when a button is held).
    Move,
    /// Released.
    Up,
}

// ===========================================================================
// Control plane: everything that crosses the bridge but is not a pixel
// ===========================================================================
//
// Pixels alone do not make an AWT desktop usable. caciocavallo's peers also
// implement `CTCClipboard`, `CTCRobotPeer`, cursor management, window titles and
// the input-method plumbing (see the `cacio-shared` / `cacio-tta` trees), and
// *all* of those need to reach the host that owns the real screen:
//
// | the JVM does | the launcher must |
// |---|---|
// | `setCursor(HAND_CURSOR)` over a link | draw a hand pointer, not an arrow |
// | `JFrame.setTitle("Forge 安装程序")` | label the canvas |
// | `Clipboard.setContents("seed")` | put it on the Android clipboard |
// | `Clipboard.getContents()` | *answer* with the Android clipboard |
// | a text field gains focus | pop the soft keyboard at the caret |
// | `Toolkit.beep()` | a haptic tick |
// | cacio's managed screen is NxM | make the canvas exactly NxM |
//
// So the bridge carries a second record type in each direction:
//
// * JVM → launcher: [`AwtControl`], variable length, magic `"RCAC"`. It shares
//   the 32-byte header shape of [`AwtFrame`] (version at 4, payload length at
//   24) so one stream reader handles both and a control message can never
//   desynchronise the frame stream.
// * launcher → JVM: a *control record* — an ordinary 32-byte [`AwtEventRecord`]
//   with the reserved id [`CONTROL_EVENT_ID`]. Keeping the reverse channel
//   strictly fixed-length means the JVM-side reader stays a `readFully(32)`
//   loop; text (a clipboard answer) is chunked across records
//   ([`encode_control_reply`]).

/// `java.awt.Cursor` type constants, so a control message can carry exactly what
/// `Component.getCursor().getType()` returned.
pub mod cursor_type {
    /// `Cursor.DEFAULT_CURSOR`
    pub const DEFAULT: i32 = 0;
    /// `Cursor.CROSSHAIR_CURSOR`
    pub const CROSSHAIR: i32 = 1;
    /// `Cursor.TEXT_CURSOR`
    pub const TEXT: i32 = 2;
    /// `Cursor.WAIT_CURSOR`
    pub const WAIT: i32 = 3;
    /// `Cursor.SW_RESIZE_CURSOR`
    pub const SW_RESIZE: i32 = 4;
    /// `Cursor.SE_RESIZE_CURSOR`
    pub const SE_RESIZE: i32 = 5;
    /// `Cursor.NW_RESIZE_CURSOR`
    pub const NW_RESIZE: i32 = 6;
    /// `Cursor.NE_RESIZE_CURSOR`
    pub const NE_RESIZE: i32 = 7;
    /// `Cursor.N_RESIZE_CURSOR`
    pub const N_RESIZE: i32 = 8;
    /// `Cursor.S_RESIZE_CURSOR`
    pub const S_RESIZE: i32 = 9;
    /// `Cursor.W_RESIZE_CURSOR`
    pub const W_RESIZE: i32 = 10;
    /// `Cursor.E_RESIZE_CURSOR`
    pub const E_RESIZE: i32 = 11;
    /// `Cursor.HAND_CURSOR`
    pub const HAND: i32 = 12;
    /// `Cursor.MOVE_CURSOR`
    pub const MOVE: i32 = 13;
    /// `Cursor.CUSTOM_CURSOR` (a bitmap cursor: we fall back to the arrow).
    pub const CUSTOM: i32 = -1;
}

/// The pointer shape the JVM asked for, in a form the UI can act on.
///
/// Android has no cursor to hand over to the window manager, so the launcher
/// draws its own — but *which* one matters: an I-beam is the only cue that a
/// Swing text field is under the finger, and a hand is the only cue that a
/// `JLabel` is a link. Unknown / custom cursors degrade to [`CursorKind::Default`]
/// instead of being an error (a bitmap cursor must not break the link).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorKind {
    /// The arrow.
    #[default]
    Default,
    /// Crosshair (`Canvas`-style components).
    Crosshair,
    /// I-beam: a text field / text area is under the pointer.
    Text,
    /// The busy cursor: the JVM is working (installer unpacking, …).
    Wait,
    /// A resize grip; the direction is kept for a future 8-way grip overlay.
    Resize {
        /// `-1`/`0`/`1` horizontal component of the grip direction.
        dx: i8,
        /// `-1`/`0`/`1` vertical component of the grip direction.
        dy: i8,
    },
    /// Hand: a clickable link / draggable item.
    Hand,
    /// Move: the whole window / component can be dragged.
    Move,
}

impl CursorKind {
    /// Map a `java.awt.Cursor` type. Unknown values become [`CursorKind::Default`].
    pub fn from_awt_type(kind: i32) -> CursorKind {
        match kind {
            cursor_type::CROSSHAIR => CursorKind::Crosshair,
            cursor_type::TEXT => CursorKind::Text,
            cursor_type::WAIT => CursorKind::Wait,
            cursor_type::SW_RESIZE => CursorKind::Resize { dx: -1, dy: 1 },
            cursor_type::SE_RESIZE => CursorKind::Resize { dx: 1, dy: 1 },
            cursor_type::NW_RESIZE => CursorKind::Resize { dx: -1, dy: -1 },
            cursor_type::NE_RESIZE => CursorKind::Resize { dx: 1, dy: -1 },
            cursor_type::N_RESIZE => CursorKind::Resize { dx: 0, dy: -1 },
            cursor_type::S_RESIZE => CursorKind::Resize { dx: 0, dy: 1 },
            cursor_type::W_RESIZE => CursorKind::Resize { dx: -1, dy: 0 },
            cursor_type::E_RESIZE => CursorKind::Resize { dx: 1, dy: 0 },
            cursor_type::HAND => CursorKind::Hand,
            cursor_type::MOVE => CursorKind::Move,
            _ => CursorKind::Default,
        }
    }

    /// Back to the `java.awt.Cursor` type (round-trips [`CursorKind::from_awt_type`]).
    pub fn awt_type(self) -> i32 {
        match self {
            CursorKind::Default => cursor_type::DEFAULT,
            CursorKind::Crosshair => cursor_type::CROSSHAIR,
            CursorKind::Text => cursor_type::TEXT,
            CursorKind::Wait => cursor_type::WAIT,
            CursorKind::Hand => cursor_type::HAND,
            CursorKind::Move => cursor_type::MOVE,
            CursorKind::Resize { dx, dy } => match (dx, dy) {
                (-1, 1) => cursor_type::SW_RESIZE,
                (1, 1) => cursor_type::SE_RESIZE,
                (-1, -1) => cursor_type::NW_RESIZE,
                (1, -1) => cursor_type::NE_RESIZE,
                (0, -1) => cursor_type::N_RESIZE,
                (0, 1) => cursor_type::S_RESIZE,
                (-1, 0) => cursor_type::W_RESIZE,
                (1, 0) => cursor_type::E_RESIZE,
                _ => cursor_type::DEFAULT,
            },
        }
    }

    /// Stable id for JSON / the Kotlin `AwtCursorKind` enum.
    pub fn id(self) -> &'static str {
        match self {
            CursorKind::Default => "default",
            CursorKind::Crosshair => "crosshair",
            CursorKind::Text => "text",
            CursorKind::Wait => "wait",
            CursorKind::Hand => "hand",
            CursorKind::Move => "move",
            CursorKind::Resize { dx, dy } => match (dx, dy) {
                (-1, 1) => "sw_resize",
                (1, 1) => "se_resize",
                (-1, -1) => "nw_resize",
                (1, -1) => "ne_resize",
                (0, -1) => "n_resize",
                (0, 1) => "s_resize",
                (-1, 0) => "w_resize",
                (1, 0) => "e_resize",
                _ => "default",
            },
        }
    }

    /// Whether this shape means "there is an editable text field here" — the cue
    /// the UI uses to offer the soft keyboard even without an explicit
    /// [`AwtControlKind::ImeShow`].
    pub fn is_text(self) -> bool {
        matches!(self, CursorKind::Text)
    }
}

/// What a [`AwtControl`] message says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AwtControlKind {
    /// The pointer shape changed (`a` = `java.awt.Cursor` type).
    Cursor,
    /// The active window's title changed (`text`).
    Title,
    /// The JVM copied `text`: push it onto the Android clipboard.
    ClipboardSet,
    /// The JVM wants the clipboard; answer with [`AwtReplyKind::Clipboard`]
    /// carrying the same `seq`.
    ClipboardRequest,
    /// `Toolkit.beep()`.
    Beep,
    /// cacio's managed screen really is `a`×`b`: make the canvas match.
    ScreenSize,
    /// A text component wants input at desktop pixel (`a`,`b`), line height `c`.
    ImeShow,
    /// No text component is focused any more: the soft keyboard can go.
    ImeHide,
    /// A window / dialog became visible (`a` = window id, `text` = title).
    WindowOpened,
    /// A window / dialog was disposed (`a` = window id).
    WindowClosed,
    /// The JVM-side bridge is shutting down cleanly (`text` = reason).
    Bye,
}

impl AwtControlKind {
    /// The on-the-wire code.
    pub fn code(self) -> u16 {
        match self {
            AwtControlKind::Cursor => 1,
            AwtControlKind::Title => 2,
            AwtControlKind::ClipboardSet => 3,
            AwtControlKind::ClipboardRequest => 4,
            AwtControlKind::Beep => 5,
            AwtControlKind::ScreenSize => 6,
            AwtControlKind::ImeShow => 7,
            AwtControlKind::ImeHide => 8,
            AwtControlKind::WindowOpened => 9,
            AwtControlKind::WindowClosed => 10,
            AwtControlKind::Bye => 11,
        }
    }

    /// Parse a wire code. An unknown kind is **not** accepted: a newer JVM-side
    /// bridge talking to an older core must be diagnosed, not silently ignored.
    pub fn from_code(code: u16) -> Option<AwtControlKind> {
        Some(match code {
            1 => AwtControlKind::Cursor,
            2 => AwtControlKind::Title,
            3 => AwtControlKind::ClipboardSet,
            4 => AwtControlKind::ClipboardRequest,
            5 => AwtControlKind::Beep,
            6 => AwtControlKind::ScreenSize,
            7 => AwtControlKind::ImeShow,
            8 => AwtControlKind::ImeHide,
            9 => AwtControlKind::WindowOpened,
            10 => AwtControlKind::WindowClosed,
            11 => AwtControlKind::Bye,
            _ => return None,
        })
    }

    /// Stable id for JSON / the Kotlin parser.
    pub fn id(self) -> &'static str {
        match self {
            AwtControlKind::Cursor => "cursor",
            AwtControlKind::Title => "title",
            AwtControlKind::ClipboardSet => "clipboard_set",
            AwtControlKind::ClipboardRequest => "clipboard_request",
            AwtControlKind::Beep => "beep",
            AwtControlKind::ScreenSize => "screen_size",
            AwtControlKind::ImeShow => "ime_show",
            AwtControlKind::ImeHide => "ime_hide",
            AwtControlKind::WindowOpened => "window_opened",
            AwtControlKind::WindowClosed => "window_closed",
            AwtControlKind::Bye => "bye",
        }
    }

    /// Whether the UI must react *now* (as opposed to purely informational
    /// bookkeeping the diagnostics panel can pick up later).
    pub fn needs_ui(self) -> bool {
        matches!(
            self,
            AwtControlKind::Cursor
                | AwtControlKind::ClipboardSet
                | AwtControlKind::ClipboardRequest
                | AwtControlKind::Beep
                | AwtControlKind::ImeShow
                | AwtControlKind::ImeHide
        )
    }
}

/// Magic of a control message: `"RCAC"` (RC AWT Control), little-endian.
pub const CONTROL_MAGIC: u32 = 0x5243_4143;
/// Control wire version.
pub const CONTROL_VERSION: u16 = 1;
/// Fixed header length; identical shape to [`FRAME_HEADER_LEN`] so a single
/// stream reader can demultiplex frames and control messages.
pub const CONTROL_HEADER_LEN: usize = 32;
/// Largest text payload a single control message may carry (a clipboard paste of
/// a modpack log is still far below this; anything larger is a bug or an attack).
pub const MAX_CONTROL_TEXT: usize = 64 * 1024;

/// One non-pixel message from the game JVM.
///
/// Wire layout (little-endian, 32-byte header + UTF-8 payload):
///
/// ```text
/// 0  u32 magic("RCAC")   4  u16 version   6  u16 kind
/// 8  u32 seq            12  i32 a        16  i32 b       20 i32 c
/// 24 u32 payload_len    28  u32 flags
/// 32 …  payload_len bytes of UTF-8 text
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwtControl {
    /// What this message is.
    pub kind: AwtControlKind,
    /// Correlation id: a [`AwtControlKind::ClipboardRequest`] is answered with
    /// the same `seq` so a late answer cannot be mistaken for a fresh one.
    pub seq: u32,
    /// First integer argument (cursor type / width / caret x / window id).
    pub a: i32,
    /// Second integer argument (height / caret y).
    pub b: i32,
    /// Third integer argument (caret line height).
    pub c: i32,
    /// Reserved bit field (unknown bits are ignored, never fatal).
    pub flags: u32,
    /// UTF-8 payload (title, clipboard text, reason); empty when unused.
    pub text: String,
}

impl AwtControl {
    /// A bare message of `kind` (no arguments).
    pub fn new(kind: AwtControlKind) -> Self {
        Self {
            kind,
            seq: 0,
            a: 0,
            b: 0,
            c: 0,
            flags: 0,
            text: String::new(),
        }
    }

    /// `setCursor` reached a peer.
    pub fn cursor(kind: CursorKind) -> Self {
        Self {
            a: kind.awt_type(),
            ..Self::new(AwtControlKind::Cursor)
        }
    }

    /// The active window's title.
    pub fn title(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::new(AwtControlKind::Title)
        }
    }

    /// The JVM copied something.
    pub fn clipboard_set(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::new(AwtControlKind::ClipboardSet)
        }
    }

    /// The JVM wants to paste; `seq` correlates the answer.
    pub fn clipboard_request(seq: u32) -> Self {
        Self {
            seq,
            ..Self::new(AwtControlKind::ClipboardRequest)
        }
    }

    /// `Toolkit.beep()`.
    pub fn beep() -> Self {
        Self::new(AwtControlKind::Beep)
    }

    /// The managed screen size cacio actually uses.
    pub fn screen_size(width: u32, height: u32) -> Self {
        Self {
            a: width.min(i32::MAX as u32) as i32,
            b: height.min(i32::MAX as u32) as i32,
            ..Self::new(AwtControlKind::ScreenSize)
        }
    }

    /// A text component wants input at a desktop pixel.
    pub fn ime_show(x: i32, y: i32, line_height: i32) -> Self {
        Self {
            a: x,
            b: y,
            c: line_height,
            ..Self::new(AwtControlKind::ImeShow)
        }
    }

    /// Nothing wants text input any more.
    pub fn ime_hide() -> Self {
        Self::new(AwtControlKind::ImeHide)
    }

    /// A window / dialog became visible.
    pub fn window_opened(id: i32, title: impl Into<String>) -> Self {
        Self {
            a: id,
            text: title.into(),
            ..Self::new(AwtControlKind::WindowOpened)
        }
    }

    /// A window / dialog was disposed.
    pub fn window_closed(id: i32) -> Self {
        Self {
            a: id,
            ..Self::new(AwtControlKind::WindowClosed)
        }
    }

    /// The JVM-side bridge is going away for a stated reason.
    pub fn bye(reason: impl Into<String>) -> Self {
        Self {
            text: reason.into(),
            ..Self::new(AwtControlKind::Bye)
        }
    }

    /// Attach a correlation id.
    pub fn with_seq(mut self, seq: u32) -> Self {
        self.seq = seq;
        self
    }

    /// The cursor shape this message asks for (only for
    /// [`AwtControlKind::Cursor`]).
    pub fn cursor_kind(&self) -> Option<CursorKind> {
        match self.kind {
            AwtControlKind::Cursor => Some(CursorKind::from_awt_type(self.a)),
            _ => None,
        }
    }

    /// Encode to the wire.
    pub fn encode(&self) -> Vec<u8> {
        let payload = self.text.as_bytes();
        let mut out = Vec::with_capacity(CONTROL_HEADER_LEN + payload.len());
        out.extend_from_slice(&CONTROL_MAGIC.to_le_bytes());
        out.extend_from_slice(&CONTROL_VERSION.to_le_bytes());
        out.extend_from_slice(&self.kind.code().to_le_bytes());
        out.extend_from_slice(&self.seq.to_le_bytes());
        out.extend_from_slice(&self.a.to_le_bytes());
        out.extend_from_slice(&self.b.to_le_bytes());
        out.extend_from_slice(&self.c.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// Parse a control message, validating **every** field that crosses the
    /// process boundary. A hostile header can neither make us allocate (the
    /// declared length is checked first) nor panic (a non-UTF-8 payload is an
    /// error, not `unwrap`).
    pub fn decode(bytes: &[u8]) -> RcResult<Self> {
        if bytes.len() < CONTROL_HEADER_LEN {
            return Err(RcError::Launch(format!(
                "AWT control message truncated: {} bytes < {CONTROL_HEADER_LEN}",
                bytes.len()
            )));
        }
        let u32_at =
            |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        let i32_at =
            |o: usize| i32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        let u16_at = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
        let magic = u32_at(0);
        if magic != CONTROL_MAGIC {
            return Err(RcError::Launch(format!(
                "AWT control magic mismatch: {magic:#010x} != {CONTROL_MAGIC:#010x}"
            )));
        }
        let version = u16_at(4);
        if version != CONTROL_VERSION {
            return Err(RcError::Launch(format!(
                "unsupported AWT control version {version} (expected {CONTROL_VERSION})"
            )));
        }
        let code = u16_at(6);
        let kind = AwtControlKind::from_code(code)
            .ok_or_else(|| RcError::Launch(format!("unknown AWT control kind {code}")))?;
        let payload_len = u32_at(24) as usize;
        if payload_len > MAX_CONTROL_TEXT {
            return Err(RcError::Launch(format!(
                "AWT control declares {payload_len} text bytes (limit {MAX_CONTROL_TEXT})"
            )));
        }
        if bytes.len() < CONTROL_HEADER_LEN + payload_len {
            return Err(RcError::Launch(format!(
                "AWT control payload truncated: {} of {} bytes",
                bytes.len().saturating_sub(CONTROL_HEADER_LEN),
                payload_len
            )));
        }
        let text =
            std::str::from_utf8(&bytes[CONTROL_HEADER_LEN..CONTROL_HEADER_LEN + payload_len])
                .map_err(|e| RcError::Launch(format!("AWT control text is not valid UTF-8: {e}")))?
                .to_string();
        Ok(Self {
            kind,
            seq: u32_at(8),
            a: i32_at(12),
            b: i32_at(16),
            c: i32_at(20),
            flags: u32_at(28),
            text,
        })
    }

    /// JSON for the UI: kind-specific keys so the Kotlin side never has to know
    /// what `a` / `b` / `c` mean for a given kind.
    pub fn to_json(&self) -> serde_json::Value {
        let mut value = serde_json::json!({
            "kind": self.kind.id(),
            "seq": self.seq,
        });
        let obj = value.as_object_mut().expect("json! built an object");
        match self.kind {
            AwtControlKind::Cursor => {
                obj.insert(
                    "cursor".to_string(),
                    serde_json::json!(CursorKind::from_awt_type(self.a).id()),
                );
                obj.insert("awt_type".to_string(), serde_json::json!(self.a));
            }
            AwtControlKind::Title | AwtControlKind::ClipboardSet | AwtControlKind::Bye => {
                obj.insert("text".to_string(), serde_json::json!(self.text));
            }
            AwtControlKind::ScreenSize => {
                obj.insert("width".to_string(), serde_json::json!(self.a));
                obj.insert("height".to_string(), serde_json::json!(self.b));
            }
            AwtControlKind::ImeShow => {
                obj.insert("x".to_string(), serde_json::json!(self.a));
                obj.insert("y".to_string(), serde_json::json!(self.b));
                obj.insert("line_height".to_string(), serde_json::json!(self.c));
            }
            AwtControlKind::WindowOpened => {
                obj.insert("window".to_string(), serde_json::json!(self.a));
                obj.insert("text".to_string(), serde_json::json!(self.text));
            }
            AwtControlKind::WindowClosed => {
                obj.insert("window".to_string(), serde_json::json!(self.a));
            }
            AwtControlKind::ClipboardRequest | AwtControlKind::Beep | AwtControlKind::ImeHide => {}
        }
        value
    }
}

// ---------------------------------------------------------------------------
// launcher -> JVM control records (fixed length, chunked text)
// ---------------------------------------------------------------------------

/// Reserved [`AwtEventRecord::id`] that marks a *control* record.
///
/// It is far outside every `java.awt.event.*Event` id range (AWT ids are small
/// positive integers below 3000), so a JVM-side reader can dispatch on it with a
/// single comparison and older readers simply ignore it — a control record can
/// never be mistaken for an input event and injected into the event queue.
pub const CONTROL_EVENT_ID: i32 = 0x7263_0001;

/// Text bytes one control record carries (the two trailing `i32` fields).
pub const CONTROL_CHUNK_BYTES: usize = 8;

/// Upper bound for a clipboard answer, in UTF-8 bytes. 8 KiB is far more than a
/// seed / URL / command, and bounds the reverse channel at 32 KiB of records
/// even if the Android clipboard holds a whole document.
pub const MAX_REPLY_TEXT: usize = 8 * 1024;

/// What a control record answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AwtReplyKind {
    /// Clipboard contents for a [`AwtControlKind::ClipboardRequest`].
    Clipboard,
    /// The clipboard is empty / holds no text (`Clipboard.getContents` → null).
    ClipboardEmpty,
    /// Liveness answer: the launcher is still here (keeps a blocking JVM-side
    /// reader from mistaking an idle user for a dead launcher).
    Pong,
}

impl AwtReplyKind {
    /// The on-the-wire code.
    pub fn code(self) -> i32 {
        match self {
            AwtReplyKind::Clipboard => 1,
            AwtReplyKind::ClipboardEmpty => 2,
            AwtReplyKind::Pong => 3,
        }
    }

    /// Parse a wire code.
    pub fn from_code(code: i32) -> Option<AwtReplyKind> {
        Some(match code {
            1 => AwtReplyKind::Clipboard,
            2 => AwtReplyKind::ClipboardEmpty,
            3 => AwtReplyKind::Pong,
            _ => return None,
        })
    }

    /// Stable id for JSON / logs.
    pub fn id(self) -> &'static str {
        match self {
            AwtReplyKind::Clipboard => "clipboard",
            AwtReplyKind::ClipboardEmpty => "clipboard_empty",
            AwtReplyKind::Pong => "pong",
        }
    }
}

/// Encode a reply as a run of fixed-length control records.
///
/// Field mapping (so the JVM side needs no extra parser):
///
/// | field | meaning |
/// |---|---|
/// | `id` | [`CONTROL_EVENT_ID`] |
/// | `x` | [`AwtReplyKind::code`] |
/// | `y` | `seq` of the request being answered |
/// | `button` | chunk index (0-based) |
/// | `key_code` | chunk count |
/// | `key_char` | valid text bytes in this chunk (0…[`CONTROL_CHUNK_BYTES`]) |
/// | `modifiers`,`wheel` | the 8 text bytes, little-endian |
///
/// `text` longer than [`MAX_REPLY_TEXT`] is truncated **on a character
/// boundary**, so the JVM always receives valid UTF-8. An empty text still
/// yields one record: a request must always be answered, or a JVM thread that
/// blocks on `getContents()` would hang for ever.
pub fn encode_control_reply(kind: AwtReplyKind, seq: u32, text: &str) -> Vec<AwtEventRecord> {
    let mut bytes = text.as_bytes();
    if bytes.len() > MAX_REPLY_TEXT {
        let mut cut = MAX_REPLY_TEXT;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        bytes = &text.as_bytes()[..cut];
    }
    let total = bytes.len().div_ceil(CONTROL_CHUNK_BYTES).max(1);
    let mut out = Vec::with_capacity(total);
    for (index, chunk) in bytes
        .chunks(CONTROL_CHUNK_BYTES)
        .chain(std::iter::repeat_n(&[][..], usize::from(bytes.is_empty())))
        .enumerate()
    {
        let mut buf = [0u8; CONTROL_CHUNK_BYTES];
        buf[..chunk.len()].copy_from_slice(chunk);
        out.push(AwtEventRecord {
            id: CONTROL_EVENT_ID,
            x: kind.code(),
            y: seq as i32,
            button: index as i32,
            key_code: total as i32,
            key_char: chunk.len() as u32,
            modifiers: i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            wheel: i32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
        });
    }
    out
}

/// Re-assemble a reply from its records (the JVM-side algorithm, kept here so
/// the contract is unit-tested on both ends of the pipe).
///
/// Returns an error when the run is inconsistent — a wrong chunk order, a
/// missing chunk, a bogus length or non-UTF-8 bytes — rather than handing a torn
/// string to the caller.
pub fn decode_control_reply(records: &[AwtEventRecord]) -> RcResult<(AwtReplyKind, u32, String)> {
    let first = records
        .first()
        .ok_or_else(|| RcError::Launch("empty AWT control reply".to_string()))?;
    if first.id != CONTROL_EVENT_ID {
        return Err(RcError::Launch(format!(
            "record id {} is not an AWT control record",
            first.id
        )));
    }
    let kind = AwtReplyKind::from_code(first.x)
        .ok_or_else(|| RcError::Launch(format!("unknown AWT reply kind {}", first.x)))?;
    let total = first.key_code;
    if total <= 0 || total as usize != records.len() {
        return Err(RcError::Launch(format!(
            "AWT control reply declares {total} chunks but {} arrived",
            records.len()
        )));
    }
    let mut bytes = Vec::with_capacity(records.len() * CONTROL_CHUNK_BYTES);
    for (index, record) in records.iter().enumerate() {
        if record.id != CONTROL_EVENT_ID || record.x != first.x || record.y != first.y {
            return Err(RcError::Launch(
                "AWT control reply mixes records of different replies".to_string(),
            ));
        }
        if record.button != index as i32 || record.key_code != total {
            return Err(RcError::Launch(format!(
                "AWT control reply chunk {} is out of order (index {})",
                index, record.button
            )));
        }
        let len = record.key_char as usize;
        if len > CONTROL_CHUNK_BYTES {
            return Err(RcError::Launch(format!(
                "AWT control reply chunk declares {len} bytes (limit {CONTROL_CHUNK_BYTES})"
            )));
        }
        let mut buf = [0u8; CONTROL_CHUNK_BYTES];
        buf[0..4].copy_from_slice(&record.modifiers.to_le_bytes());
        buf[4..8].copy_from_slice(&record.wheel.to_le_bytes());
        bytes.extend_from_slice(&buf[..len]);
    }
    let text = String::from_utf8(bytes)
        .map_err(|e| RcError::Launch(format!("AWT control reply is not valid UTF-8: {e}")))?;
    Ok((kind, first.y as u32, text))
}

// ===========================================================================
// Tests (control plane; the frame / canvas / viewport / input types are
// exercised from `launch::fakefx`, which owns the session that drives them)
// ===========================================================================

#[cfg(test)]
mod control_tests {
    use super::*;

    #[test]
    fn every_awt_cursor_type_round_trips() {
        for kind in cursor_type::DEFAULT..=cursor_type::MOVE {
            let mapped = CursorKind::from_awt_type(kind);
            assert_eq!(
                mapped.awt_type(),
                kind,
                "cursor type {kind} did not round-trip ({mapped:?})"
            );
            assert!(!mapped.id().is_empty());
        }
    }

    #[test]
    fn unknown_and_custom_cursors_degrade_to_the_arrow() {
        // A bitmap cursor (`CUSTOM_CURSOR`) or a future JDK type must not break
        // the link: the UI just keeps drawing an arrow.
        for kind in [cursor_type::CUSTOM, 99, i32::MIN, i32::MAX] {
            assert_eq!(CursorKind::from_awt_type(kind), CursorKind::Default);
        }
        assert!(CursorKind::Text.is_text());
        assert!(!CursorKind::Hand.is_text());
    }

    #[test]
    fn resize_cursors_keep_their_direction() {
        assert_eq!(
            CursorKind::from_awt_type(cursor_type::SE_RESIZE),
            CursorKind::Resize { dx: 1, dy: 1 }
        );
        assert_eq!(
            CursorKind::from_awt_type(cursor_type::N_RESIZE).id(),
            "n_resize"
        );
        // A direction we never emit still has a sane fallback.
        assert_eq!(
            CursorKind::Resize { dx: 7, dy: 7 }.awt_type(),
            cursor_type::DEFAULT
        );
        assert_eq!(CursorKind::Resize { dx: 7, dy: 7 }.id(), "default");
    }

    #[test]
    fn every_control_kind_round_trips_through_its_code() {
        let kinds = [
            AwtControlKind::Cursor,
            AwtControlKind::Title,
            AwtControlKind::ClipboardSet,
            AwtControlKind::ClipboardRequest,
            AwtControlKind::Beep,
            AwtControlKind::ScreenSize,
            AwtControlKind::ImeShow,
            AwtControlKind::ImeHide,
            AwtControlKind::WindowOpened,
            AwtControlKind::WindowClosed,
            AwtControlKind::Bye,
        ];
        for kind in kinds {
            assert_eq!(AwtControlKind::from_code(kind.code()), Some(kind));
            assert!(!kind.id().is_empty());
        }
        assert_eq!(AwtControlKind::from_code(0), None);
        assert_eq!(AwtControlKind::from_code(4242), None);
    }

    #[test]
    fn control_messages_round_trip_through_the_wire() {
        let cases = vec![
            AwtControl::cursor(CursorKind::Text),
            AwtControl::title("Forge 安装程序"),
            AwtControl::clipboard_set("seed: -4172144997902289642"),
            AwtControl::clipboard_request(9),
            AwtControl::beep(),
            AwtControl::screen_size(1024, 768),
            AwtControl::ime_show(120, 240, 18),
            AwtControl::ime_hide(),
            AwtControl::window_opened(3, "JOptionPane"),
            AwtControl::window_closed(3),
            AwtControl::bye("JVM exited"),
        ];
        for control in cases {
            let wire = control.encode();
            assert_eq!(wire.len(), CONTROL_HEADER_LEN + control.text.len());
            let back = AwtControl::decode(&wire).expect("valid control");
            assert_eq!(back, control);
        }
    }

    #[test]
    fn control_header_shares_the_frame_header_shape() {
        // Same version offset (4) and payload-length offset (24) as `AwtFrame`,
        // which is what lets one stream reader demultiplex both record types.
        let wire = AwtControl::title("abc").encode();
        assert_eq!(CONTROL_HEADER_LEN, FRAME_HEADER_LEN);
        assert_eq!(u16::from_le_bytes([wire[4], wire[5]]), CONTROL_VERSION);
        assert_eq!(
            u32::from_le_bytes([wire[24], wire[25], wire[26], wire[27]]),
            3
        );
        // …and a *different* magic, so neither can be parsed as the other.
        assert_ne!(CONTROL_MAGIC, FRAME_MAGIC);
        assert!(AwtFrame::decode(&wire).is_err());
    }

    #[test]
    fn control_decode_rejects_every_kind_of_garbage() {
        // Truncated header.
        assert!(AwtControl::decode(&[]).is_err());
        assert!(AwtControl::decode(&[0u8; CONTROL_HEADER_LEN - 1]).is_err());
        // Wrong magic.
        let mut wire = AwtControl::beep().encode();
        wire[0] ^= 0xFF;
        assert!(AwtControl::decode(&wire).is_err());
        // Unsupported version.
        let mut wire = AwtControl::beep().encode();
        wire[4] = 9;
        assert!(AwtControl::decode(&wire).is_err());
        // Unknown kind.
        let mut wire = AwtControl::beep().encode();
        wire[6] = 250;
        let err = AwtControl::decode(&wire).unwrap_err().to_string();
        assert!(err.contains("unknown AWT control kind"), "{err}");
        // Absurd payload length: refused *before* allocating.
        let mut wire = AwtControl::beep().encode();
        wire[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = AwtControl::decode(&wire).unwrap_err().to_string();
        assert!(err.contains("limit"), "{err}");
        // Declared longer than delivered.
        let mut wire = AwtControl::title("abc").encode();
        wire[24..28].copy_from_slice(&64u32.to_le_bytes());
        assert!(AwtControl::decode(&wire).is_err());
        // Invalid UTF-8 payload.
        let mut wire = AwtControl::title("").encode();
        wire[24..28].copy_from_slice(&2u32.to_le_bytes());
        wire.extend_from_slice(&[0xFF, 0xFE]);
        let err = AwtControl::decode(&wire).unwrap_err().to_string();
        assert!(err.contains("UTF-8"), "{err}");
    }

    #[test]
    fn control_json_uses_kind_specific_keys() {
        let cursor = AwtControl::cursor(CursorKind::Hand).to_json();
        assert_eq!(cursor["kind"], "cursor");
        assert_eq!(cursor["cursor"], "hand");
        assert_eq!(cursor["awt_type"], 12);

        let ime = AwtControl::ime_show(10, 20, 16).to_json();
        assert_eq!(ime["kind"], "ime_show");
        assert_eq!(ime["x"], 10);
        assert_eq!(ime["y"], 20);
        assert_eq!(ime["line_height"], 16);

        let screen = AwtControl::screen_size(800, 600).to_json();
        assert_eq!(screen["width"], 800);
        assert_eq!(screen["height"], 600);

        let win = AwtControl::window_opened(4, "标题").to_json();
        assert_eq!(win["window"], 4);
        assert_eq!(win["text"], "标题");

        let req = AwtControl::clipboard_request(11).to_json();
        assert_eq!(req["seq"], 11);
        assert!(req.get("text").is_none());
    }

    #[test]
    fn cursor_kind_is_reported_only_for_cursor_messages() {
        assert_eq!(
            AwtControl::cursor(CursorKind::Wait).cursor_kind(),
            Some(CursorKind::Wait)
        );
        assert_eq!(AwtControl::beep().cursor_kind(), None);
    }

    #[test]
    fn control_replies_chunk_and_reassemble() {
        for text in [
            "",
            "a",
            "12345678",
            "123456789",
            "seed: -4172144997902289642",
            "中文剪贴板内容，混合 ASCII 与 emoji 🙂",
        ] {
            let records = encode_control_reply(AwtReplyKind::Clipboard, 7, text);
            assert!(!records.is_empty(), "an answer is always sent");
            assert!(records.iter().all(|r| r.is_control()));
            let (kind, seq, back) = decode_control_reply(&records).expect("reassembles");
            assert_eq!(kind, AwtReplyKind::Clipboard);
            assert_eq!(seq, 7);
            assert_eq!(back, text);
        }
    }

    #[test]
    fn oversized_replies_are_truncated_on_a_char_boundary() {
        // 3 bytes per char, so the cut cannot land on a boundary by luck.
        let text = "中".repeat(MAX_REPLY_TEXT);
        let records = encode_control_reply(AwtReplyKind::Clipboard, 1, &text);
        let (_, _, back) = decode_control_reply(&records).expect("still valid UTF-8");
        assert!(back.len() <= MAX_REPLY_TEXT);
        assert!(text.starts_with(&back));
        assert!(back.chars().all(|c| c == '中'), "no torn character");
    }

    #[test]
    fn reply_reassembly_rejects_a_damaged_run() {
        let mut records = encode_control_reply(AwtReplyKind::Clipboard, 3, "hello world!");
        assert!(records.len() >= 2);
        // Missing chunk.
        let short = &records[..records.len() - 1];
        assert!(decode_control_reply(short).is_err());
        // Reordered.
        let mut swapped = records.clone();
        swapped.swap(0, 1);
        assert!(decode_control_reply(&swapped).is_err());
        // A foreign record spliced in.
        records[1].y = 99;
        assert!(decode_control_reply(&records).is_err());
        // Not a control run at all.
        assert!(decode_control_reply(&[]).is_err());
        assert!(decode_control_reply(&[AwtEventRecord::default()]).is_err());
        // A bogus per-chunk length.
        let mut bad = encode_control_reply(AwtReplyKind::Pong, 0, "xy");
        bad[0].key_char = 99;
        assert!(decode_control_reply(&bad).is_err());
    }

    #[test]
    fn reply_kinds_round_trip_and_control_ids_cannot_collide_with_awt() {
        for kind in [
            AwtReplyKind::Clipboard,
            AwtReplyKind::ClipboardEmpty,
            AwtReplyKind::Pong,
        ] {
            assert_eq!(AwtReplyKind::from_code(kind.code()), Some(kind));
            assert!(!kind.id().is_empty());
        }
        assert_eq!(AwtReplyKind::from_code(0), None);
        // Every AWT event id we can emit stays far below the reserved id, so a
        // control record is never postable as an input event.
        for id in [
            event_id::KEY_TYPED,
            event_id::KEY_PRESSED,
            event_id::KEY_RELEASED,
            event_id::MOUSE_CLICKED,
            event_id::MOUSE_WHEEL,
            event_id::COMPONENT_RESIZED,
            event_id::FOCUS_GAINED,
            event_id::FOCUS_LOST,
        ] {
            assert!(id < 3000 && id != CONTROL_EVENT_ID);
        }
        assert!(!AwtEventRecord::default().is_control());
    }
}
