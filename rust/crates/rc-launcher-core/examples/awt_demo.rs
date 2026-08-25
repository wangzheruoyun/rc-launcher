//! End-to-end demo of the AWT/Swing compatibility layer (task 18, "fakefx").
//!
//! Runs the *whole* pipeline on the host, with no Android and no JVM:
//!
//! ```text
//!   fake "game JVM" thread            launcher (AwtHost)              "Compose"
//!   ──────────────────────            ──────────────────              ─────────
//!   opens awt-frames.rcaf   ──frames──▶ frame pump → AwtSession → poll_frame_into
//!   opens awt-events.rcae   ◀─events──  event pump ← touches / keys ← this thread
//! ```
//!
//! ```bash
//! cargo run --example awt_demo
//! ```
//!
//! It proves the things that matter: a repaint reaches the framebuffer as RGBA,
//! only the *damaged* rows are copied, a corrupt frame does not take the session
//! down, a tap on the canvas arrives at the JVM as a real
//! `java.awt.event.MouseEvent` record — and the whole **control plane** works
//! both ways: the JVM's cursor / title / IME / clipboard messages ride the same
//! frame channel without desynchronising it, and a `Clipboard.getContents()` is
//! answered back over the event channel as a chunked control record.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use rc_launcher::launch::awt::{
    decode_control_reply, event_id, AwtControl, AwtControlKind, AwtEventRecord, AwtFrame,
    AwtReplyKind, AwtTransport, CursorKind, MouseButton, PointerPhase, Rect, EVENT_RECORD_LEN,
    FRAME_HEADER_LEN,
};
use rc_launcher::launch::awt_host::{create_channels, AwtHost, LinkState};
use rc_launcher::launch::fakefx::AwtSessionConfig;
use rc_launcher::launch::options::WindowSize;

const SCREEN: (u32, u32) = (64, 32);
const SURFACE: (u32, u32) = (256, 256); // deliberately a different aspect ratio
/// What the "Android clipboard" holds when the JVM asks to paste.
const ANDROID_CLIPBOARD: &str = "剪贴板：seed 42";

fn main() {
    let dir = std::env::temp_dir().join(format!("rc-awt-demo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create demo dir");
    let transport = AwtTransport::in_dir(&dir);
    create_channels(&transport).expect("mkfifo channels");

    println!("== AWT bridge demo ==");
    println!("frames channel : {}", transport.frames.display());
    println!("events channel : {}", transport.events.display());
    println!("jvm properties : {:?}", transport.jvm_args());

    let mut host = AwtHost::open(AwtSessionConfig::new(
        WindowSize {
            width: SCREEN.0,
            height: SCREEN.1,
        },
        WindowSize {
            width: SURFACE.0,
            height: SURFACE.1,
        },
    ))
    .expect("open session")
    .with_intervals(Duration::from_millis(5), Duration::from_millis(2));

    host.attach_transport(transport.clone()).expect("attach");

    // ---- the "game JVM" ----------------------------------------------------
    let frames_path = transport.frames.clone();
    let events_path = transport.events.clone();
    let jvm = std::thread::spawn(move || {
        let mut frames = std::fs::OpenOptions::new()
            .write(true)
            .open(&frames_path)
            .expect("jvm: open frame channel");

        // 1. a full repaint: a red desktop
        let full = AwtFrame::full(
            1,
            SCREEN.0,
            SCREEN.1,
            vec![0xFFCC_2222; (SCREEN.0 * SCREEN.1) as usize],
        )
        .unwrap();
        frames.write_all(&full.encode()).unwrap();

        // 1b. control messages, interleaved with the pixels on the *same*
        //     channel: a cursor change belongs to the repaint that shows the new
        //     hover state, so the two must stay ordered.
        frames
            .write_all(&AwtControl::cursor(CursorKind::Text).encode())
            .unwrap();
        frames
            .write_all(&AwtControl::window_opened(1, "Forge 安装程序").encode())
            .unwrap();
        frames
            .write_all(&AwtControl::ime_show(32, 16, 12).encode())
            .unwrap();
        frames
            .write_all(&AwtControl::clipboard_set("JVM 复制的文本").encode())
            .unwrap();
        frames.flush().unwrap();
        // Give the "UI" time to consume the full repaint, so the next (partial)
        // one is not coalesced with it — that is what shows the damage-limited
        // upload path off.
        std::thread::sleep(Duration::from_millis(150));

        // 2. a corrupt frame (bad magic, framed correctly): must not be fatal
        let mut bogus = full.encode()[..FRAME_HEADER_LEN].to_vec();
        bogus[0] ^= 0xFF;
        bogus[24..28].copy_from_slice(&0u32.to_le_bytes());
        frames.write_all(&bogus).unwrap();

        // 3. a damage-only repaint: a white 8x4 "cursor" at (16, 8)
        let patch = AwtFrame::partial(
            2,
            SCREEN.0,
            SCREEN.1,
            Rect::new(16, 8, 8, 4),
            vec![0xFFFF_FFFF; 32],
        )
        .unwrap();
        frames.write_all(&patch.encode()).unwrap();
        // 4. a corrupt *control* message (unknown kind): also framed correctly, so
        //    the channel stays aligned and the next record still parses.
        let mut bad_control = AwtControl::beep().encode();
        bad_control[6] = 250;
        frames.write_all(&bad_control).unwrap();
        // 5. `Clipboard.getContents()`: the launcher must answer, or a Swing
        //    thread would block for ever.
        frames
            .write_all(&AwtControl::clipboard_request(7).encode())
            .unwrap();
        frames.flush().unwrap();

        // ... and read what the user did back out of the event channel. The same
        // channel also carries the *answers*, as records with the reserved control
        // id — so this loop is exactly what a real JVM-side bridge does: read 32
        // bytes, dispatch on the id.
        let mut events = std::fs::File::open(&events_path).expect("jvm: open event channel");
        let mut records = Vec::new();
        let mut reply_records: Vec<AwtEventRecord> = Vec::new();
        let mut reply = None;
        let mut buf = [0u8; EVENT_RECORD_LEN];
        while records.len() < 5 || reply.is_none() {
            match events.read_exact(&mut buf) {
                Ok(()) => {
                    let record = AwtEventRecord::decode(&buf).unwrap();
                    if record.is_control() {
                        reply_records.push(record);
                        // The first chunk says how many there are.
                        if reply_records.len() as i32 == reply_records[0].key_code {
                            reply = decode_control_reply(&reply_records).ok();
                            reply_records.clear();
                        }
                    } else {
                        records.push(record);
                    }
                }
                Err(_) => break,
            }
        }
        // Keep the frame channel open until the launcher has consumed everything.
        std::thread::sleep(Duration::from_millis(150));
        (records, reply)
    });

    // ---- the "Compose" side ------------------------------------------------
    let mut framebuffer = vec![0u8; host.session().rgba_len()];
    let full_bytes = framebuffer.len();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut uploads = 0;
    let mut skipped = 0;
    let mut smallest = usize::MAX;
    let mut control_seen = 0usize;
    // One iteration = one "vsync": poll, upload if something changed, else skip.
    while Instant::now() < deadline
        && (host.link_stats().frames_accepted < 2 || host.link_stats().controls_accepted < 5)
    {
        // The control plane is drained *unconditionally*: a cursor change or a
        // paste request repaints nothing, so tying it to a changed frame would
        // stall it on an idle desktop.
        for control in host.drain_control() {
            control_seen += 1;
            match control.kind {
                AwtControlKind::Cursor => println!(
                    "control: cursor -> {}",
                    control.cursor_kind().unwrap_or_default().id()
                ),
                AwtControlKind::WindowOpened => {
                    println!("control: window {} opened ({})", control.a, control.text)
                }
                AwtControlKind::ImeShow => println!(
                    "control: a text field wants input at ({}, {})",
                    control.a, control.b
                ),
                AwtControlKind::ClipboardSet => {
                    println!(
                        "control: the JVM copied {:?} -> Android clipboard",
                        control.text
                    )
                }
                AwtControlKind::ClipboardRequest => {
                    println!("control: the JVM wants to paste (seq {})", control.seq);
                    let queued = host.answer_clipboard(Some(ANDROID_CLIPBOARD));
                    println!("         answered with {queued} control record(s)");
                }
                other => println!("control: {}", other.id()),
            }
        }
        match host.poll_frame_into(&mut framebuffer) {
            Ok(Some((rect, bytes))) => {
                uploads += 1;
                smallest = smallest.min(bytes);
                println!(
                    "upload {uploads}: {}x{}+{}+{} = {bytes} B of {full_bytes} B ({}%)",
                    rect.width,
                    rect.height,
                    rect.x,
                    rect.y,
                    bytes * 100 / full_bytes
                );
            }
            Ok(None) => skipped += 1,
            Err(e) => println!("poll error (never fatal): {e}"),
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    // Drain whatever is left of the last repaint.
    while let Ok(Some((rect, bytes))) = host.poll_frame_into(&mut framebuffer) {
        uploads += 1;
        smallest = smallest.min(bytes);
        println!(
            "upload {uploads}: {}x{}+{}+{} = {bytes} B of {full_bytes} B ({}%)",
            rect.width,
            rect.height,
            rect.x,
            rect.y,
            bytes * 100 / full_bytes
        );
    }
    assert!(
        smallest < full_bytes,
        "a damage-only repaint must cost less than a full upload"
    );

    let px = |x: u32, y: u32| {
        let i = ((y * SCREEN.0 + x) * 4) as usize;
        (
            framebuffer[i],
            framebuffer[i + 1],
            framebuffer[i + 2],
            framebuffer[i + 3],
        )
    };
    println!("pixel (0,0)   = {:?} (expect the red desktop)", px(0, 0));
    println!("pixel (18,9)  = {:?} (expect the white patch)", px(18, 9));

    // Whatever arrived on the last iteration: a paste request is answered as soon
    // as it is seen, because a Swing thread may be blocked inside `getContents()`.
    for control in host.drain_control() {
        control_seen += 1;
        if control.kind == AwtControlKind::ClipboardRequest {
            println!("control: the JVM wants to paste (seq {})", control.seq);
            host.answer_clipboard(Some(ANDROID_CLIPBOARD));
        }
    }

    // A tap in the middle of the *surface*: the 2:1 desktop is letterboxed on a
    // square surface, so the middle of the picture is the middle of the desktop.
    host.pointer(PointerPhase::Down, 128.0, 128.0, MouseButton::Left);
    host.pointer(PointerPhase::Up, 128.0, 128.0, MouseButton::Left);
    host.session().key_down_named("escape");
    host.session().key_up_named("escape");

    println!("-- control projection --");
    println!("cursor        : {}", host.cursor().id());
    println!("window title  : {:?}", host.window_title());
    println!(
        "wants keyboard: {}",
        host.session().control().wants_keyboard()
    );
    println!("jvm copied    : {:?}", host.take_clipboard_out());

    let (records, reply) = jvm.join().expect("jvm thread");
    println!("-- records the JVM received --");
    for r in &records {
        let name = match r.id {
            event_id::MOUSE_PRESSED => "MOUSE_PRESSED",
            event_id::MOUSE_RELEASED => "MOUSE_RELEASED",
            event_id::MOUSE_CLICKED => "MOUSE_CLICKED",
            event_id::KEY_PRESSED => "KEY_PRESSED",
            event_id::KEY_RELEASED => "KEY_RELEASED",
            other => {
                println!("  id={other} x={} y={}", r.x, r.y);
                continue;
            }
        };
        println!(
            "  {name:<15} at ({}, {}) button={} key={} modifiers={}",
            r.x, r.y, r.button, r.key_code, r.modifiers
        );
    }

    println!("-- session --");
    println!("{}", host.describe());
    let link = host.link_stats();
    println!(
        "uploads={uploads} (smallest {smallest} B) skipped={skipped} accepted={} rejected={} \
         events_written={}",
        link.frames_accepted, link.frames_rejected, link.events_written
    );
    println!(
        "controls: seen={control_seen} accepted={} rejected={}",
        link.controls_accepted, link.controls_rejected
    );
    assert_eq!(
        link.frames_rejected, 2,
        "the corrupt frame + control message"
    );
    assert_eq!(link.controls_accepted, 5, "cursor/window/ime/copy/paste");
    assert_eq!(host.cursor(), CursorKind::Text, "the I-beam survived");
    assert_eq!(host.window_title().as_deref(), Some("Forge 安装程序"));
    let (kind, seq, text) = reply.expect("the JVM received a clipboard answer");
    assert_eq!(kind, AwtReplyKind::Clipboard);
    assert_eq!(seq, 7, "answered the request it asked about");
    assert_eq!(text, ANDROID_CLIPBOARD, "reassembled from its chunks");
    println!("clipboard answer reached the JVM intact: {text:?}");
    assert_eq!(
        records.len(),
        5,
        "the JVM saw press + release + click + key down + key up"
    );
    assert!(
        records.iter().all(|r| !r.is_control()),
        "control records must never be posted as input events"
    );
    assert_eq!(records[0].id, event_id::MOUSE_PRESSED);
    assert_eq!(
        (records[0].x, records[0].y),
        (32, 16),
        "letterboxed mapping"
    );

    host.stop_and_join();
    assert_eq!(host.link_state(), LinkState::Ended);
    let _ = std::fs::remove_dir_all(&dir);
    println!("== done ==");
}
