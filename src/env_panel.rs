//! State of the environment panel for the open chat.
use crate::env::{ChatTracker, Connectors};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How often the git branch is read again while the panel shows.
/// Output driven polls of the session file happen at most this often.
pub const OUTPUT_POLL_EVERY: Duration = Duration::from_millis(250);
pub const BRANCH_EVERY: Duration = Duration::from_secs(5);

/// A row the panel cursor can sit on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvRow {
    Remote,
    Connectors,
    Output(usize),
}

pub struct EnvPanel {
    pub tracker: Option<ChatTracker>,
    /// the chat the panel describes; results tagged with another id are dropped
    pub chat: Option<String>,
    pub branch: String,
    pub connectors: Connectors,
    pub expanded: bool,
    pub cursor: usize,
    /// first panel line shown; drawing keeps the cursor line in view and stores it back
    pub scroll: std::cell::Cell<usize>,
    /// the output the cursor is on, so new outputs don't move it onto another file
    pinned: Option<PathBuf>,
    /// short hostname, read once at startup
    pub host: String,
    /// session-file lookups made so far (test hook)
    pub lookups: usize,
    last_poll: Option<Instant>,
    asked: Option<Instant>,
    inflight: Arc<AtomicBool>,
}

impl EnvPanel {
    pub fn new() -> EnvPanel {
        EnvPanel {
            tracker: None,
            chat: None,
            branch: String::new(),
            connectors: Connectors::Checking,
            expanded: false,
            cursor: 0,
            scroll: std::cell::Cell::new(0),
            pinned: None,
            host: "localhost".into(),
            lookups: 0,
            last_poll: None,
            asked: None,
            inflight: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Points the panel at another chat: drops everything read for the old one.
    pub fn switch(&mut self, chat: Option<String>) {
        self.chat = chat;
        self.tracker = None;
        self.branch.clear();
        self.asked = None;
        self.cursor = 0;
        self.scroll.set(0);
        self.pinned = None;
        // a read still running for the old chat must not block this one
        self.inflight = Arc::new(AtomicBool::new(false));
    }

    /// How many rows j/k can land on: remote control, connectors, then each output.
    pub fn row_count(&self) -> usize {
        2 + self.outputs().len()
    }

    pub fn row_at(&self, i: usize) -> EnvRow {
        match i {
            0 => EnvRow::Remote,
            1 => EnvRow::Connectors,
            n => EnvRow::Output(n - 2),
        }
    }

    pub fn move_cursor(&mut self, delta: i32) {
        let last = self.row_count() - 1;
        self.cursor = (self.cursor.min(last) as i64 + i64::from(delta)).clamp(0, last as i64) as usize;
        self.pinned = self.cursor.checked_sub(2).and_then(|i| self.outputs().get(i).cloned());
    }

    /// Keeps the cursor on its pinned output after the list changed.
    fn follow_pin(&mut self) {
        let Some(p) = &self.pinned else { return };
        match self.outputs().iter().position(|o| o == p) {
            Some(i) if self.cursor >= 2 => self.cursor = i + 2,
            _ => self.pinned = None,
        }
    }

    pub fn context_pct(&self) -> Option<u8> {
        self.tracker.as_ref().and_then(ChatTracker::context_pct)
    }

    pub fn outputs(&self) -> &[PathBuf] {
        self.tracker.as_ref().map_or(&[], ChatTracker::outputs)
    }

    /// Reads what the session file gained since last time.
    pub fn poll(&mut self) {
        self.last_poll = Some(Instant::now());
        if let Some(t) = &mut self.tracker {
            t.poll();
        }
        self.follow_pin();
    }

    /// Like `poll`, but at most once per `OUTPUT_POLL_EVERY`; for pty output bursts.
    pub fn poll_throttled(&mut self) {
        if self.last_poll.is_none_or(|t| t.elapsed() >= OUTPUT_POLL_EVERY) {
            self.poll();
        }
    }

    /// True when a branch read should start now (none in flight, none asked within `BRANCH_EVERY`).
    pub fn branch_due(&self) -> bool {
        self.chat.is_some() && !self.inflight.load(Ordering::SeqCst) && self.asked.is_none_or(|t| t.elapsed() >= BRANCH_EVERY)
    }

    /// Marks a read as started and returns the flag the worker clears when done.
    pub fn branch_started(&mut self) -> Arc<AtomicBool> {
        self.asked = Some(Instant::now());
        self.inflight.store(true, Ordering::SeqCst);
        self.inflight.clone()
    }
}

impl Default for EnvPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// `<root>/<project dir>/<id>.jsonl`, if some project dir has it.
pub fn find_session_file(root: &Path, id: &str) -> Option<PathBuf> {
    let name = format!("{id}.jsonl");
    std::fs::read_dir(root).ok()?.flatten().map(|d| d.path().join(&name)).find(|p| p.is_file())
}

/// claude's project dir name for a folder: every character that isn't a letter or digit becomes `-`.
pub fn encode_project_dir(cwd: &Path) -> String {
    cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// The chat's session file: its folder's project dir first (one stat), and only when `scan` is set
/// every project dir.
pub fn locate_session(root: &Path, cwd: &Path, id: &str, scan: bool) -> Option<PathBuf> {
    let p = root.join(encode_project_dir(cwd)).join(format!("{id}.jsonl"));
    if p.is_file() {
        return Some(p);
    }
    if scan { find_session_file(root, id) } else { None }
}
