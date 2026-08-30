//! Reproduction + verification of the rotation input bug (task 9).
//!
//! Rotating the phone used to make touches land in the wrong place. The reason is
//! that pointer samples travel in **surface** pixels and are mapped to *desktop*
//! pixels by the core with whatever viewport it currently has:
//!
//! ```text
//!   finger down at (600,450) on a 1200x900 surface   -> desktop (320,240)  ✔
//!   ... device rotates, surface becomes 900x1200 ...
//!   the same queued sample, mapped afterwards        -> desktop (426,133)  ✘
//! ```
//!
//! Two mechanisms fix that, and this demo asserts both against the real core:
//!
//! 1. the UI flushes queued samples *before* publishing the new geometry
//!    (`AwtSurfaceViewModel.onSurfaceSizeChanged`), so every sample is mapped with
//!    the viewport it was taken in — reproduced here by comparing the two orders;
//! 2. `AwtSession::set_surface_size` treats a quarter turn as a gesture
//!    invalidation and releases whatever was held, so no button stays stuck and no
//!    drag continues from a coordinate the finger no longer has.
//!
//! ```bash
//! cargo run --example rotation_demo
//! ```

use rc_launcher::display::{rotation_flips, ScreenOrientation};
use rc_launcher::launch::awt::event_id;
use rc_launcher::launch::{
    AwtSession, AwtSessionConfig, MouseButton, PointerPhase, Viewport, WindowSize,
};

fn size(width: u32, height: u32) -> WindowSize {
    WindowSize { width, height }
}

/// A 640x480 AWT desktop shown on a `surface`-sized Compose canvas.
fn session(surface: WindowSize) -> AwtSession {
    AwtSession::open(AwtSessionConfig::new(size(640, 480), surface)).expect("session")
}

fn main() {
    let wide = size(1200, 900);
    let tall = size(900, 1200);
    let finger = (600.0f32, 450.0f32);

    // ---- 1. Why the order matters -----------------------------------------
    let before = Viewport::new((640, 480), (wide.width, wide.height))
        .map_pointer(finger.0, finger.1)
        .expect("inside the picture");
    let after = Viewport::new((640, 480), (tall.width, tall.height))
        .map_pointer(finger.0, finger.1)
        .expect("inside the picture");
    println!("finger {finger:?} on {wide:?} -> desktop {before:?}");
    println!("finger {finger:?} on {tall:?} -> desktop {after:?}  <- the bug, if the");
    println!("    sample is mapped after the rotation instead of before it");
    assert_ne!(
        before, after,
        "if these were equal there would be nothing to fix"
    );

    // Good order (what the UI does now): flush, then resize.
    let mut good = session(wide);
    good.pointer(PointerPhase::Down, finger.0, finger.1, MouseButton::Left);
    let flushed: Vec<(i32, i32)> = good
        .drain_events()
        .iter()
        .filter(|r| r.id == event_id::MOUSE_PRESSED)
        .map(|r| (r.x, r.y))
        .collect();
    good.set_surface_size(tall.width, tall.height).unwrap();
    println!("flush-then-rotate  -> {flushed:?} (correct)");
    assert_eq!(flushed, vec![(before.0 as i32, before.1 as i32)]);

    // Bad order (the bug): resize, then map the sample the user already made.
    let mut bad = session(wide);
    bad.set_surface_size(tall.width, tall.height).unwrap();
    bad.pointer(PointerPhase::Down, finger.0, finger.1, MouseButton::Left);
    let late: Vec<(i32, i32)> = bad
        .drain_events()
        .iter()
        .filter(|r| r.id == event_id::MOUSE_PRESSED)
        .map(|r| (r.x, r.y))
        .collect();
    println!("rotate-then-flush  -> {late:?} (wrong: {} px off)", {
        let (x, y) = (late[0].0 - before.0 as i32, late[0].1 - before.1 as i32);
        ((x * x + y * y) as f64).sqrt().round() as i64
    });
    assert_ne!(flushed, late);

    // ---- 2. A rotation releases the in-flight gesture ---------------------
    let mut s = session(tall);
    s.pointer(PointerPhase::Down, 450.0, 600.0, MouseButton::Left);
    s.drain_events();
    assert_ne!(s.modifiers(), 0, "a button is held");
    assert!(rotation_flips(
        (tall.width, tall.height),
        (wide.width, wide.height)
    ));
    s.set_surface_size(wide.width, wide.height).unwrap();
    let ids: Vec<i32> = s.drain_events().iter().map(|r| r.id).collect();
    println!(
        "rotation released the gesture: {} (records {:?}), surface is now {:?}",
        s.modifiers() == 0,
        ids,
        s.surface_orientation()
    );
    assert!(ids.contains(&event_id::MOUSE_RELEASED));
    assert_eq!(s.modifiers(), 0);
    assert_eq!(s.stats().surface_rotations, 1);
    assert_eq!(s.surface_orientation(), ScreenOrientation::Landscape);

    // ... but a plain resize (soft keyboard) keeps it alive.
    let mut s = session(tall);
    s.pointer(PointerPhase::Down, 450.0, 600.0, MouseButton::Left);
    s.drain_events();
    // 900x1000 is still portrait, so this is a resize and not a quarter turn.
    s.set_surface_size(tall.width, 1000).unwrap();
    assert!(s.drain_events().is_empty());
    assert_ne!(s.modifiers(), 0, "the keyboard must not drop the drag");
    assert_eq!(s.stats().surface_rotations, 0);
    println!("soft-keyboard resize kept the gesture: true");

    println!("\nrotation demo OK — input stays aligned across a quarter turn");
}
