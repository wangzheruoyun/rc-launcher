//! AWT host (task 18) — the long-lived owner of a live [`AwtSession`].
//!
//! [`awt`](crate::launch::awt) defines the *pieces* of the AWT-on-Android
//! adaptation and [`fakefx`](crate::launch::fakefx) the *session* that ties them
//! together. Neither of them touches the operating system: a session is pure
//! state, driven by whoever owns it. This module is that owner — the piece that
//! actually connects a running game JVM to the Compose canvas:
//!
//! ```text
//!   game JVM (caciocavallo peers)                    launcher (this module)
//!   ───────────────────────────                      ──────────────────────
//!   awt bridge ──▶ frames channel  ──[RCAF]──▶  frame pump thread
//!                  (named pipe)                      │ AwtSession::submit_frame
//!                                                    ▼
//!                                            Arc<Mutex<AwtSession>>  ◀── FFI /
//!                                                    │                   Compose
//!                                                    │ drain_events      (poll
//!   awt event queue ◀── events channel ◀─────── event pump thread        + input)
//! ```
//!
//! ## Why threads, and why only two
//!
//! A named pipe read blocks; Compose must not. So the *frames* channel is
//! drained by one dedicated thread that pushes into the shared session, and the
//! *events* channel is fed by a second thread that pulls from it. The UI thread
//! then only ever does two cheap things per vsync: `poll_frame_into` (a
//! damage-limited memcpy into the `Bitmap`'s buffer) and a couple of input
//! calls. Everything else — blocking I/O, frame validation, load shedding —
//! happens off the UI thread.
//!
//! ## Robustness (task 19)
//!
//! * A frame that fails validation is **counted, not fatal**: cacio recovers on
//!   the next repaint ([`FrameRead::Rejected`]).
//! * A desynchronised or truncated stream ends the pump with a *reason string*
//!   the UI can show, instead of panicking or spinning.
//! * The pump threads check a stop flag between frames and wake up at most
//!   [`AwtHost::poll_interval`] later, so [`AwtHost::stop`] never hangs and a
//!   dropped host cannot leak a thread that keeps writing into a dead session.
//! * A poisoned session mutex is recovered from (`into_inner`) rather than
//!   propagated: the pixels may be stale, but the UI keeps running.
//! * Nothing here can block the caller: opening a FIFO (which waits for the peer)
//!   happens *inside* the pump thread.

use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::{RcError, RcResult};
use crate::launch::awt::{
    AwtControl, AwtTransport, CursorKind, MouseButton, PointerPhase, Rect, ScaleMode,
};
use crate::launch::fakefx::{
    AwtEventWriter, AwtFrameStream, AwtSession, AwtSessionConfig, FrameRead,
};

/// How long a pump thread may sleep before it re-checks the stop flag.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// How long the event pump sleeps when the outbox is empty (~1 frame at 240 Hz,
/// so a tap reaches the JVM within a frame without busy-spinning).
pub const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_millis(4);
/// Permissions of a freshly created channel: owner only (the JVM runs as us).
pub const CHANNEL_MODE: u32 = 0o600;

// ===========================================================================
// Link state & counters
// ===========================================================================

/// State of the link between the launcher and the game JVM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// No transport attached: the session exists, nothing feeds it.
    Detached,
    /// At least one pump thread is running.
    Attached,
    /// Every pump finished (the JVM exited, or we stopped it).
    Ended,
}

impl LinkState {
    /// Stable id for JSON / logs.
    pub fn id(self) -> &'static str {
        match self {
            LinkState::Detached => "detached",
            LinkState::Attached => "attached",
            LinkState::Ended => "ended",
        }
    }
}

/// Shared, lock-free counters of the transport (safe to read from the UI thread).
#[derive(Debug, Default)]
struct LinkCounters {
    frames_accepted: AtomicU64,
    frames_rejected: AtomicU64,
    controls_accepted: AtomicU64,
    controls_rejected: AtomicU64,
    events_written: AtomicU64,
    events_lost: AtomicU64,
    frame_pump_alive: AtomicBool,
    event_pump_alive: AtomicBool,
    ever_attached: AtomicBool,
    reason: Mutex<Option<String>>,
}

impl LinkCounters {
    fn set_reason(&self, reason: String) {
        let mut slot = self.reason.lock().unwrap_or_else(|e| e.into_inner());
        // Keep the *first* reason: it explains why the link died; later ones are
        // consequences (e.g. "broken pipe" after "the JVM exited").
        if slot.is_none() {
            *slot = Some(reason);
        }
    }

    fn reason(&self) -> Option<String> {
        self.reason
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn state(&self) -> LinkState {
        if self.frame_pump_alive.load(Ordering::Relaxed)
            || self.event_pump_alive.load(Ordering::Relaxed)
        {
            LinkState::Attached
        } else if self.ever_attached.load(Ordering::Relaxed) {
            LinkState::Ended
        } else {
            LinkState::Detached
        }
    }
}

/// Snapshot of the transport, for the diagnostics screen / FFI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkStats {
    /// Current link state.
    pub state: LinkState,
    /// Frames accepted from the JVM and published to the canvas.
    pub frames_accepted: u64,
    /// Frames the JVM sent that failed validation (kept the stream aligned).
    pub frames_rejected: u64,
    /// Control messages (cursor / title / clipboard / IME) accepted.
    pub controls_accepted: u64,
    /// Control messages the session refused (unknown kind, impossible screen).
    pub controls_rejected: u64,
    /// AWT records handed to the JVM.
    pub events_written: u64,
    /// AWT records drained but never written (the channel died mid-write).
    pub events_lost: u64,
    /// Why the link ended (`None` while it is healthy).
    pub reason: Option<String>,
}

impl LinkStats {
    /// JSON snapshot.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "state": self.state.id(),
            "frames_accepted": self.frames_accepted,
            "frames_rejected": self.frames_rejected,
            "controls_accepted": self.controls_accepted,
            "controls_rejected": self.controls_rejected,
            "events_written": self.events_written,
            "events_lost": self.events_lost,
            "reason": self.reason,
        })
    }
}

// ===========================================================================
// The host
// ===========================================================================

/// Owns one live AWT session and the threads that pump its transport.
///
/// Cloning is deliberately not implemented: the host *is* the lifetime of the
/// pump threads. Share the session itself with [`AwtHost::share`] when another
/// component needs access (the FFI keeps the host in a global `Mutex`).
#[derive(Debug)]
pub struct AwtHost {
    session: Arc<Mutex<AwtSession>>,
    counters: Arc<LinkCounters>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    transport: Option<AwtTransport>,
    /// Stop-flag polling interval of the pump threads.
    pub poll_interval: Duration,
    /// Idle sleep of the event pump.
    pub flush_interval: Duration,
}

impl AwtHost {
    /// Open a host around a fresh session (no transport attached yet).
    pub fn open(config: AwtSessionConfig) -> RcResult<Self> {
        Ok(Self {
            session: Arc::new(Mutex::new(AwtSession::open(config)?)),
            counters: Arc::new(LinkCounters::default()),
            stop: Arc::new(AtomicBool::new(false)),
            threads: Vec::new(),
            transport: None,
            poll_interval: DEFAULT_POLL_INTERVAL,
            flush_interval: DEFAULT_FLUSH_INTERVAL,
        })
    }

    /// Open a host around the default 1280×720 desktop.
    pub fn open_default() -> RcResult<Self> {
        Self::open(AwtSessionConfig::default())
    }

    /// Override the pump timings (tests use much shorter ones).
    pub fn with_intervals(mut self, poll: Duration, flush: Duration) -> Self {
        self.poll_interval = poll.max(Duration::from_millis(1));
        self.flush_interval = flush.max(Duration::from_millis(1));
        self
    }

    /// Lock the session (recovering from a poisoned mutex).
    pub fn session(&self) -> MutexGuard<'_, AwtSession> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A handle to the session, for another owner (FFI, tests).
    pub fn share(&self) -> Arc<Mutex<AwtSession>> {
        Arc::clone(&self.session)
    }

    /// The channels advertised to the JVM, when a transport is attached.
    pub fn transport(&self) -> Option<&AwtTransport> {
        self.transport.as_ref()
    }

    /// Current link state.
    pub fn link_state(&self) -> LinkState {
        self.counters.state()
    }

    /// `true` while at least one pump thread is running.
    pub fn is_attached(&self) -> bool {
        self.link_state() == LinkState::Attached
    }

    /// Transport counters.
    pub fn link_stats(&self) -> LinkStats {
        LinkStats {
            state: self.counters.state(),
            frames_accepted: self.counters.frames_accepted.load(Ordering::Relaxed),
            frames_rejected: self.counters.frames_rejected.load(Ordering::Relaxed),
            controls_accepted: self.counters.controls_accepted.load(Ordering::Relaxed),
            controls_rejected: self.counters.controls_rejected.load(Ordering::Relaxed),
            events_written: self.counters.events_written.load(Ordering::Relaxed),
            events_lost: self.counters.events_lost.load(Ordering::Relaxed),
            reason: self.counters.reason(),
        }
    }

    // ---- Transport --------------------------------------------------------

    /// Pump frames from `reader` (the JVM → launcher direction).
    ///
    /// `reader` must be pollable (a pipe, FIFO or socket) so the thread can
    /// notice [`AwtHost::stop`] while the JVM is idle.
    pub fn attach_frames<R>(&mut self, reader: R)
    where
        R: Read + PollFd + Send + 'static,
    {
        let session = Arc::clone(&self.session);
        let counters = Arc::clone(&self.counters);
        let stop = Arc::clone(&self.stop);
        let poll = self.poll_interval;
        counters.ever_attached.store(true, Ordering::Relaxed);
        counters.frame_pump_alive.store(true, Ordering::Relaxed);
        self.threads.push(std::thread::spawn(move || {
            pump_frames(reader, session, counters, stop, poll);
        }));
    }

    /// Pump queued AWT records into `writer` (the launcher → JVM direction).
    pub fn attach_events<W>(&mut self, writer: W)
    where
        W: Write + Send + 'static,
    {
        let session = Arc::clone(&self.session);
        let counters = Arc::clone(&self.counters);
        let stop = Arc::clone(&self.stop);
        let flush = self.flush_interval;
        counters.ever_attached.store(true, Ordering::Relaxed);
        counters.event_pump_alive.store(true, Ordering::Relaxed);
        self.threads.push(std::thread::spawn(move || {
            pump_events(writer, session, counters, stop, flush);
        }));
    }

    /// Create the named pipes of `transport` and pump both directions.
    ///
    /// Returns immediately: each channel is *opened inside its pump thread*,
    /// because opening a FIFO waits for the peer and the JVM has not even been
    /// spawned yet when the launcher advertises the paths
    /// ([`AwtTransport::jvm_args`]).
    #[cfg(unix)]
    pub fn attach_transport(&mut self, transport: AwtTransport) -> RcResult<()> {
        // Attaching twice would recreate the FIFOs under the feet of the running
        // pumps (which keep an fd to the deleted inode) and leak a thread pair
        // per call, so refuse instead: one game session, one transport.
        if self.transport.is_some() {
            return Err(RcError::Launch(format!(
                "an AWT transport is already attached ({}); close the session first",
                self.transport
                    .as_ref()
                    .map(|t| t.frames.to_string_lossy().to_string())
                    .unwrap_or_default()
            )));
        }
        create_channels(&transport)?;
        let frames = transport.frames.clone();
        let events = transport.events.clone();
        {
            let session = Arc::clone(&self.session);
            let counters = Arc::clone(&self.counters);
            let stop = Arc::clone(&self.stop);
            let poll = self.poll_interval;
            counters.ever_attached.store(true, Ordering::Relaxed);
            counters.frame_pump_alive.store(true, Ordering::Relaxed);
            self.threads.push(std::thread::spawn(move || {
                match open_fifo_read(&frames, &stop, poll) {
                    Ok(file) => pump_frames(file, session, counters, stop, poll),
                    Err(e) => {
                        counters.set_reason(format!("cannot open the AWT frame channel: {e}"));
                        counters.frame_pump_alive.store(false, Ordering::Relaxed);
                    }
                }
            }));
        }
        {
            let session = Arc::clone(&self.session);
            let counters = Arc::clone(&self.counters);
            let stop = Arc::clone(&self.stop);
            let poll = self.poll_interval;
            let flush = self.flush_interval;
            counters.event_pump_alive.store(true, Ordering::Relaxed);
            self.threads.push(std::thread::spawn(move || {
                match open_fifo_write(&events, &stop, poll) {
                    Ok(file) => pump_events(file, session, counters, stop, flush),
                    Err(e) => {
                        counters.set_reason(format!("cannot open the AWT event channel: {e}"));
                        counters.event_pump_alive.store(false, Ordering::Relaxed);
                    }
                }
            }));
        }
        self.transport = Some(transport);
        Ok(())
    }

    /// Named-pipe transport is a Unix facility.
    #[cfg(not(unix))]
    pub fn attach_transport(&mut self, _transport: AwtTransport) -> RcResult<()> {
        Err(RcError::UnsupportedPlatform(
            "the AWT named-pipe transport requires a Unix platform".to_string(),
        ))
    }

    /// Ask every pump thread to finish (they notice within [`AwtHost::poll_interval`]).
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.counters
            .set_reason("stopped by the launcher".to_string());
    }

    /// Stop and wait for the pump threads, so the caller knows the session is
    /// nobody else's any more (used by the FFI when closing a session and by
    /// tests).
    pub fn stop_and_join(&mut self) {
        self.stop();
        for handle in self.threads.drain(..) {
            let _ = handle.join();
        }
    }

    // ---- Reporting --------------------------------------------------------

    /// Full JSON snapshot: the session plus the transport.
    pub fn to_json(&self) -> serde_json::Value {
        let mut value = self.session().to_json();
        if let Some(obj) = value.as_object_mut() {
            obj.insert("link".to_string(), self.link_stats().to_json());
            obj.insert(
                "transport".to_string(),
                match &self.transport {
                    Some(t) => t.to_json(),
                    None => serde_json::Value::Null,
                },
            );
        }
        value
    }

    /// One-line human summary (log lines, bug reports).
    pub fn describe(&self) -> String {
        let link = self.link_stats();
        format!(
            "{} | link {} ({} frames, {} rejected, {} events{})",
            self.session().describe(),
            link.state.id(),
            link.frames_accepted,
            link.frames_rejected,
            link.events_written,
            link.reason
                .as_deref()
                .map(|r| format!(", {r}"))
                .unwrap_or_default(),
        )
    }

    // ---- Convenience proxies (so the FFI does not lock three times) --------

    /// Feed one encoded frame straight into the session (loopback / tests /
    /// a Kotlin-side transport).
    pub fn submit_frame_bytes(&self, bytes: &[u8]) -> RcResult<Option<Rect>> {
        self.session().submit_frame_bytes(bytes)
    }

    /// Refresh a persistent RGBA framebuffer with whatever changed.
    pub fn poll_frame_into(&self, dst: &mut [u8]) -> RcResult<Option<(Rect, usize)>> {
        self.session().poll_frame_into(dst)
    }

    /// The Compose surface changed size.
    /// Take the control messages the JVM sent (cursor / title / clipboard / IME).
    ///
    /// One call per UI frame; the side effects (push to the Android clipboard,
    /// buzz, pop the keyboard) belong to the caller.
    pub fn drain_control(&self) -> Vec<AwtControl> {
        self.session().drain_control()
    }

    /// Feed one encoded (`RCAC`) control message in — used by a Kotlin-owned
    /// transport and by the self-test path, exactly like
    /// [`AwtHost::submit_frame_bytes`].
    pub fn submit_control_bytes(&self, bytes: &[u8]) -> RcResult<()> {
        self.session().submit_control_bytes(bytes)
    }

    /// Answer every outstanding `Clipboard.getContents()` with `text` (`None` =
    /// the Android clipboard holds no text).
    pub fn answer_clipboard(&self, text: Option<&str>) -> usize {
        self.session().answer_clipboard(text)
    }

    /// Pointer shape the JVM last asked for.
    pub fn cursor(&self) -> CursorKind {
        self.session().cursor()
    }

    /// Title of the active AWT window, if the bridge reported one.
    pub fn window_title(&self) -> Option<String> {
        self.session().window_title().map(str::to_string)
    }

    /// Text the JVM copied, consumed so it is pushed to Android exactly once.
    pub fn take_clipboard_out(&self) -> Option<String> {
        self.session().take_clipboard_out()
    }

    pub fn set_surface_size(&self, width: u32, height: u32) -> RcResult<()> {
        self.session().set_surface_size(width, height)
    }

    /// Change the fitting policy.
    pub fn set_scale_mode(&self, mode: ScaleMode) {
        self.session().set_scale_mode(mode);
    }

    /// Resize the virtual desktop (tells the JVM to re-lay out).
    pub fn resize_screen(&self, width: u32, height: u32) -> RcResult<()> {
        self.session().resize_screen(width, height)
    }

    /// A touch / mouse event in surface coordinates.
    pub fn pointer(
        &self,
        phase: PointerPhase,
        surface_x: f32,
        surface_y: f32,
        button: MouseButton,
    ) -> usize {
        self.session().pointer(phase, surface_x, surface_y, button)
    }
}

impl Drop for AwtHost {
    fn drop(&mut self) {
        // Never block a `drop`: the threads exit on their own within one poll
        // interval, and they only touch `Arc`s they own.
        self.stop.store(true, Ordering::Relaxed);
    }
}

// ===========================================================================
// Pump loops
// ===========================================================================

fn lock_session(session: &Mutex<AwtSession>) -> MutexGuard<'_, AwtSession> {
    session.lock().unwrap_or_else(|e| e.into_inner())
}

fn pump_frames<R: Read + PollFd>(
    reader: R,
    session: Arc<Mutex<AwtSession>>,
    counters: Arc<LinkCounters>,
    stop: Arc<AtomicBool>,
    poll: Duration,
) {
    let mut stream = AwtFrameStream::new(StopReader::new(reader, Arc::clone(&stop), poll));
    let reason = loop {
        if stop.load(Ordering::Relaxed) {
            break "stopped by the launcher".to_string();
        }
        match stream.read_next() {
            Ok(FrameRead::Frame(frame)) => {
                let accepted = lock_session(&session).submit_frame(&frame).is_ok();
                let counter = if accepted {
                    &counters.frames_accepted
                } else {
                    &counters.frames_rejected
                };
                counter.fetch_add(1, Ordering::Relaxed);
            }
            // A control message rides the same channel as the pixels so the two
            // stay ordered; the session folds it into its projection.
            Ok(FrameRead::Control(control)) => {
                let accepted = lock_session(&session).submit_control(&control).is_ok();
                let counter = if accepted {
                    &counters.controls_accepted
                } else {
                    &counters.controls_rejected
                };
                counter.fetch_add(1, Ordering::Relaxed);
            }
            // Framed but invalid: the stream is still aligned, keep pumping.
            Ok(FrameRead::Rejected(_)) => {
                counters.frames_rejected.fetch_add(1, Ordering::Relaxed);
            }
            Ok(FrameRead::Eof) => break "the game JVM closed the AWT frame channel".to_string(),
            Err(e) => break format!("AWT frame channel error: {e}"),
        }
    };
    counters.set_reason(reason);
    counters.frame_pump_alive.store(false, Ordering::Relaxed);
}

fn pump_events<W: Write>(
    writer: W,
    session: Arc<Mutex<AwtSession>>,
    counters: Arc<LinkCounters>,
    stop: Arc<AtomicBool>,
    flush: Duration,
) {
    let mut sink = AwtEventWriter::new(writer);
    let reason = loop {
        if stop.load(Ordering::Relaxed) {
            break "stopped by the launcher".to_string();
        }
        let records = lock_session(&session).drain_events();
        if records.is_empty() {
            std::thread::sleep(flush);
            continue;
        }
        match sink.write_records(&records) {
            Ok(_) => {
                counters
                    .events_written
                    .fetch_add(records.len() as u64, Ordering::Relaxed);
            }
            Err(e) => {
                // The records were already drained from the session: account for
                // them instead of pretending the JVM saw them.
                counters
                    .events_lost
                    .fetch_add(records.len() as u64, Ordering::Relaxed);
                break format!("AWT event channel error: {e}");
            }
        }
    };
    counters.set_reason(reason);
    counters.event_pump_alive.store(false, Ordering::Relaxed);
}

// ===========================================================================
// Stop-aware reader
// ===========================================================================

/// A borrowed file descriptor, so the pump can wait on the channel *and* the
/// stop flag. Implemented for `File`, `UnixStream`, `PipeReader`, … and,
/// crucially, kept trivially implementable in tests.
pub trait PollFd {
    /// The raw descriptor. `-1` means "not pollable": the pump then reads
    /// straight through (used by in-memory test readers).
    fn poll_fd(&self) -> i32;
}

#[cfg(unix)]
impl<T: std::os::unix::io::AsRawFd> PollFd for T {
    fn poll_fd(&self) -> i32 {
        self.as_raw_fd()
    }
}

/// Wraps a blocking reader so a read only starts once the descriptor is
/// readable, letting the pump observe the stop flag while the JVM is idle.
///
/// Reads themselves stay **blocking**, which is what keeps a frame that arrives
/// in several TCP/pipe chunks aligned on the wire.
struct StopReader<R> {
    inner: R,
    stop: Arc<AtomicBool>,
    poll: Duration,
}

impl<R> StopReader<R> {
    fn new(inner: R, stop: Arc<AtomicBool>, poll: Duration) -> Self {
        Self { inner, stop, poll }
    }
}

impl<R: Read + PollFd> Read for StopReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let fd = self.inner.poll_fd();
        loop {
            if self.stop.load(Ordering::Relaxed) {
                // Not `Interrupted`: that is retried by the frame reader.
                return Err(io::Error::other("AWT frame pump stopped"));
            }
            if fd < 0 || wait_readable(fd, self.poll)? {
                return self.inner.read(buf);
            }
        }
    }
}

/// Wait until `fd` is readable (or hung up). `Ok(false)` on timeout.
#[cfg(unix)]
fn wait_readable(fd: i32, timeout: Duration) -> io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
    loop {
        let rc = unsafe { libc::poll(&mut pfd, 1, millis) };
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        // POLLHUP / POLLERR must also be reported as "go read": the read then
        // returns 0 / the real error, which the pump turns into a clean EOF.
        return Ok(rc > 0);
    }
}

#[cfg(not(unix))]
fn wait_readable(_fd: i32, _timeout: Duration) -> io::Result<bool> {
    Ok(true)
}

// ===========================================================================
// Named-pipe channels (Unix)
// ===========================================================================

/// Create both channels of `transport` as fresh FIFOs (`mkfifo`, mode `0600`).
///
/// A leftover file from a previous session is removed first: a *regular* file at
/// that path would silently turn the live link into a growing log file.
#[cfg(unix)]
pub fn create_channels(transport: &AwtTransport) -> RcResult<()> {
    for path in [&transport.frames, &transport.events] {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        if path.symlink_metadata().is_ok() {
            std::fs::remove_file(path)?;
        }
        mkfifo(path, CHANNEL_MODE)?;
    }
    Ok(())
}

#[cfg(unix)]
fn cstring(path: &Path) -> RcResult<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        RcError::Launch(format!(
            "channel path {} contains a NUL byte",
            path.display()
        ))
    })
}

#[cfg(unix)]
fn mkfifo(path: &Path, mode: u32) -> RcResult<()> {
    let c_path = cstring(path)?;
    let rc = unsafe { libc::mkfifo(c_path.as_ptr(), mode as libc::mode_t) };
    if rc != 0 {
        return Err(RcError::Io(io::Error::last_os_error()));
    }
    Ok(())
}

/// Open the read end of a FIFO without waiting for a writer.
///
/// `O_NONBLOCK` makes the *open* return immediately (a blocking open would wait
/// for the JVM, which does not exist yet); the flag is then cleared so the
/// subsequent reads block and stay frame-aligned.
#[cfg(unix)]
fn open_fifo_read(path: &Path, stop: &AtomicBool, poll: Duration) -> RcResult<std::fs::File> {
    use std::os::unix::io::FromRawFd;
    let c_path = cstring(path)?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
        if fd >= 0 {
            clear_nonblocking(fd)?;
            // SAFETY: `fd` was just returned by a successful `open` and is not
            // owned by anything else.
            return Ok(unsafe { std::fs::File::from_raw_fd(fd) });
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::NotFound && err.kind() != io::ErrorKind::Interrupted {
            return Err(RcError::Io(err));
        }
        if stop.load(Ordering::Relaxed) || Instant::now() > deadline {
            return Err(RcError::Io(err));
        }
        std::thread::sleep(poll);
    }
}

/// Open the write end of a FIFO, waiting for the JVM to open the read end.
///
/// A FIFO opened `O_WRONLY | O_NONBLOCK` fails with `ENXIO` while it has no
/// reader, so we retry until the JVM-side bridge shows up (or we are stopped).
#[cfg(unix)]
fn open_fifo_write(path: &Path, stop: &AtomicBool, poll: Duration) -> RcResult<std::fs::File> {
    use std::os::unix::io::FromRawFd;
    let c_path = cstring(path)?;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(RcError::Launch(
                "stopped while waiting for the JVM to open the AWT event channel".to_string(),
            ));
        }
        let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK) };
        if fd >= 0 {
            clear_nonblocking(fd)?;
            // SAFETY: see `open_fifo_read`.
            return Ok(unsafe { std::fs::File::from_raw_fd(fd) });
        }
        let err = io::Error::last_os_error();
        let raw = err.raw_os_error().unwrap_or(0);
        if raw != libc::ENXIO && raw != libc::EINTR && err.kind() != io::ErrorKind::NotFound {
            return Err(RcError::Io(err));
        }
        std::thread::sleep(poll);
    }
}

#[cfg(unix)]
fn clear_nonblocking(fd: i32) -> RcResult<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(RcError::Io(io::Error::last_os_error()));
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) } < 0 {
        return Err(RcError::Io(io::Error::last_os_error()));
    }
    Ok(())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::awt::{
        decode_control_reply, AwtEventRecord, AwtFrame, AwtReplyKind, EVENT_RECORD_LEN,
        FRAME_HEADER_LEN,
    };
    use crate::launch::options::WindowSize;
    use std::io::Cursor;
    use std::os::unix::net::UnixStream;

    fn size(width: u32, height: u32) -> WindowSize {
        WindowSize { width, height }
    }

    fn host() -> AwtHost {
        AwtHost::open(AwtSessionConfig::new(size(64, 48), size(128, 96)))
            .unwrap()
            .with_intervals(Duration::from_millis(2), Duration::from_millis(1))
    }

    fn frame(seq: u32, argb: u32) -> Vec<u8> {
        AwtFrame::full(seq, 64, 48, vec![argb; 64 * 48])
            .unwrap()
            .encode()
    }

    /// Spin until `check` holds (or ~5 s pass): the pumps are real threads, so
    /// the assertions have to be time-based — but never *sleep-based*, so the
    /// suite stays fast.
    fn wait_until(mut check: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if check() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        check()
    }

    fn read_exact_within(stream: &mut UnixStream, buf: &mut [u8]) -> bool {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.read_exact(buf).is_ok()
    }

    // ---- Plumbing ---------------------------------------------------------

    #[test]
    fn a_fresh_host_is_detached_and_idle() {
        let h = host();
        assert_eq!(h.link_state(), LinkState::Detached);
        assert!(!h.is_attached());
        let stats = h.link_stats();
        assert_eq!(stats.frames_accepted, 0);
        assert_eq!(stats.frames_rejected, 0);
        assert_eq!(stats.events_written, 0);
        assert!(stats.reason.is_none());
        assert!(h.transport().is_none());
        // The session is alive and starts black + fully damaged.
        assert_eq!(h.session().screen_size(), (64, 48));
    }

    #[test]
    fn intervals_have_a_floor_so_a_pump_cannot_busy_spin() {
        let h = AwtHost::open_default()
            .unwrap()
            .with_intervals(Duration::from_nanos(0), Duration::from_nanos(0));
        assert_eq!(h.poll_interval, Duration::from_millis(1));
        assert_eq!(h.flush_interval, Duration::from_millis(1));
        assert_eq!(h.session().screen_size(), (1280, 720));
    }

    #[test]
    fn json_and_describe_report_session_and_link() {
        let h = host();
        let json = h.to_json();
        assert_eq!(json["screen"]["width"], 64);
        assert_eq!(json["link"]["state"], "detached");
        assert!(json["transport"].is_null());
        assert!(h.describe().contains("link detached"), "{}", h.describe());
    }

    // ---- Control plane ----------------------------------------------------

    #[test]
    fn control_messages_written_by_the_jvm_reach_the_session() {
        let (mut jvm, launcher) = UnixStream::pair().unwrap();
        let mut h = host();
        h.attach_frames(launcher);
        assert!(wait_until(|| h.is_attached()));

        jvm.write_all(&AwtControl::cursor(CursorKind::Hand).encode())
            .unwrap();
        jvm.write_all(&AwtControl::title("Forge 安装程序").encode())
            .unwrap();
        jvm.write_all(&AwtControl::ime_show(8, 9, 12).encode())
            .unwrap();
        jvm.flush().unwrap();
        assert!(wait_until(|| h.link_stats().controls_accepted == 3));

        assert_eq!(h.cursor(), CursorKind::Hand);
        assert_eq!(h.window_title().as_deref(), Some("Forge 安装程序"));
        assert!(h.session().control().wants_keyboard());
        // The messages are also queued for the UI (one drain per frame).
        let drained = h.drain_control();
        assert_eq!(drained.len(), 3);
        assert!(h.drain_control().is_empty());
        h.stop_and_join();
    }

    #[test]
    fn pixels_and_control_messages_share_one_channel_without_desyncing() {
        let (mut jvm, launcher) = UnixStream::pair().unwrap();
        let mut h = host();
        h.attach_frames(launcher);
        assert!(wait_until(|| h.is_attached()));

        jvm.write_all(&frame(1, 0xFF00_00FF)).unwrap();
        jvm.write_all(&AwtControl::cursor(CursorKind::Text).encode())
            .unwrap();
        jvm.write_all(&frame(2, 0xFF00_FF00)).unwrap();
        jvm.flush().unwrap();
        assert!(wait_until(|| {
            let s = h.link_stats();
            s.frames_accepted == 2 && s.controls_accepted == 1
        }));
        assert_eq!(h.link_stats().frames_rejected, 0);
        assert_eq!(h.cursor(), CursorKind::Text);

        let mut rgba = vec![0u8; h.session().rgba_len()];
        h.poll_frame_into(&mut rgba).unwrap().unwrap();
        assert_eq!(&rgba[0..4], &[0x00, 0xFF, 0x00, 0xFF]);
        h.stop_and_join();
    }

    #[test]
    fn a_clipboard_request_is_answered_back_through_the_event_pump() {
        let (mut jvm_frames, launcher_frames) = UnixStream::pair().unwrap();
        let (mut jvm_events, launcher_events) = UnixStream::pair().unwrap();
        let mut h = host();
        h.attach_frames(launcher_frames);
        h.attach_events(launcher_events);
        assert!(wait_until(|| h.is_attached()));

        jvm_frames
            .write_all(&AwtControl::clipboard_request(77).encode())
            .unwrap();
        jvm_frames.flush().unwrap();
        assert!(wait_until(|| h.session().pending_clipboard_requests() == 1));

        // The UI read the Android clipboard and hands it back.
        let text = "copied on Android 中文";
        let queued = h.answer_clipboard(Some(text));
        assert!(queued >= 2, "a multi-chunk reply");

        let mut buf = vec![0u8; queued * EVENT_RECORD_LEN];
        assert!(read_exact_within(&mut jvm_events, &mut buf));
        let records = AwtEventRecord::decode_batch(&buf).unwrap();
        let (kind, seq, back) = decode_control_reply(&records).unwrap();
        assert_eq!(kind, AwtReplyKind::Clipboard);
        assert_eq!(seq, 77);
        assert_eq!(back, text);
        assert_eq!(h.session().pending_clipboard_requests(), 0);
        h.stop_and_join();
    }

    #[test]
    fn a_corrupt_control_message_is_counted_and_the_pump_keeps_going() {
        let (mut jvm, launcher) = UnixStream::pair().unwrap();
        let mut h = host();
        h.attach_frames(launcher);
        assert!(wait_until(|| h.is_attached()));

        // Unknown kind: framed correctly, so the channel stays aligned.
        let mut broken = AwtControl::beep().encode();
        broken[6] = 250;
        jvm.write_all(&broken).unwrap();
        // An impossible managed screen: decodes, but the session refuses it.
        jvm.write_all(&AwtControl::screen_size(0, 0).encode())
            .unwrap();
        jvm.write_all(&frame(5, 0xFFAB_CDEF)).unwrap();
        jvm.flush().unwrap();

        assert!(wait_until(|| h.link_stats().frames_accepted == 1));
        let stats = h.link_stats();
        assert_eq!(stats.controls_rejected, 1, "the session refused one");
        assert_eq!(stats.frames_rejected, 1, "the undecodable one");
        assert_eq!(h.session().screen_size(), (64, 48), "geometry untouched");
        assert_eq!(h.link_state(), LinkState::Attached, "the link survived");
        h.stop_and_join();
    }

    #[test]
    fn the_link_snapshot_reports_control_counters() {
        let h = host();
        let json = h.to_json();
        assert_eq!(json["link"]["controls_accepted"], 0);
        assert_eq!(json["link"]["controls_rejected"], 0);
        assert_eq!(json["control"]["cursor"], "default");
        assert_eq!(json["control"]["wants_keyboard"], false);
    }

    #[test]
    fn a_kotlin_owned_transport_can_inject_control_messages() {
        // No pipes at all: the Kotlin side owns the channels and forwards bytes,
        // exactly as it may already do for frames.
        let h = host();
        h.submit_control_bytes(&AwtControl::clipboard_set("copied").encode())
            .unwrap();
        assert_eq!(h.take_clipboard_out().as_deref(), Some("copied"));
        assert_eq!(h.take_clipboard_out(), None);
        assert!(h.submit_control_bytes(&[0u8; 8]).is_err());
        assert_eq!(h.link_state(), LinkState::Detached);
    }

    // ---- Frame pump -------------------------------------------------------

    #[test]
    fn frames_written_by_the_jvm_reach_the_canvas() {
        let (mut jvm, launcher) = UnixStream::pair().unwrap();
        let mut h = host();
        h.attach_frames(launcher);
        assert!(wait_until(|| h.is_attached()));

        jvm.write_all(&frame(1, 0xFF00_FF00)).unwrap();
        jvm.flush().unwrap();
        assert!(wait_until(|| h.link_stats().frames_accepted == 1));

        // The pixels really landed in the front buffer.
        let mut rgba = vec![0u8; h.session().rgba_len()];
        let (rect, _) = h.poll_frame_into(&mut rgba).unwrap().unwrap();
        assert_eq!(rect, Rect::whole(64, 48));
        assert_eq!(&rgba[0..4], &[0x00, 0xFF, 0x00, 0xFF]); // RGBA8888
        h.stop_and_join();
    }

    #[test]
    fn a_rejected_frame_is_counted_and_the_pump_keeps_going() {
        let (mut jvm, launcher) = UnixStream::pair().unwrap();
        let mut h = host();
        h.attach_frames(launcher);

        // Well-framed (32-byte header, no payload) but invalid: bad magic.
        let mut bogus = frame(1, 0xFF00_0000)[..FRAME_HEADER_LEN].to_vec();
        bogus[0] ^= 0xFF;
        bogus[24..28].copy_from_slice(&0u32.to_le_bytes()); // payload_len = 0
        jvm.write_all(&bogus).unwrap();
        // ... and a perfectly good frame right after it.
        jvm.write_all(&frame(2, 0xFF00_00FF)).unwrap();
        jvm.flush().unwrap();

        assert!(wait_until(|| {
            let s = h.link_stats();
            s.frames_rejected == 1 && s.frames_accepted == 1
        }));
        assert_eq!(
            h.link_state(),
            LinkState::Attached,
            "a bad frame is not fatal"
        );
        assert_eq!(h.session().canvas().pixel(0, 0), Some(0xFF00_00FF));
        h.stop_and_join();
    }

    #[test]
    fn the_jvm_exiting_ends_the_link_with_a_reason() {
        let (jvm, launcher) = UnixStream::pair().unwrap();
        let mut h = host();
        h.attach_frames(launcher);
        drop(jvm); // the game process exited

        assert!(wait_until(|| h.link_state() == LinkState::Ended));
        let reason = h.link_stats().reason.unwrap_or_default();
        assert!(reason.contains("closed the AWT frame channel"), "{reason}");
        h.stop_and_join();
    }

    #[test]
    fn stop_and_join_terminates_an_idle_pump() {
        let (_jvm, launcher) = UnixStream::pair().unwrap(); // stays open: fully idle
        let mut h = host();
        h.attach_frames(launcher);
        assert!(wait_until(|| h.is_attached()));

        let start = Instant::now();
        h.stop_and_join();
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "stop must not wait for the JVM"
        );
        assert_eq!(h.link_state(), LinkState::Ended);
        assert_eq!(
            h.link_stats().reason.as_deref(),
            Some("stopped by the launcher")
        );
    }

    #[test]
    fn a_desynchronised_stream_ends_the_link_instead_of_spinning() {
        let (mut jvm, launcher) = UnixStream::pair().unwrap();
        let mut h = host();
        h.attach_frames(launcher);
        jvm.write_all(b"half a header").unwrap();
        drop(jvm);
        assert!(wait_until(|| h.link_state() == LinkState::Ended));
        let reason = h.link_stats().reason.unwrap_or_default();
        assert!(reason.contains("frame channel"), "{reason}");
    }

    // ---- Event pump -------------------------------------------------------

    #[test]
    fn queued_input_is_written_to_the_jvm_channel() {
        let (mut jvm, launcher) = UnixStream::pair().unwrap();
        let mut h = host();
        h.attach_events(launcher);

        // A tap in the middle of the surface: press + release + synthetic click.
        let queued = h.pointer(PointerPhase::Down, 64.0, 48.0, MouseButton::Left);
        assert!(queued >= 1);
        h.pointer(PointerPhase::Up, 64.0, 48.0, MouseButton::Left);

        let mut buf = [0u8; EVENT_RECORD_LEN];
        assert!(read_exact_within(&mut jvm, &mut buf));
        let record = AwtEventRecord::decode(&buf).unwrap();
        assert_eq!(record.id, crate::launch::awt::event_id::MOUSE_PRESSED);
        assert_eq!((record.x, record.y), (32, 24)); // 128x96 surface -> 64x48 desktop
        assert!(wait_until(|| h.link_stats().events_written >= 1));
        h.stop_and_join();
    }

    #[test]
    fn a_dead_event_channel_is_accounted_not_fatal() {
        let (jvm, launcher) = UnixStream::pair().unwrap();
        let mut h = host();
        h.attach_events(launcher);
        drop(jvm); // the JVM stopped reading

        h.session().key_down_named("escape");
        assert!(wait_until(|| h.link_state() == LinkState::Ended));
        let stats = h.link_stats();
        assert!(stats.events_lost >= 1, "{stats:?}");
        let reason = stats.reason.unwrap_or_default();
        assert!(reason.contains("event channel"), "{reason}");
        // The session itself is untouched and still usable.
        assert_eq!(h.session().screen_size(), (64, 48));
    }

    #[test]
    fn a_non_pollable_reader_still_pumps() {
        // In-memory reader: `poll_fd() == -1` means "read straight through".
        struct MemReader(Cursor<Vec<u8>>);
        impl Read for MemReader {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.0.read(buf)
            }
        }
        impl PollFd for MemReader {
            fn poll_fd(&self) -> i32 {
                -1
            }
        }

        let mut h = host();
        h.attach_frames(MemReader(Cursor::new(frame(7, 0xFFFF_0000))));
        assert!(wait_until(|| h.link_stats().frames_accepted == 1));
        assert_eq!(h.session().canvas().pixel(1, 1), Some(0xFFFF_0000));
        assert!(wait_until(|| h.link_state() == LinkState::Ended)); // EOF
    }

    // ---- Named-pipe transport --------------------------------------------

    #[test]
    fn create_channels_makes_two_fifos_and_replaces_stale_files() {
        use std::os::unix::fs::FileTypeExt;
        let dir = tempfile::tempdir().unwrap();
        let transport = AwtTransport::in_dir(dir.path());
        // A leftover *regular* file from a crashed session must not survive.
        std::fs::write(&transport.frames, b"stale").unwrap();

        create_channels(&transport).unwrap();
        for path in [&transport.frames, &transport.events] {
            let meta = std::fs::metadata(path).unwrap();
            assert!(
                meta.file_type().is_fifo(),
                "{} is not a FIFO",
                path.display()
            );
        }
        // Idempotent: a second call recreates them cleanly.
        create_channels(&transport).unwrap();
        assert!(std::fs::metadata(&transport.frames)
            .unwrap()
            .file_type()
            .is_fifo());
    }

    #[test]
    fn attach_transport_round_trips_frames_and_events_over_named_pipes() {
        let dir = tempfile::tempdir().unwrap();
        let transport = AwtTransport::in_dir(dir.path());
        let mut h = host();
        h.attach_transport(transport.clone()).unwrap();
        // The properties handed to the JVM point at the channels we created.
        assert!(h
            .transport()
            .unwrap()
            .jvm_args()
            .iter()
            .any(|a| a.contains("rc.awt.bridge.frames")));

        // The "JVM" side: write a frame, read the events.
        let frames_path = transport.frames.clone();
        let producer = std::thread::spawn(move || {
            let mut w = std::fs::OpenOptions::new()
                .write(true)
                .open(&frames_path)
                .unwrap();
            w.write_all(&frame(1, 0xFF12_3456)).unwrap();
            w.flush().unwrap();
            // Keep the pipe open a moment so the pump does not see EOF at once.
            std::thread::sleep(Duration::from_millis(200));
        });

        assert!(wait_until(|| h.link_stats().frames_accepted == 1));
        assert_eq!(h.session().canvas().pixel(0, 0), Some(0xFF12_3456));

        h.session().key_down_named("escape");
        let mut reader = std::fs::File::open(&transport.events).unwrap();
        let mut buf = [0u8; EVENT_RECORD_LEN];
        reader.read_exact(&mut buf).unwrap();
        let record = AwtEventRecord::decode(&buf).unwrap();
        assert_eq!(record.id, crate::launch::awt::event_id::KEY_PRESSED);
        assert_eq!(record.key_code, crate::launch::awt::vk::ESCAPE);

        producer.join().unwrap();
        h.stop_and_join();
        assert_eq!(h.link_state(), LinkState::Ended);
    }

    #[test]
    fn attaching_a_second_transport_is_refused_not_leaked() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = host();
        h.attach_transport(AwtTransport::in_dir(dir.path()))
            .unwrap();
        let err = h
            .attach_transport(AwtTransport::in_dir(dir.path()))
            .unwrap_err();
        assert!(err.to_string().contains("already attached"), "{err}");
        // The first transport is untouched and still advertised to the JVM.
        assert_eq!(
            h.transport().unwrap().frames,
            dir.path().join("awt-frames.rcaf")
        );
        h.stop_and_join();
    }

    #[test]
    fn a_channel_path_that_cannot_be_created_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        // A *directory* where a channel belongs: `remove_file` cannot clear it.
        std::fs::create_dir(dir.path().join("awt-frames.rcaf")).unwrap();
        let mut h = host();
        assert!(h
            .attach_transport(AwtTransport::in_dir(dir.path()))
            .is_err());
        assert_eq!(h.link_state(), LinkState::Detached, "no pump was started");
        assert!(h.transport().is_none());
        // A NUL byte in the path is rejected before it reaches libc.
        let err = create_channels(&AwtTransport::new("/tmp/a\0b", "/tmp/c")).unwrap_err();
        assert!(err.to_string().contains("NUL"), "{err}");
    }

    #[test]
    fn dropping_a_host_stops_its_pumps() {
        let (_jvm, launcher) = UnixStream::pair().unwrap();
        let counters;
        {
            let mut h = host();
            counters = Arc::clone(&h.counters);
            h.attach_frames(launcher);
            assert!(wait_until(|| counters.state() == LinkState::Attached));
        } // dropped without an explicit stop
        assert!(wait_until(|| counters.state() == LinkState::Ended));
    }

    // ---- Proxies ----------------------------------------------------------

    #[test]
    fn proxies_forward_to_the_session() {
        let h = host();
        h.set_surface_size(256, 192).unwrap();
        assert_eq!(h.session().surface_size(), (256, 192));
        h.set_scale_mode(ScaleMode::Stretch);
        assert_eq!(h.session().config().scale_mode, ScaleMode::Stretch);
        h.resize_screen(32, 24).unwrap();
        assert_eq!(h.session().screen_size(), (32, 24));

        let bytes = AwtFrame::full(1, 32, 24, vec![0xFF00_00FF; 32 * 24])
            .unwrap()
            .encode();
        assert_eq!(
            h.submit_frame_bytes(&bytes).unwrap(),
            Some(Rect::whole(32, 24))
        );
        let mut rgba = vec![0u8; h.session().rgba_len()];
        assert!(h.poll_frame_into(&mut rgba).unwrap().is_some());
        assert!(
            h.poll_frame_into(&mut rgba).unwrap().is_none(),
            "nothing changed"
        );
        // A corrupt frame is an error, never a panic.
        assert!(h.submit_frame_bytes(b"nonsense").is_err());
    }
}
