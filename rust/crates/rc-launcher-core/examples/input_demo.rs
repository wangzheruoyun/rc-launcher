//! End-to-end demo of in-game keyboard & mouse support (task 12).
//!
//! Reproduces, on the host and with neither Android nor a JVM, what a player with
//! a USB mouse and keyboard does — and prints what *both* consumers receive:
//!
//! ```text
//!   physical device        AwtSession (Rust core)         two event queues
//!   ───────────────        ──────────────────────         ────────────────
//!   mouse moved 12 px  ─▶  MouseMotion (sensitivity)  ─▶  AWT  MOUSE_MOVED      → Swing
//!   right button       ─▶  InputBindings (remap)      ─▶  GLFW MOUSE_BUTTON     → Minecraft
//!   W with scancode 17 ─▶  key + scancode forwarding  ─▶  GLFW KEY(87,17,PRESS) → Minecraft
//!   pointer captured   ─▶  free-running cursor        ─▶  ANDROID_TYPE_GRAB_STATE
//! ```
//!
//! ```bash
//! cargo run --example input_demo
//! ```
//!
//! Every line it prints is an assertion in disguise: the demo `assert!`s the
//! properties that made the feature necessary in the first place, so running it
//! is a regression test you can read.
//!
//! * a captured mouse can turn **past** the desktop edge (clamping would cap how
//!   far you can look around) while the AWT pointer stays inside the desktop;
//! * a slow mouse (0.4×) still moves — the sub-pixel remainder is remembered;
//! * a key carries the **physical scancode** Android reported, because
//!   `GLFWKeyCallback` needs it and Minecraft falls back to it for anything its
//!   key enumeration does not cover;
//! * a stray touch does not turn the view while the mouse owns the pointer,
//!   unless the player asked for the mixed mode;
//! * losing focus releases everything held, so the game does not keep walking.

use rc_launcher::launch::awt::{event_id, AwtEventRecord, MouseButton, PointerPhase};
use rc_launcher::launch::fakefx::{AwtSession, AwtSessionConfig};
use rc_launcher::launch::input::{
    game_event, glfw, GameInputEvent, MouseSensitivity, PointerMode, PointerSource,
};
use rc_launcher::launch::options::WindowSize;

/// The virtual desktop / game window used throughout the demo.
const SCREEN: (u32, u32) = (320, 240);
/// The Compose surface (a 2× integer scale, so no letterbox bars get in the way).
const SURFACE: (u32, u32) = (640, 480);

fn main() {
    let mut session = AwtSession::open(AwtSessionConfig::new(
        WindowSize {
            width: SCREEN.0,
            height: SCREEN.1,
        },
        WindowSize {
            width: SURFACE.0,
            height: SURFACE.1,
        },
    ))
    .expect("open session");

    println!("== in-game keyboard & mouse demo (task 12) ==");
    println!("desktop {:?} on surface {:?}", SCREEN, SURFACE);
    println!("settings: {}", session.input_settings().to_json());
    println!();

    absolute_mouse(&mut session);
    captured_mouse(&mut session);
    slow_mouse(&mut session);
    keyboard_with_scancodes(&mut session);
    remapping(&mut session);
    wheel(&mut session);
    hybrid_touch(&mut session);
    focus_loss(&mut session);

    println!();
    println!("session counters : {}", session.to_json()["session"]);
    println!("native counters  : {:?}", session.game_input_stats());
    println!("== every assertion held ==");
}

// ---------------------------------------------------------------------------

/// A released pointer behaves like a desktop mouse: absolute, bounded.
fn absolute_mouse(session: &mut AwtSession) {
    step("a released mouse moves the cursor and stops at the edge");
    session.drain_events();
    session.pointer_relative(30.0, 20.0, PointerSource::Mouse);
    println!("   pointer      : {:?}", session.pointer_position());
    println!("   game cursor  : {:?}", session.game_cursor());
    assert_eq!(session.pointer_position(), (30, 20));
    assert_eq!(session.game_cursor(), (30, 20));

    session.pointer_relative(9_000.0, 0.0, PointerSource::Mouse);
    println!("   after +9000  : {:?}", session.pointer_position());
    assert_eq!(
        session.pointer_position().0,
        SCREEN.0 as i32 - 1,
        "a released pointer must not leave the desktop"
    );
    report(session);
}

/// A captured pointer is the whole point of the feature: you can keep turning.
fn captured_mouse(session: &mut AwtSession) {
    step("capturing the pointer lets the view keep turning");
    session.drain_events();
    session.set_capture(true);
    let grab = native(session)
        .into_iter()
        .find(|e| e.kind == game_event::GRAB_STATE)
        .expect("the game must be told it owns the cursor");
    println!("   grab state   : {}", grab.describe());
    assert_eq!(grab.p0, 1);

    let before = session.game_cursor().0;
    for _ in 0..6 {
        session.pointer_relative(400.0, 0.0, PointerSource::Mouse);
    }
    println!(
        "   awt pointer  : {:?} (clamped to the desktop)",
        session.pointer_position()
    );
    println!(
        "   game cursor  : {:?} (free-running: +{} px)",
        session.game_cursor(),
        session.game_cursor().0 - before
    );
    assert_eq!(session.pointer_position().0, SCREEN.0 as i32 - 1);
    assert_eq!(
        session.game_cursor().0 - before,
        2_400,
        "clamping here would cap how far the player can look around"
    );
    report(session);
    session.set_capture(false);
    session.drain_events();
}

/// A low sensitivity must not look like a dead mouse.
fn slow_mouse(session: &mut AwtSession) {
    step("0.4x sensitivity: sub-pixel motion accumulates instead of vanishing");
    session.set_sensitivity(MouseSensitivity::uniform(0.4));
    session.reset_input();
    let start = session.pointer_position().0;
    let mut queued = Vec::new();
    for _ in 0..5 {
        queued.push(session.pointer_relative(1.0, 0.0, PointerSource::Mouse));
    }
    println!("   records/sample: {queued:?} (0 = remembered, not lost)");
    println!(
        "   travelled     : {} px in 5 samples",
        session.pointer_position().0 - start
    );
    assert_eq!(session.pointer_position().0 - start, 2);
    assert!(queued.contains(&0), "a sub-pixel sample must cost no event");

    session.set_sensitivity(MouseSensitivity::uniform(2.0).with_invert_y(true));
    let before = session.pointer_position();
    session.pointer_relative(0.0, -3.0, PointerSource::Mouse);
    println!(
        "   inverted 2x   : dy -3 -> {:+} px",
        session.pointer_position().1 - before.1
    );
    assert_eq!(session.pointer_position().1 - before.1, 6);
    session.set_sensitivity(MouseSensitivity::default());
    report(session);
}

/// The scancode is what makes non-US layouts and exotic keys work in the game.
fn keyboard_with_scancodes(session: &mut AwtSession) {
    step("a key reaches Swing *and* the game, with its physical scancode");
    session.drain_events();
    // 17 is what `KeyEvent.getScanCode()` reports for W on Android (evdev KEY_W).
    session.key_named("key.keyboard.w", Some(17), true);
    let records = session.drain_events();
    for record in &records {
        println!("   {}", describe(record));
    }
    let key = records
        .iter()
        .filter_map(GameInputEvent::from_record)
        .find(|e| e.kind == game_event::KEY)
        .expect("the game must see the key");
    assert_eq!((key.p0, key.p1, key.p2), ('W' as i32, 17, glfw::PRESS));
    assert!(
        records.iter().any(|r| r.id == event_id::KEY_PRESSED),
        "a Forge installer dialog still needs the AWT event"
    );

    // A synthetic press (an on-screen button) has no scancode: the core fills it.
    session.key_named("key.keyboard.left.shift", None, true);
    let shift = native(session)
        .into_iter()
        .find(|e| e.kind == game_event::KEY)
        .unwrap();
    println!(
        "   synthetic     : {} (scancode from the table)",
        shift.describe()
    );
    assert_eq!((shift.p0, shift.p1), (glfw::KEY_LEFT_SHIFT, 42));
    assert_eq!(shift.p3, glfw::MOD_SHIFT, "the modifier bit rides along");
    session.release_all();
    session.drain_events();
}

/// Remapping applies once, for both consumers.
fn remapping(session: &mut AwtSession) {
    step("remapping E->F and left->right applies to both queues");
    session
        .bind_key("key.keyboard.e", "key.keyboard.f")
        .unwrap();
    session.bind_button(MouseButton::Left, MouseButton::Right);
    session.drain_events();
    session.key_named("key.keyboard.e", Some(18), true);
    session.pointer(PointerPhase::Down, 320.0, 240.0, MouseButton::Left);
    for record in session.drain_events() {
        println!("   {}", describe(&record));
    }
    assert!(session.bind_key("key.keyboard.e", "banana").is_err());
    session.release_all();
    session.clear_bindings();
    session.drain_events();
}

/// AWT and GLFW disagree about which way is up; the core translates.
fn wheel(session: &mut AwtSession) {
    step("the wheel keeps the AWT sign and flips it for GLFW");
    session.drain_events();
    session.scroll(320.0, 240.0, 2);
    for record in session.drain_events() {
        println!("   {}", describe(&record));
    }
}

/// With a mouse plugged in, a palm on the screen must not turn the view.
fn hybrid_touch(session: &mut AwtSession) {
    step("a stray touch is filtered while the pointer is captured");
    let mut settings = session.input_settings().clone();
    settings.pointer_mode = PointerMode::Captured;
    settings.hybrid_touch = false;
    session.set_input_settings(settings);
    session.drain_events();
    let touched = session.pointer_from(
        PointerPhase::Down,
        320.0,
        240.0,
        MouseButton::Left,
        PointerSource::Touch,
    );
    println!(
        "   touch records : {touched} (filtered: {})",
        session.stats().input_filtered
    );
    assert_eq!(touched, 0);
    let moved = session.pointer_relative(5.0, 0.0, PointerSource::Mouse);
    println!("   mouse records : {moved}");
    assert!(moved > 0, "the mouse always gets through");

    let mut settings = session.input_settings().clone();
    settings.hybrid_touch = true;
    session.set_input_settings(settings);
    let touched = session.pointer_from(
        PointerPhase::Down,
        320.0,
        240.0,
        MouseButton::Left,
        PointerSource::Touch,
    );
    println!("   hybrid on     : {touched} records for the same tap");
    assert!(touched > 0);
    session.release_all();
    session.set_capture(false);
    session.drain_events();
}

/// The "possessed game" bug: focus loss must unstick everything.
fn focus_loss(session: &mut AwtSession) {
    step("losing focus releases what the game thinks is held");
    session.key_named("key.keyboard.w", Some(17), true);
    session.pointer(PointerPhase::Down, 320.0, 240.0, MouseButton::Left);
    session.drain_events();
    session.set_focus(false);
    let events = native(session);
    for event in &events {
        println!("   {}", event.describe());
    }
    assert!(events
        .iter()
        .any(|e| e.kind == game_event::KEY && e.p2 == glfw::RELEASE));
    assert!(events
        .iter()
        .any(|e| e.kind == game_event::MOUSE_BUTTON && e.p1 == glfw::RELEASE));
    session.set_focus(true);
    session.drain_events();
}

// ---- helpers --------------------------------------------------------------

fn step(what: &str) {
    println!("-- {what}");
}

fn report(session: &mut AwtSession) {
    let records = session.drain_events();
    println!("   queued        : {} records", records.len());
}

fn native(session: &mut AwtSession) -> Vec<GameInputEvent> {
    session
        .drain_events()
        .iter()
        .filter_map(GameInputEvent::from_record)
        .collect()
}

/// Render one record as the consumer that will read it sees it.
fn describe(record: &AwtEventRecord) -> String {
    match GameInputEvent::from_record(record) {
        Some(event) => format!(
            "game  -> CallbackBridge.sendData({}, \"{}\")   [{}]",
            event.kind,
            event.payload(),
            game_event::name(event.kind)
        ),
        None => format!(
            "swing -> {} at ({},{}) button={} key={} char={} mods={} wheel={}",
            name_of(record.id),
            record.x,
            record.y,
            record.button,
            record.key_code,
            record.key_char,
            record.modifiers,
            record.wheel
        ),
    }
}

fn name_of(id: i32) -> &'static str {
    match id {
        event_id::KEY_TYPED => "KEY_TYPED",
        event_id::KEY_PRESSED => "KEY_PRESSED",
        event_id::KEY_RELEASED => "KEY_RELEASED",
        event_id::MOUSE_CLICKED => "MOUSE_CLICKED",
        event_id::MOUSE_PRESSED => "MOUSE_PRESSED",
        event_id::MOUSE_RELEASED => "MOUSE_RELEASED",
        event_id::MOUSE_MOVED => "MOUSE_MOVED",
        event_id::MOUSE_DRAGGED => "MOUSE_DRAGGED",
        event_id::MOUSE_WHEEL => "MOUSE_WHEEL",
        event_id::FOCUS_GAINED => "FOCUS_GAINED",
        event_id::FOCUS_LOST => "FOCUS_LOST",
        event_id::COMPONENT_RESIZED => "COMPONENT_RESIZED",
        _ => "EVENT",
    }
}
