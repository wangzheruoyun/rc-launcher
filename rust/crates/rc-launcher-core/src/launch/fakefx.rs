//! fakefx session runtime (task 18) — the live end of the AWT/Swing bridge.
//!
//! [`awt`](crate::launch::awt) provides the *pieces* of the AWT-on-Android
//! adaptation (which caciocavallo backend a Java version needs, the JVM
//! arguments that activate it, the frame/event wire formats, the off-screen
//! canvas, the viewport and the input translator). This module is the *runtime*
//! that ties them into one object the FFI and Compose can drive:
//!
//! ```text
//!   game JVM (caciocavallo peers)                    Android app
//!   ─────────────────────────────                    ───────────────────────────
//!   Swing/AWT paints an int[] ARGB
//!            │
//!            ▼  32-byte header + damaged pixels
//!   awt_bridge → pipe/socket ──▶ AwtFrameStream::read_frame
//!                                        │
//!                                        ▼
//!                                 AwtSession::submit_frame  (validate → back
//!                                        │                   buffer → present)
//!                                        ▼
//!                            AwtSession::copy_dirty_rgba_into ──▶ Compose Bitmap
//!
//!   AWT event queue ◀── AwtEventWriter ◀── AwtSession::drain_events ◀── touches
//! ```
//!
//! Everything the session does is *fail-soft and accounted for*: a corrupt frame
//! is counted and rejected (never panics, never blits out of bounds), a UI that
//! stops consuming cannot grow the event queue without bound, and a lost focus
//! releases every held button/modifier so nothing stays stuck.
//!
//! ## Why a session at all?
//!
//! The FFI needs a *single* handle with interior state (double buffer, modifier
//! state, damage) that survives across JNI calls, and the Compose side needs a
//! consistent snapshot per frame. Keeping that in one place (instead of asking
//! Kotlin to orchestrate canvas + translator + viewport over the JNI boundary
//! three times per frame) keeps the boundary chatter at *one* call per frame and
//! makes the whole pipeline unit-testable on the host.

use std::collections::VecDeque;
use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use crate::error::{RcError, RcResult};
use crate::launch::awt::{
    event_id, now_millis, AwtBackend, AwtCanvas, AwtEvent, AwtEventRecord, AwtFrame,
    AwtInputTranslator, MouseButton, PointerPhase, Rect, ScaleMode, Viewport, EVENT_RECORD_LEN,
    FRAME_HEADER_LEN, MAX_CANVAS_DIM, OPAQUE_BLACK,
};
use crate::launch::options::WindowSize;
use crate::runtime::JavaVersion;
use crate::util::bufpool::{BufPool, PooledBuf};

/// Default bound for the outbound AWT event queue.
///
/// One second of 120 Hz dragging is ~120 records; 4096 gives the JVM ~30 s of
/// slack before the session starts shedding motion events.
pub const DEFAULT_MAX_PENDING_EVENTS: usize = 4096;

/// Largest frame we are willing to buffer from the JVM stream, in bytes
/// (`8192 × 8192 × 4 B` + header). Anything bigger cannot be a legal frame,
/// so the stream reader refuses it instead of allocating.
pub const MAX_FRAME_BYTES: usize =
    FRAME_HEADER_LEN + (MAX_CANVAS_DIM as usize) * (MAX_CANVAS_DIM as usize) * 4;

/// Default click tolerance in desktop pixels (a finger always jitters).
pub const DEFAULT_CLICK_SLOP: u32 = 8;

// ===========================================================================
// Configuration
// ===========================================================================

/// Everything the UI can decide about an AWT session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AwtSessionConfig {
    /// Virtual AWT desktop size — must match `-Dcacio.managed.screensize`.
    pub screen: WindowSize,
    /// Size of the Compose surface the desktop is drawn on, in pixels.
    pub surface: WindowSize,
    /// How the desktop is fitted into the surface.
    pub scale_mode: ScaleMode,
    /// Click tolerance for the synthesised `MOUSE_CLICKED`.
    pub click_slop: u32,
    /// Upper bound for the outbound event queue.
    pub max_pending_events: usize,
    /// Which caciocavallo backend the game JVM runs (reporting only).
    pub backend: AwtBackend,
}

impl Default for AwtSessionConfig {
    fn default() -> Self {
        Self {
            screen: WindowSize::default(),
            surface: WindowSize::default(),
            scale_mode: ScaleMode::default(),
            click_slop: DEFAULT_CLICK_SLOP,
            max_pending_events: DEFAULT_MAX_PENDING_EVENTS,
            backend: AwtBackend::Headless,
        }
    }
}

impl AwtSessionConfig {
    /// Config for a `screen`-sized desktop shown on a `surface`-sized view.
    pub fn new(screen: WindowSize, surface: WindowSize) -> Self {
        Self {
            screen,
            surface,
            ..Self::default()
        }
    }

    /// Pick the caciocavallo backend a Java version needs (task 18 mapping).
    pub fn for_java(mut self, java: JavaVersion) -> Self {
        self.backend = AwtBackend::for_java(java);
        self
    }

    /// Override the scale mode.
    pub fn with_scale_mode(mut self, mode: ScaleMode) -> Self {
        self.scale_mode = mode;
        self
    }

    /// Clamp every field into a range the session can actually honour, so a
    /// hostile / buggy caller can never make us allocate gigabytes or spin on a
    /// zero-sized canvas. Returns the sanitised copy.
    pub fn sanitized(mut self) -> Self {
        self.screen = clamp_size(self.screen);
        self.surface = clamp_size(self.surface);
        // A slop larger than the screen would turn every drag into a click.
        self.click_slop = self
            .click_slop
            .min(self.screen.width.max(self.screen.height));
        self.max_pending_events = self.max_pending_events.clamp(64, 1 << 20);
        self
    }
}

fn clamp_size(size: WindowSize) -> WindowSize {
    WindowSize {
        width: size.width.clamp(1, MAX_CANVAS_DIM),
        height: size.height.clamp(1, MAX_CANVAS_DIM),
    }
}

// ===========================================================================
// Statistics
// ===========================================================================

/// Session counters, surfaced to the HUD / diagnostics screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SessionStats {
    /// Frames accepted from the JVM.
    pub frames_accepted: u64,
    /// Frames rejected because they failed validation (corrupt / stale size).
    pub frames_rejected: u64,
    /// AWT records queued for the JVM.
    pub events_queued: u64,
    /// AWT records handed to the JVM.
    pub events_drained: u64,
    /// AWT records shed because the JVM stopped reading.
    pub events_dropped: u64,
    /// How often the virtual desktop was resized.
    pub screen_resizes: u64,
    /// How often the Compose surface changed size.
    pub surface_resizes: u64,
}

// ===========================================================================
// The session
// ===========================================================================

/// A live AWT/Swing session: the off-screen desktop plus its input plumbing.
///
/// One session corresponds to one running game JVM. It is *not* internally
/// synchronised — the FFI layer owns it behind a `Mutex`, which keeps the lock
/// discipline in one place (and lets tests drive it without locking at all).
#[derive(Debug, Clone)]
pub struct AwtSession {
    config: AwtSessionConfig,
    canvas: AwtCanvas,
    translator: AwtInputTranslator,
    outbox: VecDeque<AwtEventRecord>,
    stats: SessionStats,
    focused: bool,
    opened_at_ms: u64,
    /// Recycled RGBA framebuffers for the render hot path (task 25 — object
    /// pool so the Compose blit does not allocate a fresh `Vec<u8>` per frame).
    frame_pool: BufPool,
}

/// A pooled RGBA framebuffer handed to the renderer (task 25).
///
/// The bytes are written straight into a recycled buffer (no per-frame
/// allocation). When this value is dropped the buffer returns to the session's
/// pool, so a steady 60 fps blit never pressures the allocator.
pub struct PooledFrame {
    buf: PooledBuf,
    rect: Rect,
    len: usize,
}

impl PooledFrame {
    /// The packed RGBA8888 bytes (Bitmap-compatible layout).
    pub fn as_slice(&self) -> &[u8] {
        self.buf.as_ref()
    }

    /// Mutable access to the packed bytes.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.buf.as_mut().as_mut_slice()
    }

    /// The damaged rectangle this buffer covers.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Number of initialised bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Detach the underlying buffer from the pool (it will NOT be recycled on
    /// drop) — use when the renderer must keep ownership across frames.
    pub fn into_vec(self) -> Vec<u8> {
        self.buf.into_inner()
    }
}

impl AwtSession {
    /// Open a session, allocating the off-screen desktop.
    pub fn open(config: AwtSessionConfig) -> RcResult<Self> {
        let config = config.sanitized();
        let canvas = AwtCanvas::new(config.screen.width, config.screen.height)?;
        Ok(Self {
            translator: AwtInputTranslator::new().with_click_slop(config.click_slop),
            canvas,
            outbox: VecDeque::new(),
            stats: SessionStats::default(),
            focused: true,
            opened_at_ms: now_millis(),
            config,
            frame_pool: BufPool::new(),
        })
    }

    /// Open a session with the default 1280×720 desktop.
    pub fn open_default() -> RcResult<Self> {
        Self::open(AwtSessionConfig::default())
    }

    /// The (sanitised) configuration in force.
    pub fn config(&self) -> AwtSessionConfig {
        self.config
    }

    /// Session counters.
    pub fn stats(&self) -> SessionStats {
        self.stats
    }

    /// The off-screen desktop (read-only access for the consumer side).
    pub fn canvas(&self) -> &AwtCanvas {
        &self.canvas
    }

    /// Virtual desktop size in pixels.
    pub fn screen_size(&self) -> (u32, u32) {
        self.canvas.size()
    }

    /// Compose surface size in pixels.
    pub fn surface_size(&self) -> (u32, u32) {
        (self.config.surface.width, self.config.surface.height)
    }

    /// The viewport that maps between desktop and surface pixels.
    pub fn viewport(&self) -> Viewport {
        Viewport::new(self.canvas.size(), self.surface_size()).with_mode(self.config.scale_mode)
    }

    /// Bytes an RGBA8888 copy of the whole desktop needs.
    pub fn rgba_len(&self) -> usize {
        self.canvas.rgba_len()
    }

    // ---- Geometry ---------------------------------------------------------

    /// The Compose surface changed size (rotation, split screen, …).
    ///
    /// This never touches the desktop: AWT keeps painting at its own resolution
    /// and only the letterboxing changes, so no frame is lost on a rotation.
    pub fn set_surface_size(&mut self, width: u32, height: u32) -> RcResult<()> {
        let next = clamp_size(WindowSize { width, height });
        if next == self.config.surface {
            return Ok(());
        }
        self.config.surface = next;
        self.stats.surface_resizes += 1;
        Ok(())
    }

    /// Change the fitting policy (stretch / fit / crop / 1:1).
    pub fn set_scale_mode(&mut self, mode: ScaleMode) {
        self.config.scale_mode = mode;
    }

    /// Resize the *virtual desktop*: reallocates the canvas and queues a
    /// `COMPONENT_RESIZED` record so the JVM re-lays out its Swing hierarchy at
    /// the new size (mirrors cacio's managed-screen resize).
    pub fn resize_screen(&mut self, width: u32, height: u32) -> RcResult<()> {
        let next = clamp_size(WindowSize { width, height });
        if (next.width, next.height) == self.canvas.size() {
            return Ok(());
        }
        self.canvas.resize(next.width, next.height)?;
        self.config.screen = next;
        self.stats.screen_resizes += 1;
        // Any in-flight gesture refers to the old geometry: drop it.
        let releases = self.translator.release_all();
        self.enqueue(releases);
        let resize = self.translator.translate(AwtEvent::Resize {
            width: next.width,
            height: next.height,
        });
        self.enqueue(resize);
        Ok(())
    }

    // ---- Producer side (JVM → canvas) -------------------------------------

    /// Accept one encoded frame from the JVM bridge and publish it.
    ///
    /// Returns the damaged rectangle the UI has to re-upload (`None` when the
    /// frame was empty). A frame that fails validation increments
    /// [`SessionStats::frames_rejected`] and returns the error, so the UI can
    /// show "the AWT bridge is misbehaving" instead of silently freezing.
    pub fn submit_frame_bytes(&mut self, bytes: &[u8]) -> RcResult<Option<Rect>> {
        match AwtFrame::decode(bytes) {
            Ok(frame) => self.submit_frame(&frame),
            Err(e) => {
                self.stats.frames_rejected += 1;
                Err(e)
            }
        }
    }

    /// Accept an already decoded frame.
    ///
    /// A frame for a *different* desktop size is not an error the caller has to
    /// handle specially: cacio was restarted at a new resolution, so we adopt
    /// the new size (and tell the UI everything is dirty) instead of rejecting
    /// every frame from then on.
    pub fn submit_frame(&mut self, frame: &AwtFrame) -> RcResult<Option<Rect>> {
        if (frame.width, frame.height) != self.canvas.size() {
            self.resize_screen(frame.width, frame.height)?;
        }
        match self.canvas.submit_and_present(frame) {
            Ok(dirty) => {
                self.stats.frames_accepted += 1;
                Ok(dirty)
            }
            Err(e) => {
                self.stats.frames_rejected += 1;
                Err(e)
            }
        }
    }

    /// Paint the whole desktop one colour (e.g. black while AWT starts, or a
    /// dim shade once the JVM exited) and damage everything.
    pub fn fill(&mut self, argb: u32) {
        self.canvas.fill(argb);
    }

    /// Reset the desktop to opaque black (bridge restart).
    pub fn clear(&mut self) {
        self.canvas.fill(OPAQUE_BLACK);
    }

    // ---- Consumer side (canvas → Compose) ---------------------------------

    /// The region published but not yet consumed by the UI.
    pub fn dirty_rect(&self) -> Option<Rect> {
        self.canvas.dirty_rect()
    }

    /// Copy the whole desktop as RGBA8888 (`Bitmap.copyPixelsFromBuffer` layout)
    /// and mark it consumed.
    pub fn copy_rgba_into(&mut self, dst: &mut [u8]) -> RcResult<usize> {
        let n = self.canvas.copy_rgba_into(dst)?;
        self.canvas.take_dirty();
        Ok(n)
    }

    /// Copy *only* the damaged region, tightly packed, and mark it consumed.
    ///
    /// Returns `None` when nothing changed since the last call — the UI can then
    /// skip the upload *and* the recomposition entirely, which is what keeps a
    /// blinking Swing caret from costing a full-screen blit at 60 fps.
    pub fn copy_dirty_rgba_into(&mut self, dst: &mut [u8]) -> RcResult<Option<(Rect, usize)>> {
        let Some(rect) = self.canvas.dirty_rect() else {
            return Ok(None);
        };
        let n = self.canvas.copy_region_rgba_into(rect, dst)?;
        self.canvas.take_dirty();
        Ok(Some((rect, n)))
    }

    /// Refresh a *persistent* full-desktop RGBA framebuffer with whatever
    /// changed, and mark it consumed.
    ///
    /// This is the call the Android UI makes once per vsync: `dst` is the direct
    /// `ByteBuffer` that backs the `Bitmap.Config.ARGB_8888` Compose draws, so
    /// only the damaged rows are converted (cheap) while the bitmap upload stays
    /// a single `copyPixelsFromBuffer` memcpy.
    ///
    /// Returns `None` when nothing changed since the last call, so the UI can
    /// skip both the upload *and* the recomposition.
    pub fn poll_frame_into(&mut self, dst: &mut [u8]) -> RcResult<Option<(Rect, usize)>> {
        let Some(rect) = self.canvas.dirty_rect() else {
            return Ok(None);
        };
        let n = self.canvas.copy_region_into_framebuffer(rect, dst)?;
        self.canvas.take_dirty();
        Ok(Some((rect, n)))
    }

    /// Like [`AwtSession::copy_dirty_rgba_into`] but writes into a *pooled*
    /// buffer that is recycled on drop — the Compose render loop reuses one
    /// allocation across frames instead of allocating a fresh `Vec<u8>` every
    /// vsync (render zero-copy / object pool, task 25).
    pub fn copy_dirty_rgba_into_pooled(&mut self) -> RcResult<Option<PooledFrame>> {
        let Some(rect) = self.canvas.dirty_rect() else {
            return Ok(None);
        };
        let need = rect.width as usize * rect.height as usize * 4;
        let mut buf = self.frame_pool.acquire(need.max(1));
        buf.clear();
        buf.fit(need);
        let n = self.canvas.copy_region_rgba_into(rect, buf.as_mut())?;
        self.canvas.take_dirty();
        Ok(Some(PooledFrame { buf, rect, len: n }))
    }

    /// Pooled variant of [`AwtSession::poll_frame_into`].
    pub fn poll_frame_into_pooled(&mut self) -> RcResult<Option<PooledFrame>> {
        let Some(rect) = self.canvas.dirty_rect() else {
            return Ok(None);
        };
        let need = rect.width as usize * rect.height as usize * 4;
        let mut buf = self.frame_pool.acquire(need.max(1));
        buf.clear();
        buf.fit(need);
        let n = self
            .canvas
            .copy_region_into_framebuffer(rect, buf.as_mut())?;
        self.canvas.take_dirty();
        Ok(Some(PooledFrame { buf, rect, len: n }))
    }

    /// Number of pooled (recyclable) framebuffers currently idle — diagnostic
    /// for the render object-pool (task 25).
    pub fn frame_pool_idle(&self) -> usize {
        self.frame_pool.idle_count()
    }

    // ---- Input side (Compose → JVM) ---------------------------------------

    /// A touch / mouse event in *surface* coordinates.
    ///
    /// Returns how many AWT records were queued (0 when the position is on the
    /// letterbox bars and no button is held).
    pub fn pointer(
        &mut self,
        phase: PointerPhase,
        surface_x: f32,
        surface_y: f32,
        button: MouseButton,
    ) -> usize {
        let viewport = self.viewport();
        let records = self
            .translator
            .pointer_from_surface(&viewport, surface_x, surface_y, phase, button);
        self.enqueue(records)
    }

    /// A scroll gesture in surface coordinates (`ticks > 0` scrolls away).
    pub fn scroll(&mut self, surface_x: f32, surface_y: f32, ticks: i32) -> usize {
        let viewport = self.viewport();
        let Some((x, y)) = viewport.map_pointer(surface_x, surface_y) else {
            return 0;
        };
        let records = self.translator.translate(AwtEvent::Scroll { x, y, ticks });
        self.enqueue(records)
    }

    /// Press a key by `VK_*` code.
    pub fn key_down(&mut self, code: i32) -> usize {
        let records = self.translator.translate(AwtEvent::KeyDown { code });
        self.enqueue(records)
    }

    /// Release a key by `VK_*` code.
    pub fn key_up(&mut self, code: i32) -> usize {
        let records = self.translator.translate(AwtEvent::KeyUp { code });
        self.enqueue(records)
    }

    /// Press a key by *name* (`"escape"`, `"key.keyboard.left.shift"`, …).
    ///
    /// Unknown names degrade to typed text when they are a single character, so
    /// a control layout with an exotic binding still reaches the game.
    pub fn key_down_named(&mut self, name: &str) -> usize {
        match self.translator.press_named(name) {
            Some(records) => self.enqueue(records),
            None => self.type_fallback(name),
        }
    }

    /// Release a key by name (unknown names are a no-op: nothing was pressed).
    pub fn key_up_named(&mut self, name: &str) -> usize {
        match self.translator.release_named(name) {
            Some(records) => self.enqueue(records),
            None => 0,
        }
    }

    /// Commit text from the soft keyboard / IME as `KEY_TYPED` records.
    pub fn type_text(&mut self, text: &str) -> usize {
        let records = self.translator.type_str(text);
        self.enqueue(records)
    }

    /// The canvas gained or lost focus. Losing focus releases everything held.
    pub fn set_focus(&mut self, gained: bool) -> usize {
        self.focused = gained;
        let records = self.translator.translate(AwtEvent::Focus { gained });
        self.enqueue(records)
    }

    /// Whether the canvas currently has focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Release every held button / modifier (app went to the background).
    pub fn release_all(&mut self) -> usize {
        let records = self.translator.release_all();
        self.enqueue(records)
    }

    /// Number of records waiting for the JVM.
    pub fn pending_events(&self) -> usize {
        self.outbox.len()
    }

    /// Current `getModifiersEx()` value (held modifiers + buttons).
    pub fn modifiers(&self) -> i32 {
        self.translator.modifiers()
    }

    /// Last pointer position in desktop pixels.
    pub fn pointer_position(&self) -> (i32, i32) {
        self.translator.pointer()
    }

    /// Take every queued record (FIFO).
    pub fn drain_events(&mut self) -> Vec<AwtEventRecord> {
        let out: Vec<AwtEventRecord> = self.outbox.drain(..).collect();
        self.stats.events_drained += out.len() as u64;
        out
    }

    /// Take every queued record, already encoded for the JVM (one write).
    pub fn drain_encoded(&mut self) -> Vec<u8> {
        AwtEventRecord::encode_batch(&self.drain_events())
    }

    /// Forget all input state and queued records (bridge restart), keeping the
    /// pixels so the UI does not flash.
    pub fn reset_input(&mut self) {
        self.translator.reset();
        self.outbox.clear();
    }

    // ---- Reporting --------------------------------------------------------

    /// Milliseconds since the session was opened.
    pub fn uptime_ms(&self) -> u64 {
        now_millis().saturating_sub(self.opened_at_ms)
    }

    /// JSON snapshot for the FFI / diagnostics screen.
    pub fn to_json(&self) -> serde_json::Value {
        let p = self.viewport().placement();
        let (sw, sh) = self.surface_size();
        serde_json::json!({
            "backend": self.config.backend.id(),
            "screen": { "width": self.canvas.width(), "height": self.canvas.height() },
            "surface": { "width": sw, "height": sh },
            "scale_mode": self.config.scale_mode,
            "placement": { "x": p.x, "y": p.y, "width": p.width, "height": p.height },
            "focused": self.focused,
            "modifiers": self.modifiers(),
            "pending_events": self.outbox.len(),
            "rgba_len": self.rgba_len(),
            "uptime_ms": self.uptime_ms(),
            "canvas": self.canvas.stats_json(),
            "session": {
                "frames_accepted": self.stats.frames_accepted,
                "frames_rejected": self.stats.frames_rejected,
                "events_queued": self.stats.events_queued,
                "events_drained": self.stats.events_drained,
                "events_dropped": self.stats.events_dropped,
                "screen_resizes": self.stats.screen_resizes,
                "surface_resizes": self.stats.surface_resizes,
            },
        })
    }

    /// One-line human summary (log lines, bug reports).
    pub fn describe(&self) -> String {
        let (w, h) = self.canvas.size();
        let (sw, sh) = self.surface_size();
        format!(
            "AWT session: {}, desktop {w}x{h} on {sw}x{sh} surface, {} frames ({} rejected), {} events queued ({} dropped), {:.1} fps",
            self.config.backend.id(),
            self.stats.frames_accepted,
            self.stats.frames_rejected,
            self.stats.events_queued,
            self.stats.events_dropped,
            self.canvas.fps(),
        )
    }

    // ---- Internals --------------------------------------------------------

    /// Queue records, shedding load when the JVM stopped reading.
    ///
    /// Motion records (`MOUSE_MOVED` / `MOUSE_DRAGGED` / `MOUSE_WHEEL`) are
    /// dropped *first*: they are transient, whereas a lost press/release or key
    /// event would leave the game with a stuck button. Only when no motion
    /// record is left do we drop the oldest record of any kind.
    fn enqueue(&mut self, records: Vec<AwtEventRecord>) -> usize {
        let queued = records.len();
        for record in records {
            if self.outbox.len() >= self.config.max_pending_events {
                self.shed_one();
            }
            self.outbox.push_back(record);
        }
        self.stats.events_queued += queued as u64;
        queued
    }

    fn shed_one(&mut self) {
        let motion = self.outbox.iter().position(|r| {
            matches!(
                r.id,
                event_id::MOUSE_MOVED | event_id::MOUSE_DRAGGED | event_id::MOUSE_WHEEL
            )
        });
        match motion {
            Some(i) => {
                self.outbox.remove(i);
            }
            None => {
                self.outbox.pop_front();
            }
        }
        self.stats.events_dropped += 1;
    }

    /// A single-character key name we have no `VK_*` for still reaches the game
    /// as typed text.
    fn type_fallback(&mut self, name: &str) -> usize {
        let mut chars = name.chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) => {
                let records = self.translator.type_str(&ch.to_string());
                self.enqueue(records)
            }
            _ => 0,
        }
    }
}

// ===========================================================================
// Frame transport: reading the JVM's stream
// ===========================================================================

/// Reads length-prefixed AWT frames off a byte stream (the pipe / socket the
/// JVM-side `awt_bridge` writes to).
///
/// The frame header is self-describing (`payload_len` at offset 24), so the
/// reader needs no extra framing: it reads the 32-byte header, sanity-checks the
/// declared payload length *before* allocating, then reads exactly that many
/// bytes and validates the whole frame through [`AwtFrame::decode`].
/// Outcome of one [`AwtFrameStream::read_next`] call.
///
/// The distinction matters for the long-lived pump thread
/// ([`crate::launch::awt_host`]): a frame that arrived *intact on the wire* but
/// failed validation leaves the stream **aligned**, so the next frame can still
/// be read; a truncated / absurd header desynchronises the stream and is fatal.
/// Collapsing both into one error (as [`AwtFrameStream::read_frame`] must, for
/// its `Option` signature) would tear down a whole game session because cacio
/// emitted one bad repaint.
#[derive(Debug)]
pub enum FrameRead {
    /// A valid frame, ready to be submitted to a session.
    Frame(AwtFrame),
    /// A framed-but-invalid payload. The stream is still aligned: keep pumping.
    Rejected(RcError),
    /// Clean end of stream between frames (the JVM exited).
    Eof,
}

#[derive(Debug)]
pub struct AwtFrameStream<R: Read> {
    inner: R,
    header: [u8; FRAME_HEADER_LEN],
    payload: Vec<u8>,
    frames_read: u64,
}

impl<R: Read> AwtFrameStream<R> {
    /// Wrap a reader.
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            header: [0u8; FRAME_HEADER_LEN],
            payload: Vec::new(),
            frames_read: 0,
        }
    }

    /// Frames successfully read so far.
    pub fn frames_read(&self) -> u64 {
        self.frames_read
    }

    /// Unwrap the underlying reader.
    pub fn into_inner(self) -> R {
        self.inner
    }

    /// Read the next frame, distinguishing a *recoverable* bad frame from a
    /// fatal desync (see [`FrameRead`]).
    ///
    /// The declared payload length is checked **before** allocating, so a hostile
    /// header cannot make us reserve gigabytes.
    pub fn read_next(&mut self) -> RcResult<FrameRead> {
        match read_full(&mut self.inner, &mut self.header)? {
            0 => return Ok(FrameRead::Eof), // clean EOF
            n if n < FRAME_HEADER_LEN => {
                return Err(RcError::Launch(format!(
                    "AWT frame stream ended mid-header ({n} of {FRAME_HEADER_LEN} bytes)"
                )))
            }
            _ => {}
        }
        let payload_len = u32::from_le_bytes([
            self.header[24],
            self.header[25],
            self.header[26],
            self.header[27],
        ]) as usize;
        if payload_len > MAX_FRAME_BYTES {
            return Err(RcError::Launch(format!(
                "AWT frame declares {payload_len} payload bytes (limit {MAX_FRAME_BYTES})"
            )));
        }
        self.payload.clear();
        self.payload.resize(payload_len, 0);
        let got = read_full(&mut self.inner, &mut self.payload)?;
        if got < payload_len {
            return Err(RcError::Launch(format!(
                "AWT frame stream ended mid-payload ({got} of {payload_len} bytes)"
            )));
        }
        let mut buf = Vec::with_capacity(FRAME_HEADER_LEN + payload_len);
        buf.extend_from_slice(&self.header);
        buf.extend_from_slice(&self.payload);
        // The frame occupied exactly `FRAME_HEADER_LEN + payload_len` bytes, so
        // whatever `decode` thinks of its contents the *stream* stays aligned.
        match AwtFrame::decode(&buf) {
            Ok(frame) => {
                self.frames_read += 1;
                Ok(FrameRead::Frame(frame))
            }
            Err(e) => Ok(FrameRead::Rejected(e)),
        }
    }

    /// Read the next frame.
    ///
    /// Returns `Ok(None)` at a clean end of stream (the JVM exited between
    /// frames), and an [`RcError`] for a truncated or bogus frame. Use
    /// [`AwtFrameStream::read_next`] when a single invalid frame must not end
    /// the session.
    pub fn read_frame(&mut self) -> RcResult<Option<AwtFrame>> {
        match self.read_next()? {
            FrameRead::Frame(f) => Ok(Some(f)),
            FrameRead::Rejected(e) => Err(e),
            FrameRead::Eof => Ok(None),
        }
    }

    /// Pump every frame the stream still holds into a session.
    ///
    /// Returns `(accepted, rejected)`. A rejected frame does **not** abort the
    /// pump: cacio may recover on the next repaint, and a single corrupt frame
    /// must not take the whole game session down (task 19 robustness).
    pub fn pump_into(&mut self, session: &mut AwtSession) -> RcResult<(u64, u64)> {
        let mut accepted = 0u64;
        let mut rejected = 0u64;
        loop {
            match self.read_frame() {
                Ok(Some(frame)) => match session.submit_frame(&frame) {
                    Ok(_) => accepted += 1,
                    Err(_) => rejected += 1,
                },
                Ok(None) => return Ok((accepted, rejected)),
                Err(e) => {
                    if accepted == 0 && rejected == 0 {
                        return Err(e);
                    }
                    rejected += 1;
                    return Ok((accepted, rejected));
                }
            }
        }
    }
}

/// Read until `buf` is full or the stream ends; returns how many bytes were
/// read. `ErrorKind::Interrupted` is retried (signals are normal on Android).
fn read_full<R: Read>(reader: &mut R, buf: &mut [u8]) -> RcResult<usize> {
    let mut filled = 0usize;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(RcError::Io(e)),
        }
    }
    Ok(filled)
}

// ===========================================================================
// Event transport: writing to the JVM
// ===========================================================================

/// Writes drained [`AwtEventRecord`]s to the JVM side of the bridge.
///
/// One `write_all` per UI frame (not per event) keeps the syscall count at ~1
/// per frame even while dragging.
#[derive(Debug)]
pub struct AwtEventWriter<W: Write> {
    inner: W,
    records_written: u64,
}

impl<W: Write> AwtEventWriter<W> {
    /// Wrap a writer.
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            records_written: 0,
        }
    }

    /// Records written so far.
    pub fn records_written(&self) -> u64 {
        self.records_written
    }

    /// Unwrap the underlying writer.
    pub fn into_inner(self) -> W {
        self.inner
    }

    /// Write a batch (no-op for an empty batch, so an idle frame costs nothing).
    pub fn write_records(&mut self, records: &[AwtEventRecord]) -> RcResult<usize> {
        if records.is_empty() {
            return Ok(0);
        }
        let bytes = AwtEventRecord::encode_batch(records);
        self.inner.write_all(&bytes)?;
        self.inner.flush()?;
        self.records_written += records.len() as u64;
        Ok(records.len() * EVENT_RECORD_LEN)
    }

    /// Drain a session's queue straight onto the wire.
    pub fn flush_session(&mut self, session: &mut AwtSession) -> RcResult<usize> {
        let records = session.drain_events();
        self.write_records(&records)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::awt::{mask, vk, PixelFormat};
    use std::io::Cursor;

    fn size(width: u32, height: u32) -> WindowSize {
        WindowSize { width, height }
    }

    /// A 320×240 desktop shown on a 640×480 surface (integer 2× scale, no bars).
    fn session() -> AwtSession {
        AwtSession::open(AwtSessionConfig::new(size(320, 240), size(640, 480))).unwrap()
    }

    fn full_frame(seq: u32, w: u32, h: u32, argb: u32) -> AwtFrame {
        AwtFrame::full(seq, w, h, vec![argb; (w * h) as usize]).unwrap()
    }

    // ---- Configuration ----------------------------------------------------

    #[test]
    fn default_config_is_a_720p_desktop() {
        let c = AwtSessionConfig::default();
        assert_eq!(c.screen, size(1280, 720));
        assert_eq!(c.scale_mode, ScaleMode::Fit);
        assert_eq!(c.click_slop, DEFAULT_CLICK_SLOP);
        assert_eq!(c.max_pending_events, DEFAULT_MAX_PENDING_EVENTS);
        assert_eq!(c.backend, AwtBackend::Headless);
    }

    #[test]
    fn config_for_java_picks_the_cacio_backend() {
        assert_eq!(
            AwtSessionConfig::default()
                .for_java(JavaVersion::Java8)
                .backend,
            AwtBackend::Cacio8
        );
        assert_eq!(
            AwtSessionConfig::default()
                .for_java(JavaVersion::Java17)
                .backend,
            AwtBackend::Cacio17
        );
    }

    #[test]
    fn sanitize_clamps_zero_huge_and_absurd_values() {
        let c = AwtSessionConfig {
            screen: size(0, 0),
            surface: size(99_999, 99_999),
            scale_mode: ScaleMode::Stretch,
            click_slop: 100_000,
            max_pending_events: 1,
            backend: AwtBackend::Cacio17,
        }
        .sanitized();
        assert_eq!(c.screen, size(1, 1), "a zero-sized desktop is impossible");
        assert_eq!(c.surface, size(MAX_CANVAS_DIM, MAX_CANVAS_DIM));
        assert_eq!(c.click_slop, 1, "slop cannot exceed the desktop");
        assert_eq!(c.max_pending_events, 64, "queue bound has a floor");
        assert_eq!(c.scale_mode, ScaleMode::Stretch, "policy is preserved");
    }

    #[test]
    fn open_sanitizes_so_a_bogus_config_cannot_allocate_gigabytes() {
        let s = AwtSession::open(AwtSessionConfig {
            screen: size(0, 4_000_000),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(s.screen_size(), (1, MAX_CANVAS_DIM));
    }

    // ---- Opening ----------------------------------------------------------

    #[test]
    fn open_starts_black_fully_damaged_and_focused() {
        let s = session();
        assert_eq!(s.screen_size(), (320, 240));
        assert_eq!(s.surface_size(), (640, 480));
        assert_eq!(s.rgba_len(), 320 * 240 * 4);
        assert!(s.is_focused());
        assert_eq!(s.pending_events(), 0);
        assert_eq!(s.canvas().pixel(0, 0), Some(OPAQUE_BLACK));
        // The UI has never drawn: everything must be dirty.
        assert_eq!(s.dirty_rect(), Some(Rect::whole(320, 240)));
    }

    #[test]
    fn open_default_is_the_720p_desktop() {
        assert_eq!(
            AwtSession::open_default().unwrap().screen_size(),
            (1280, 720)
        );
    }

    // ---- Producer side ----------------------------------------------------

    #[test]
    fn submit_frame_bytes_publishes_pixels_and_damage() {
        let mut s = session();
        s.copy_dirty_rgba_into(&mut vec![0u8; s.rgba_len()])
            .unwrap(); // consume initial damage
        let frame = full_frame(1, 320, 240, 0xFF10_2030);
        let dirty = s.submit_frame_bytes(&frame.encode()).unwrap();
        assert_eq!(dirty, Some(Rect::whole(320, 240)));
        assert_eq!(s.canvas().pixel(5, 5), Some(0xFF10_2030));
        assert_eq!(s.stats().frames_accepted, 1);
        assert_eq!(s.stats().frames_rejected, 0);
    }

    #[test]
    fn partial_frame_only_damages_its_rectangle() {
        let mut s = session();
        s.submit_frame(&full_frame(1, 320, 240, OPAQUE_BLACK))
            .unwrap();
        s.copy_rgba_into(&mut vec![0u8; s.rgba_len()]).unwrap(); // clean
        let damage = Rect::new(10, 20, 4, 2);
        let frame = AwtFrame::partial(2, 320, 240, damage, vec![0xFFAA_BBCC; 8]).unwrap();
        let dirty = s.submit_frame_bytes(&frame.encode()).unwrap();
        assert_eq!(dirty, Some(damage));
        assert_eq!(s.canvas().pixel(10, 20), Some(0xFFAA_BBCC));
        assert_eq!(
            s.canvas().pixel(9, 20),
            Some(OPAQUE_BLACK),
            "outside untouched"
        );
    }

    #[test]
    fn corrupt_frame_is_counted_and_never_panics() {
        let mut s = session();
        // bad magic
        assert!(s.submit_frame_bytes(&[0u8; FRAME_HEADER_LEN]).is_err());
        // truncated header
        assert!(s.submit_frame_bytes(&[1, 2, 3]).is_err());
        // header ok, payload length lies
        let mut bytes = full_frame(1, 320, 240, 0xFF00_FF00).encode();
        bytes.truncate(FRAME_HEADER_LEN + 8);
        assert!(s.submit_frame_bytes(&bytes).is_err());
        assert_eq!(s.stats().frames_rejected, 3);
        assert_eq!(s.stats().frames_accepted, 0);
    }

    #[test]
    fn frame_for_a_new_desktop_size_adopts_it() {
        let mut s = session();
        let frame = full_frame(7, 800, 600, 0xFF01_0203);
        assert!(s.submit_frame_bytes(&frame.encode()).is_ok());
        assert_eq!(s.screen_size(), (800, 600));
        assert_eq!(s.stats().screen_resizes, 1);
        assert_eq!(s.stats().frames_accepted, 1);
        // The JVM is told about the new geometry.
        let ids: Vec<i32> = s.drain_events().iter().map(|r| r.id).collect();
        assert!(ids.contains(&event_id::COMPONENT_RESIZED));
    }

    #[test]
    fn rgb_frames_are_forced_opaque() {
        let mut s = session();
        let frame = AwtFrame::full(1, 320, 240, vec![0x0011_2233; 320 * 240])
            .unwrap()
            .with_format(PixelFormat::IntRgb);
        s.submit_frame(&frame).unwrap();
        assert_eq!(s.canvas().pixel(0, 0), Some(0xFF11_2233));
    }

    #[test]
    fn fill_and_clear_repaint_everything() {
        let mut s = session();
        s.fill(0xFF12_3456);
        assert_eq!(s.canvas().pixel(1, 1), Some(0xFF12_3456));
        assert_eq!(s.dirty_rect(), Some(Rect::whole(320, 240)));
        s.clear();
        assert_eq!(s.canvas().pixel(1, 1), Some(OPAQUE_BLACK));
    }

    // ---- Geometry ---------------------------------------------------------

    #[test]
    fn resize_screen_releases_gestures_and_announces_the_new_size() {
        let mut s = session();
        s.pointer(PointerPhase::Down, 320.0, 240.0, MouseButton::Left);
        s.key_down(vk::SHIFT);
        s.drain_events();
        s.resize_screen(640, 360).unwrap();
        assert_eq!(s.screen_size(), (640, 360));
        let records = s.drain_events();
        let ids: Vec<i32> = records.iter().map(|r| r.id).collect();
        assert!(
            ids.contains(&event_id::MOUSE_RELEASED),
            "held button released"
        );
        assert!(ids.contains(&event_id::KEY_RELEASED), "held Shift released");
        let resize = records.last().unwrap();
        assert_eq!(resize.id, event_id::COMPONENT_RESIZED);
        assert_eq!((resize.x, resize.y), (640, 360));
        assert_eq!(s.modifiers(), 0, "nothing stays stuck across a resize");
    }

    #[test]
    fn resize_screen_to_the_same_size_is_a_no_op() {
        let mut s = session();
        s.resize_screen(320, 240).unwrap();
        assert_eq!(s.stats().screen_resizes, 0);
        assert_eq!(s.pending_events(), 0);
    }

    #[test]
    fn surface_resize_keeps_the_desktop_and_the_damage() {
        let mut s = session();
        s.submit_frame(&full_frame(1, 320, 240, 0xFF33_4455))
            .unwrap();
        s.set_surface_size(1080, 2400).unwrap();
        assert_eq!(s.surface_size(), (1080, 2400));
        assert_eq!(s.screen_size(), (320, 240), "AWT keeps its own resolution");
        assert_eq!(
            s.canvas().pixel(0, 0),
            Some(0xFF33_4455),
            "no pixel is lost"
        );
        assert_eq!(s.stats().surface_resizes, 1);
        s.set_surface_size(1080, 2400).unwrap();
        assert_eq!(s.stats().surface_resizes, 1, "idempotent");
    }

    #[test]
    fn surface_size_is_clamped_not_rejected() {
        let mut s = session();
        s.set_surface_size(0, 0).unwrap();
        assert_eq!(s.surface_size(), (1, 1));
    }

    #[test]
    fn viewport_letterboxes_a_43_desktop_on_a_tall_phone() {
        let mut s = session();
        s.set_surface_size(1080, 2400).unwrap();
        let p = s.viewport().placement();
        assert_eq!(p.width, 1080, "width limited");
        assert_eq!(p.height, 810, "4:3 of 1080");
        assert_eq!(p.x, 0);
        assert_eq!(p.y, (2400 - 810) / 2);
    }

    #[test]
    fn scale_mode_switches_the_fitting_policy() {
        let mut s = session();
        s.set_surface_size(1000, 500).unwrap();
        s.set_scale_mode(ScaleMode::Stretch);
        let p = s.viewport().placement();
        assert_eq!((p.width, p.height), (1000, 500));
        s.set_scale_mode(ScaleMode::Center);
        let p = s.viewport().placement();
        assert_eq!((p.width, p.height), (320, 240));
        assert_eq!(s.config().scale_mode, ScaleMode::Center);
    }

    // ---- Consumer side ----------------------------------------------------

    #[test]
    fn copy_rgba_converts_argb_to_rgba_and_clears_the_damage() {
        let mut s = session();
        s.submit_frame(&full_frame(1, 320, 240, 0xC0AB_CDEF))
            .unwrap();
        let mut dst = vec![0u8; s.rgba_len()];
        let n = s.copy_rgba_into(&mut dst).unwrap();
        assert_eq!(n, s.rgba_len());
        assert_eq!(&dst[0..4], &[0xAB, 0xCD, 0xEF, 0xC0], "R,G,B,A order");
        assert_eq!(s.dirty_rect(), None, "consumed");
    }

    #[test]
    fn copy_rgba_rejects_a_short_buffer() {
        let mut s = session();
        let err = s.copy_rgba_into(&mut [0u8; 16]).unwrap_err();
        assert!(err.to_string().contains("too small"), "{err}");
        assert!(s.dirty_rect().is_some(), "a failed copy does not consume");
    }

    #[test]
    fn copy_dirty_uploads_only_the_damaged_rows() {
        let mut s = session();
        s.copy_dirty_rgba_into(&mut vec![0u8; s.rgba_len()])
            .unwrap();
        assert!(
            s.copy_dirty_rgba_into(&mut [0u8; 4]).unwrap().is_none(),
            "clean"
        );

        let damage = Rect::new(2, 3, 2, 1);
        let frame = AwtFrame::partial(2, 320, 240, damage, vec![0xFF00_FF00; 2]).unwrap();
        s.submit_frame(&frame).unwrap();
        let mut dst = vec![0u8; 64];
        let (rect, bytes) = s.copy_dirty_rgba_into(&mut dst).unwrap().unwrap();
        assert_eq!(rect, damage);
        assert_eq!(bytes, 2 * 4, "2 px, not the whole 320x240 desktop");
        assert_eq!(&dst[0..4], &[0x00, 0xFF, 0x00, 0xFF]);
        assert!(s.copy_dirty_rgba_into(&mut dst).unwrap().is_none());
    }

    // ---- Input side -------------------------------------------------------

    #[test]
    fn tap_in_the_middle_maps_to_the_desktop_centre() {
        let mut s = session();
        assert_eq!(
            s.pointer(PointerPhase::Down, 320.0, 240.0, MouseButton::Left),
            1
        );
        let r = s.drain_events();
        assert_eq!(r[0].id, event_id::MOUSE_PRESSED);
        assert_eq!((r[0].x, r[0].y), (160, 120));
        assert_eq!(r[0].button, MouseButton::Left.number());
        assert_eq!(r[0].modifiers & mask::BUTTON1_DOWN, mask::BUTTON1_DOWN);
    }

    #[test]
    fn tap_on_the_letterbox_bar_is_not_an_awt_event() {
        let mut s = session();
        s.set_surface_size(1080, 2400).unwrap(); // 810 px tall picture, bars above/below
        assert_eq!(
            s.pointer(PointerPhase::Down, 500.0, 10.0, MouseButton::Left),
            0
        );
        assert_eq!(s.pending_events(), 0);
    }

    #[test]
    fn drag_off_the_picture_keeps_dragging_and_releases_inside() {
        let mut s = session();
        s.set_surface_size(1080, 2400).unwrap();
        s.pointer(PointerPhase::Down, 540.0, 1200.0, MouseButton::Left);
        // Finger wanders onto the black bar: Swing must still receive the drag.
        assert_eq!(
            s.pointer(PointerPhase::Move, 540.0, 5.0, MouseButton::Left),
            1
        );
        let r = s.drain_events();
        assert_eq!(r.last().unwrap().id, event_id::MOUSE_DRAGGED);
        // …and the release must arrive, or the scrollbar stays grabbed forever.
        assert!(s.pointer(PointerPhase::Up, 540.0, 5.0, MouseButton::Left) >= 1);
        let ids: Vec<i32> = s.drain_events().iter().map(|r| r.id).collect();
        assert!(ids.contains(&event_id::MOUSE_RELEASED));
        assert_eq!(s.modifiers(), 0);
    }

    #[test]
    fn a_steady_tap_also_synthesises_mouse_clicked() {
        let mut s = session();
        s.pointer(PointerPhase::Down, 320.0, 240.0, MouseButton::Left);
        s.pointer(PointerPhase::Up, 321.0, 241.0, MouseButton::Left);
        let ids: Vec<i32> = s.drain_events().iter().map(|r| r.id).collect();
        assert_eq!(
            ids,
            vec![
                event_id::MOUSE_PRESSED,
                event_id::MOUSE_RELEASED,
                event_id::MOUSE_CLICKED
            ]
        );
    }

    #[test]
    fn scroll_maps_and_ignores_the_bars() {
        let mut s = session();
        assert_eq!(s.scroll(320.0, 240.0, -3), 1);
        let r = s.drain_events();
        assert_eq!(r[0].id, event_id::MOUSE_WHEEL);
        assert_eq!(r[0].wheel, -3);
        s.set_surface_size(1080, 2400).unwrap();
        assert_eq!(s.scroll(540.0, 1.0, 1), 0);
    }

    #[test]
    fn non_finite_pointer_coordinates_are_dropped() {
        let mut s = session();
        assert_eq!(
            s.pointer(PointerPhase::Down, f32::NAN, 1.0, MouseButton::Left),
            0
        );
        assert_eq!(s.scroll(f32::INFINITY, 0.0, 1), 0);
        assert_eq!(s.pending_events(), 0);
    }

    #[test]
    fn named_keys_track_modifier_state() {
        let mut s = session();
        assert_eq!(s.key_down_named("key.keyboard.left.shift"), 1);
        assert_eq!(s.modifiers() & mask::SHIFT_DOWN, mask::SHIFT_DOWN);
        assert_eq!(s.key_down_named("w"), 1);
        let r = s.drain_events();
        assert_eq!(r[1].key_code, 'W' as i32);
        assert_eq!(r[1].modifiers & mask::SHIFT_DOWN, mask::SHIFT_DOWN);
        assert_eq!(s.key_up_named("shift"), 1);
        assert_eq!(s.modifiers(), 0);
    }

    #[test]
    fn an_unknown_key_name_degrades_to_typed_text() {
        let mut s = session();
        assert_eq!(s.key_down_named("€"), 1, "single char -> KEY_TYPED");
        let r = s.drain_events();
        assert_eq!(r[0].id, event_id::KEY_TYPED);
        assert_eq!(r[0].key_char, '€' as u32);
        assert_eq!(s.key_down_named("no.such.key"), 0, "and never a bogus VK");
        assert_eq!(s.key_up_named("no.such.key"), 0);
    }

    #[test]
    fn type_text_emits_one_key_typed_per_char() {
        let mut s = session();
        assert_eq!(s.type_text("hi 中"), 4);
        let r = s.drain_events();
        assert!(r.iter().all(|x| x.id == event_id::KEY_TYPED));
        assert_eq!(r[3].key_char, '中' as u32);
    }

    #[test]
    fn losing_focus_releases_everything_held() {
        let mut s = session();
        s.pointer(PointerPhase::Down, 320.0, 240.0, MouseButton::Right);
        s.key_down(vk::CONTROL);
        s.drain_events();
        assert!(s.set_focus(false) >= 3);
        assert!(!s.is_focused());
        let ids: Vec<i32> = s.drain_events().iter().map(|r| r.id).collect();
        assert!(ids.contains(&event_id::MOUSE_RELEASED));
        assert!(ids.contains(&event_id::KEY_RELEASED));
        assert_eq!(*ids.last().unwrap(), event_id::FOCUS_LOST);
        assert_eq!(s.modifiers(), 0);
        assert_eq!(s.set_focus(true), 1);
        assert!(s.is_focused());
    }

    #[test]
    fn release_all_is_idempotent_when_nothing_is_held() {
        let mut s = session();
        assert_eq!(s.release_all(), 0);
        s.key_down(vk::ALT);
        s.drain_events();
        assert_eq!(s.release_all(), 1);
        assert_eq!(s.release_all(), 0);
    }

    #[test]
    fn pointer_position_and_modifiers_are_reported() {
        let mut s = session();
        // The far corner *inside* the surface maps to the far desktop pixel …
        s.pointer(PointerPhase::Move, 639.0, 479.0, MouseButton::Left);
        assert_eq!(s.pointer_position(), (319, 239));
        // … while a hover exactly on the exclusive edge is outside the picture and
        // is not forwarded at all (no button held), leaving the position intact.
        assert_eq!(
            s.pointer(PointerPhase::Move, 640.0, 480.0, MouseButton::Left),
            0
        );
        assert_eq!(s.pointer_position(), (319, 239));
        assert_eq!(s.modifiers(), 0, "a hover holds nothing");
    }

    #[test]
    fn reset_input_clears_the_queue_but_not_the_pixels() {
        let mut s = session();
        s.submit_frame(&full_frame(1, 320, 240, 0xFF99_8877))
            .unwrap();
        s.key_down(vk::SHIFT);
        s.reset_input();
        assert_eq!(s.pending_events(), 0);
        assert_eq!(s.modifiers(), 0);
        assert_eq!(s.canvas().pixel(0, 0), Some(0xFF99_8877));
    }

    // ---- Event queue bounds ----------------------------------------------

    #[test]
    fn a_stalled_jvm_sheds_motion_records_first() {
        let mut s = AwtSession::open(AwtSessionConfig {
            screen: size(320, 240),
            surface: size(320, 240),
            max_pending_events: 64, // sanitize floor
            ..Default::default()
        })
        .unwrap();
        // One press (state carrying) then a flood of moves.
        s.pointer(PointerPhase::Down, 10.0, 10.0, MouseButton::Left);
        for i in 0..200 {
            s.pointer(
                PointerPhase::Move,
                10.0 + i as f32 % 100.0,
                20.0,
                MouseButton::Left,
            );
        }
        assert_eq!(s.pending_events(), 64, "queue stays bounded");
        assert!(s.stats().events_dropped > 0);
        let ids: Vec<i32> = s.drain_events().iter().map(|r| r.id).collect();
        assert!(
            ids.contains(&event_id::MOUSE_PRESSED),
            "the press survived the flood (no stuck button)"
        );
    }

    #[test]
    fn when_only_state_records_remain_the_oldest_is_dropped() {
        let mut s = AwtSession::open(AwtSessionConfig {
            screen: size(64, 64),
            surface: size(64, 64),
            max_pending_events: 64,
            ..Default::default()
        })
        .unwrap();
        for _ in 0..100 {
            s.type_text("x");
        }
        assert_eq!(s.pending_events(), 64);
        assert_eq!(s.stats().events_dropped, 36);
        assert_eq!(s.stats().events_queued, 100);
    }

    #[test]
    fn drain_encoded_roundtrips_through_the_wire_format() {
        let mut s = session();
        s.pointer(PointerPhase::Down, 320.0, 240.0, MouseButton::Middle);
        s.type_text("ok");
        let bytes = s.drain_encoded();
        assert_eq!(bytes.len(), 3 * EVENT_RECORD_LEN);
        let decoded = AwtEventRecord::decode_batch(&bytes).unwrap();
        assert_eq!(decoded[0].id, event_id::MOUSE_PRESSED);
        assert_eq!(decoded[2].key_char, 'k' as u32);
        assert_eq!(s.pending_events(), 0);
        assert_eq!(s.stats().events_drained, 3);
        assert!(s.drain_encoded().is_empty());
    }

    // ---- Reporting --------------------------------------------------------

    #[test]
    fn json_snapshot_carries_geometry_and_counters() {
        let mut s = AwtSession::open(
            AwtSessionConfig::new(size(320, 240), size(1080, 2400)).for_java(JavaVersion::Java17),
        )
        .unwrap();
        s.submit_frame(&full_frame(1, 320, 240, 0xFF00_0000))
            .unwrap();
        s.pointer(PointerPhase::Down, 540.0, 1200.0, MouseButton::Left);
        let j = s.to_json();
        assert_eq!(j["backend"], "cacio17");
        assert_eq!(j["screen"]["width"], 320);
        assert_eq!(j["surface"]["height"], 2400);
        assert_eq!(j["scale_mode"], "fit");
        assert_eq!(j["placement"]["height"], 810);
        assert_eq!(j["pending_events"], 1);
        assert_eq!(j["rgba_len"], 320 * 240 * 4);
        assert_eq!(j["session"]["frames_accepted"], 1);
        assert_eq!(j["canvas"]["width"], 320);
        assert_eq!(j["focused"], true);
    }

    #[test]
    fn describe_is_a_single_useful_line() {
        let s = session();
        let d = s.describe();
        assert!(!d.contains('\n'));
        assert!(d.contains("320x240"), "{d}");
        assert!(d.contains("headless"), "{d}");
    }

    // ---- Frame stream -----------------------------------------------------

    #[test]
    fn frame_stream_reads_a_sequence_then_reports_eof() {
        let mut wire = Vec::new();
        wire.extend_from_slice(&full_frame(1, 8, 4, 0xFF11_1111).encode());
        wire.extend_from_slice(
            &AwtFrame::partial(2, 8, 4, Rect::new(1, 1, 2, 2), vec![0xFF22_2222; 4])
                .unwrap()
                .encode(),
        );
        let mut stream = AwtFrameStream::new(Cursor::new(wire));
        assert_eq!(stream.read_frame().unwrap().unwrap().seq, 1);
        let second = stream.read_frame().unwrap().unwrap();
        assert_eq!(second.seq, 2);
        assert_eq!(second.damage, Rect::new(1, 1, 2, 2));
        assert!(stream.read_frame().unwrap().is_none(), "clean EOF");
        assert_eq!(stream.frames_read(), 2);
    }

    #[test]
    fn frame_stream_detects_truncation() {
        // mid-header
        let mut half = full_frame(1, 8, 4, 0xFF00_0000).encode();
        half.truncate(FRAME_HEADER_LEN - 4);
        let err = AwtFrameStream::new(Cursor::new(half))
            .read_frame()
            .unwrap_err();
        assert!(err.to_string().contains("mid-header"), "{err}");

        // mid-payload
        let mut short = full_frame(1, 8, 4, 0xFF00_0000).encode();
        short.truncate(FRAME_HEADER_LEN + 8);
        let err = AwtFrameStream::new(Cursor::new(short))
            .read_frame()
            .unwrap_err();
        assert!(err.to_string().contains("mid-payload"), "{err}");
    }

    #[test]
    fn frame_stream_refuses_an_absurd_payload_length_without_allocating() {
        let mut header = [0u8; FRAME_HEADER_LEN];
        header[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = AwtFrameStream::new(Cursor::new(header.to_vec()))
            .read_frame()
            .unwrap_err();
        assert!(err.to_string().contains("limit"), "{err}");
    }

    #[test]
    fn frame_stream_pumps_a_whole_burst_into_a_session() {
        let mut s = AwtSession::open(AwtSessionConfig::new(size(8, 4), size(8, 4))).unwrap();
        let mut wire = Vec::new();
        for seq in 1..=5 {
            wire.extend_from_slice(&full_frame(seq, 8, 4, 0xFF00_0000 | seq).encode());
        }
        let mut stream = AwtFrameStream::new(Cursor::new(wire));
        assert_eq!(stream.pump_into(&mut s).unwrap(), (5, 0));
        assert_eq!(s.stats().frames_accepted, 5);
        assert_eq!(s.canvas().pixel(0, 0), Some(0xFF00_0005));
    }

    #[test]
    fn a_single_corrupt_frame_does_not_kill_the_pump() {
        let mut s = AwtSession::open(AwtSessionConfig::new(size(8, 4), size(8, 4))).unwrap();
        let mut wire = full_frame(1, 8, 4, 0xFF00_0001).encode();
        wire.extend_from_slice(&[0xEE; 8]); // garbage tail -> mid-header truncation
        let mut stream = AwtFrameStream::new(Cursor::new(wire));
        let (accepted, rejected) = stream.pump_into(&mut s).unwrap();
        assert_eq!(accepted, 1);
        assert_eq!(rejected, 1);
        assert_eq!(
            s.canvas().pixel(0, 0),
            Some(0xFF00_0001),
            "good frame survived"
        );
    }

    #[test]
    fn a_stream_that_is_garbage_from_the_start_is_an_error() {
        let mut s = AwtSession::open(AwtSessionConfig::new(size(8, 4), size(8, 4))).unwrap();
        let mut stream = AwtFrameStream::new(Cursor::new(vec![0xAB; FRAME_HEADER_LEN]));
        assert!(stream.pump_into(&mut s).is_err());
    }

    #[test]
    fn frame_stream_survives_a_reader_that_dribbles_bytes() {
        /// A reader that returns at most one byte per call (worst-case pipe).
        struct Dribble(Vec<u8>, usize);
        impl Read for Dribble {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.1 >= self.0.len() || buf.is_empty() {
                    return Ok(0);
                }
                buf[0] = self.0[self.1];
                self.1 += 1;
                Ok(1)
            }
        }
        let wire = full_frame(9, 8, 4, 0xFF44_5566).encode();
        let mut stream = AwtFrameStream::new(Dribble(wire, 0));
        assert_eq!(stream.read_frame().unwrap().unwrap().seq, 9);
    }

    // ---- Event writer -----------------------------------------------------

    #[test]
    fn event_writer_writes_batches_and_skips_empty_ones() {
        let mut w = AwtEventWriter::new(Vec::new());
        assert_eq!(w.write_records(&[]).unwrap(), 0);
        let records = vec![AwtEventRecord {
            id: event_id::KEY_TYPED,
            key_char: 'z' as u32,
            ..Default::default()
        }];
        assert_eq!(w.write_records(&records).unwrap(), EVENT_RECORD_LEN);
        assert_eq!(w.records_written(), 1);
        let bytes = w.into_inner();
        assert_eq!(
            AwtEventRecord::decode_batch(&bytes).unwrap()[0].key_char,
            'z' as u32
        );
    }

    #[test]
    fn event_writer_flushes_a_session_queue() {
        let mut s = session();
        s.pointer(PointerPhase::Down, 320.0, 240.0, MouseButton::Left);
        s.type_text("a");
        let mut w = AwtEventWriter::new(Vec::new());
        assert_eq!(w.flush_session(&mut s).unwrap(), 2 * EVENT_RECORD_LEN);
        assert_eq!(s.pending_events(), 0);
        assert_eq!(w.records_written(), 2);
    }

    #[test]
    fn a_broken_pipe_surfaces_as_an_error_not_a_panic() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "gone"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut w = AwtEventWriter::new(Broken);
        let records = vec![AwtEventRecord::default()];
        assert!(w.write_records(&records).is_err());
    }

    // ---- End-to-end -------------------------------------------------------

    #[test]
    fn full_round_trip_jvm_frames_out_touches_in() {
        // 1) the "JVM" paints two frames onto the wire
        let mut wire = Vec::new();
        wire.extend_from_slice(&full_frame(1, 64, 48, 0xFF20_2020).encode());
        wire.extend_from_slice(
            &AwtFrame::partial(2, 64, 48, Rect::new(4, 4, 2, 2), vec![0xFFFF_0000; 4])
                .unwrap()
                .encode(),
        );

        // 2) the launcher pumps them into the session
        let mut s = AwtSession::open(AwtSessionConfig::new(size(64, 48), size(640, 480))).unwrap();
        let mut stream = AwtFrameStream::new(Cursor::new(wire));
        assert_eq!(stream.pump_into(&mut s).unwrap(), (2, 0));

        // 3) Compose uploads the damage
        let mut dst = vec![0u8; s.rgba_len()];
        let (rect, bytes) = s.copy_dirty_rgba_into(&mut dst).unwrap().unwrap();
        assert_eq!(rect, Rect::whole(64, 48), "initial + partial coalesced");
        assert_eq!(bytes, 64 * 48 * 4);
        let px = ((4 * 64) + 4) * 4;
        assert_eq!(&dst[px..px + 4], &[0xFF, 0x00, 0x00, 0xFF], "red patch");

        // 4) the user taps that red patch; the JVM gets the click at (4,4)
        let mut out = AwtEventWriter::new(Vec::new());
        s.pointer(PointerPhase::Down, 45.0, 45.0, MouseButton::Left);
        s.pointer(PointerPhase::Up, 45.0, 45.0, MouseButton::Left);
        out.flush_session(&mut s).unwrap();
        let records = AwtEventRecord::decode_batch(&out.into_inner()).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!((records[0].x, records[0].y), (4, 4));
        assert_eq!(records[2].id, event_id::MOUSE_CLICKED);
        assert_eq!(s.stats().frames_accepted, 2);
        assert_eq!(s.stats().frames_rejected, 0);
    }

    #[test]
    fn pooled_frame_recycles_buffer_across_frames() {
        let mut s = session();
        // Paint one frame so the desktop is dirty.
        let wire = full_frame(1, 64, 48, 0xFF20_2020).encode();
        let mut stream = AwtFrameStream::new(Cursor::new(wire));
        stream.pump_into(&mut s).unwrap();

        assert_eq!(s.frame_pool_idle(), 0);
        {
            let f = s.copy_dirty_rgba_into_pooled().unwrap().unwrap();
            assert_eq!(f.len(), 64 * 48 * 4);
            assert_eq!(&f.as_slice()[0..4], &[0x20, 0x20, 0x20, 0xFF]);
        }
        // After the pooled frame is dropped, its buffer is back in the pool.
        assert_eq!(s.frame_pool_idle(), 1);

        // Next frame reuses the pooled buffer (no fresh allocation).
        let wire2 = full_frame(2, 64, 48, 0xFF00_FF00).encode();
        let mut stream2 = AwtFrameStream::new(Cursor::new(wire2));
        stream2.pump_into(&mut s).unwrap();
        let f2 = s.copy_dirty_rgba_into_pooled().unwrap().unwrap();
        assert_eq!(f2.len(), 64 * 48 * 4);
        assert_eq!(&f2.as_slice()[0..4], &[0x00, 0xFF, 0x00, 0xFF]);
        assert_eq!(s.frame_pool_idle(), 0);
    }
}
