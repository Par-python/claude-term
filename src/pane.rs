use crate::event::AppEvent;
use anyhow::Result;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub type PaneId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notify {
    Output(PaneId),
    Exited(PaneId),
}

#[derive(Debug, Clone)]
pub struct PaneSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

fn nz(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n.max(1)).unwrap()
}

const SCROLLBACK: usize = 1000;

/// Answers the terminal queries programs send (cursor position, device attributes, colors),
/// since nothing else in the chain will. Replies pile up here during `process` and the
/// reader thread writes them out right after.
#[derive(Default)]
pub struct PaneCb {
    replies: Vec<u8>,
    /// when the program began a synchronized-output block (DEC mode 2026) that hasn't ended
    sync_since: Option<Instant>,
}

/// A synchronized block older than this is treated as lost, so drawing can't freeze.
pub const SYNC_CAP: Duration = Duration::from_millis(100);

const REPLY_DA1: &[u8] = b"\x1b[?62;22c";
const REPLY_FG: &[u8] = b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\";
const REPLY_BG: &[u8] = b"\x1b]11;rgb:0000/0000/0000\x1b\\";

impl vt100::Callbacks for PaneCb {
    fn unhandled_csi(&mut self, screen: &mut vt100::Screen, i1: Option<u8>, i2: Option<u8>, params: &[&[u16]], c: char) {
        if i1 == Some(b'?') && i2.is_none() && params.iter().any(|p| p.first() == Some(&2026)) {
            match c {
                'h' => self.sync_since = Some(Instant::now()),
                'l' => self.sync_since = None,
                _ => {}
            }
            return;
        }
        if i1.is_some() || i2.is_some() {
            return;
        }
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        match c {
            'n' if first == 6 => {
                let (row, col) = screen.cursor_position();
                self.replies.extend_from_slice(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
            }
            'n' if first == 5 => self.replies.extend_from_slice(b"\x1b[0n"),
            'c' if first == 0 => self.replies.extend_from_slice(REPLY_DA1),
            _ => {}
        }
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        match params {
            [b"10", b"?"] => self.replies.extend_from_slice(REPLY_FG),
            [b"11", b"?"] => self.replies.extend_from_slice(REPLY_BG),
            _ => {}
        }
    }
}

type Writer = Arc<Mutex<Box<dyn Write + Send>>>;

pub struct Pane {
    pub id: PaneId,
    pub spec: PaneSpec,
    pub parser: Arc<Mutex<vt100::Parser<PaneCb>>>,
    pub exited: Arc<AtomicBool>,
    rows: u16,
    cols: u16,
    // set when this run is superseded, so its reader stays quiet
    cancelled: Arc<AtomicBool>,
    master: Box<dyn MasterPty + Send>,
    writer: Writer,
    child: Option<Box<dyn Child + Send + Sync>>,
}

struct Run {
    parser: Arc<Mutex<vt100::Parser<PaneCb>>>,
    exited: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    master: Box<dyn MasterPty + Send>,
    writer: Writer,
    child: Box<dyn Child + Send + Sync>,
}

fn write_to(w: &Writer, bytes: &[u8]) {
    let mut w = w.lock().unwrap_or_else(|e| e.into_inner());
    let _ = w.write_all(bytes);
    let _ = w.flush();
}

/// Feeds everything `reader` yields to `f` until EOF, a real error, or `f` returns false.
/// An interrupted read (a signal landed) is retried, not treated as EOF.
fn read_chunks(reader: &mut impl Read, mut f: impl FnMut(&[u8]) -> bool) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if !f(&buf[..n]) {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

fn start(id: PaneId, spec: &PaneSpec, rows: u16, cols: u16, tx: Sender<AppEvent>) -> Result<Run> {
    let (rows, cols) = (rows.max(1), cols.max(1));
    let pair = native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
    let mut cmd = CommandBuilder::new(&spec.program);
    cmd.args(&spec.args);
    cmd.cwd(&spec.cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    // these describe the outer terminal, which isn't what the child talks to
    cmd.env_remove("TERM_PROGRAM");
    cmd.env_remove("TERM_PROGRAM_VERSION");
    let child = pair.slave.spawn_command(cmd)?;
    // drop our slave handle so EOF arrives when the child exits
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader()?;
    let writer: Writer = Arc::new(Mutex::new(pair.master.take_writer()?));
    let parser = Arc::new(Mutex::new(vt100::Parser::new_with_callbacks(nz(rows), nz(cols), SCROLLBACK, PaneCb::default())));
    let exited = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let (parser, exited, cancelled, writer) = (parser.clone(), exited.clone(), cancelled.clone(), writer.clone());
        std::thread::spawn(move || {
            read_chunks(&mut reader, |chunk| {
                let replies = {
                    let mut p = parser.lock().unwrap_or_else(|e| e.into_inner());
                    p.process(chunk);
                    std::mem::take(&mut p.callbacks_mut().replies)
                };
                if !replies.is_empty() && !cancelled.load(Ordering::SeqCst) {
                    write_to(&writer, &replies);
                }
                if cancelled.load(Ordering::SeqCst) {
                    return false;
                }
                let _ = tx.send(AppEvent::Pty(Notify::Output(id)));
                true
            });
            exited.store(true, Ordering::SeqCst);
            if !cancelled.load(Ordering::SeqCst) {
                let _ = tx.send(AppEvent::Pty(Notify::Exited(id)));
            }
        });
    }
    Ok(Run { parser, exited, cancelled, master: pair.master, writer, child })
}

impl Pane {
    pub fn spawn(id: PaneId, spec: PaneSpec, rows: u16, cols: u16, tx: Sender<AppEvent>) -> Result<Pane> {
        let r = start(id, &spec, rows, cols, tx)?;
        Ok(Pane {
            id,
            spec,
            parser: r.parser,
            exited: r.exited,
            rows: rows.max(1),
            cols: cols.max(1),
            cancelled: r.cancelled,
            master: r.master,
            writer: r.writer,
            child: Some(r.child),
        })
    }

    /// Child process id, if the child is still held.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(|c| c.process_id())
    }

    pub fn write(&mut self, bytes: &[u8]) {
        write_to(&self.writer, bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(1), cols.max(1));
        self.rows = rows;
        self.cols = cols;
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        self.screen_lock().screen_mut().set_size(nz(rows), nz(cols));
    }

    /// How many lines the view is scrolled up from the live screen.
    pub fn scroll_offset(&self) -> usize {
        self.screen_lock().screen().scrollback()
    }

    /// Lines of history available to scroll through.
    pub fn scroll_max(&self) -> usize {
        self.screen_lock().screen().scrollback_len()
    }

    /// Scrolls the view: positive goes back in history, negative toward the live screen. Clamped.
    pub fn scroll_by(&self, delta: i32) {
        let mut g = self.screen_lock();
        let cur = g.screen().scrollback() as i64;
        g.screen_mut().set_scrollback((cur + i64::from(delta)).max(0) as usize);
    }

    pub fn scroll_to(&self, rows: usize) {
        self.screen_lock().screen_mut().set_scrollback(rows);
    }

    /// True while the program is inside a synchronized-output block (it asked us not to
    /// draw half a frame). Blocks older than `SYNC_CAP` don't count.
    pub fn in_sync_block(&self) -> bool {
        self.in_sync_block_at(Instant::now())
    }

    pub fn in_sync_block_at(&self, now: Instant) -> bool {
        self.screen_lock().callbacks().sync_since.is_some_and(|t| now.saturating_duration_since(t) < SYNC_CAP)
    }

    /// True when the program inside asked for mouse events.
    pub fn mouse_mode_on(&self) -> bool {
        self.screen_lock().screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None
    }

    /// Locks the parser, recovering from a poisoned mutex.
    pub fn screen_lock(&self) -> MutexGuard<'_, vt100::Parser<PaneCb>> {
        self.parser.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Kills the old run first (so two children never share a session),
    /// then starts a new one. If starting fails, the pane stays exited.
    pub fn restart(&mut self, tx: Sender<AppEvent>) -> Result<()> {
        self.cancelled.store(true, Ordering::SeqCst);
        // an already-exited child needs no work; a live one must be
        // dead before the new run starts
        if let Some(g) = self.take_group() {
            terminate(vec![g], GRACE);
        }
        self.exited = Arc::new(AtomicBool::new(true));
        let r = start(self.id, &self.spec, self.rows, self.cols, tx)?;
        self.parser = r.parser;
        self.exited = r.exited;
        self.cancelled = r.cancelled;
        self.master = r.master;
        self.writer = r.writer;
        self.child = Some(r.child);
        Ok(())
    }

    /// Hands over the child and its process group, for `terminate`. The pane no longer owns a child after this.
    pub fn take_group(&mut self) -> Option<Group> {
        let child = self.child.take()?;
        let pgid = child.process_id()? as i32;
        Some(Group { pgid, child: Some(child) })
    }

    /// Returns immediately; the hangup, kill and reap of the whole process group happen on a short-lived thread.
    pub fn kill(&mut self) {
        if let Some(g) = self.take_group() {
            std::thread::spawn(move || terminate(vec![g], GRACE));
        }
    }
}

/// How long processes get to react to SIGHUP before SIGKILL.
pub const GRACE: Duration = Duration::from_millis(300);

/// A pty child and its process group. The child is a session leader, so its pid is the group id.
pub struct Group {
    pgid: i32,
    child: Option<Box<dyn Child + Send + Sync>>,
}

fn group_alive(pgid: i32) -> bool {
    // SAFETY: killpg with signal 0 only checks that the group exists.
    unsafe { libc::killpg(pgid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

/// Sends SIGHUP to every group, waits up to `grace` for all of them to go (leaders are
/// reaped along the way), then SIGKILLs whatever is left. Blocks, so callers that can't wait
/// run it on a thread.
pub fn terminate(mut groups: Vec<Group>, grace: Duration) {
    for g in &groups {
        // SAFETY: plain signal to a group we started; the unreaped leader keeps the id from being reused.
        unsafe {
            libc::killpg(g.pgid, libc::SIGHUP);
        }
    }
    let end = Instant::now() + grace;
    loop {
        groups.retain_mut(|g| {
            if let Some(c) = g.child.as_mut()
                && !matches!(c.try_wait(), Ok(None))
            {
                g.child = None;
            }
            g.child.is_some() || group_alive(g.pgid)
        });
        if groups.is_empty() || Instant::now() >= end {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    for g in &mut groups {
        // SAFETY: as above.
        unsafe {
            libc::killpg(g.pgid, libc::SIGKILL);
        }
        if let Some(c) = g.child.as_mut() {
            let _ = c.wait();
        }
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error, ErrorKind};

    struct Flaky(Vec<std::io::Result<Vec<u8>>>);

    impl Read for Flaky {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.0.is_empty() {
                return Ok(0);
            }
            match self.0.remove(0) {
                Ok(b) => {
                    buf[..b.len()].copy_from_slice(&b);
                    Ok(b.len())
                }
                Err(e) => Err(e),
            }
        }
    }

    #[test]
    fn interrupted_read_is_retried_not_eof() {
        let mut r = Flaky(vec![Err(Error::from(ErrorKind::Interrupted)), Ok(b"ab".to_vec()), Err(Error::from(ErrorKind::Interrupted)), Ok(b"cd".to_vec())]);
        let mut got = Vec::new();
        read_chunks(&mut r, |c| {
            got.extend_from_slice(c);
            true
        });
        assert_eq!(got, b"abcd");
    }

    #[test]
    fn real_error_ends_the_loop() {
        let mut r = Flaky(vec![Ok(b"a".to_vec()), Err(Error::from(ErrorKind::BrokenPipe)), Ok(b"b".to_vec())]);
        let mut got = Vec::new();
        read_chunks(&mut r, |c| {
            got.extend_from_slice(c);
            true
        });
        assert_eq!(got, b"a");
    }
}
