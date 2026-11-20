use crate::event::AppEvent;
use crate::launch::shell_spec;
use crate::pane::{Pane, PaneId};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// A chat that wrote output this recently counts as working.
pub const WORKING: Duration = Duration::from_secs(2);

/// The sidebar mark for a chat ct started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    None,
    /// output in the last 2s
    Working,
    /// wrote while in the background, quiet now, not looked at yet
    Unread,
}

pub struct Procs {
    chats: Vec<(String, Pane, Instant)>,
    shells: HashMap<PathBuf, Pane>,
    /// chats that wrote output while in the background and haven't been focused since
    pub unread: HashSet<String>,
    last_output: HashMap<String, Instant>,
    max: usize,
    next: PaneId,
    /// when each chat was last dropped to make room
    evicted: HashMap<String, Instant>,
}

impl Procs {
    pub fn new(max: usize) -> Self {
        Procs { chats: vec![], shells: HashMap::new(), unread: HashSet::new(), last_output: HashMap::new(), max, next: 1, evicted: HashMap::new() }
    }

    pub fn next_id(&mut self) -> PaneId {
        let id = self.next;
        self.next += 1;
        id
    }

    /// Marks the chat as used and clears its unread dot.
    pub fn chat(&mut self, id: &str) -> Option<&mut Pane> {
        self.unread.remove(id);
        let e = self.chats.iter_mut().find(|(s, _, _)| s == id)?;
        e.2 = Instant::now();
        Some(&mut e.1)
    }

    /// Keeps `max` background chats plus the focused one (max + 1 total); evicts least recently used, never the new one.
    /// Evicted panes are dropped, which kills their children.
    pub fn insert_chat(&mut self, id: String, pane: Pane) {
        self.chats.retain(|(s, _, _)| *s != id);
        self.unread.remove(&id);
        self.chats.push((id, pane, Instant::now()));
        // the newest entry is always last, so it is never picked
        while self.chats.len() > self.max + 1 {
            let last = self.chats.len() - 1;
            let oldest = self.chats[..last].iter().enumerate().min_by_key(|(_, e)| e.2).map(|(i, _)| i);
            let Some(i) = oldest else { break };
            let (sid, _pane, _) = self.chats.remove(i);
            self.unread.remove(&sid);
            self.last_output.remove(&sid);
            self.evicted.insert(sid, Instant::now());
        }
    }

    /// Takes the chat's pane out. It counts as closed by ct.
    pub fn take_chat(&mut self, id: &str) -> Option<Pane> {
        let i = self.chats.iter().position(|(s, _, _)| s == id)?;
        let (_, pane, _) = self.chats.remove(i);
        self.unread.remove(id);
        self.last_output.remove(id);
        self.evicted.insert(id.to_string(), Instant::now());
        Some(pane)
    }

    /// True if ct dropped this chat's process less than `within` ago.
    pub fn closed_recently(&self, id: &str, within: std::time::Duration) -> bool {
        self.evicted.get(id).is_some_and(|t| t.elapsed() < within)
    }

    pub fn shell_for(&mut self, cwd: &Path, rows: u16, cols: u16, tx: Sender<AppEvent>) -> anyhow::Result<&mut Pane> {
        if !self.shells.contains_key(cwd) {
            let id = self.next_id();
            let pane = Pane::spawn(id, shell_spec(cwd), rows, cols, tx)?;
            self.shells.insert(cwd.to_path_buf(), pane);
        }
        Ok(self.shells.get_mut(cwd).expect("just inserted"))
    }

    /// Read-only lookup; does not touch recency or the unread dot.
    pub fn chat_pane(&self, id: &str) -> Option<&Pane> {
        self.chats.iter().find(|(s, _, _)| s == id).map(|(_, p, _)| p)
    }

    pub fn chat_pane_mut(&mut self, id: &str) -> Option<&mut Pane> {
        self.chats.iter_mut().find(|(s, _, _)| s == id).map(|(_, p, _)| p)
    }

    pub fn shell_pane(&self, cwd: &Path) -> Option<&Pane> {
        self.shells.get(cwd)
    }

    pub fn shell_pane_mut(&mut self, cwd: &Path) -> Option<&mut Pane> {
        self.shells.get_mut(cwd)
    }

    pub fn is_running(&self, id: &str) -> bool {
        use std::sync::atomic::Ordering;
        self.chats.iter().any(|(s, p, _)| s == id && !p.exited.load(Ordering::SeqCst))
    }

    /// How many running chats there are besides the focused one.
    pub fn running_besides(&self, focused: Option<&str>) -> usize {
        use std::sync::atomic::Ordering;
        self.chats.iter().filter(|(s, p, _)| Some(s.as_str()) != focused && !p.exited.load(Ordering::SeqCst)).count()
    }

    /// Records output from a pane. A background chat that writes becomes unread.
    pub fn on_output(&mut self, pane: PaneId, focused_chat: Option<&str>) {
        self.on_output_at(pane, focused_chat, Instant::now());
    }

    pub fn on_output_at(&mut self, pane: PaneId, focused_chat: Option<&str>, now: Instant) {
        let Some((sid, _, _)) = self.chats.iter().find(|(_, p, _)| p.id == pane) else { return };
        let sid = sid.clone();
        if Some(sid.as_str()) != focused_chat {
            self.unread.insert(sid.clone());
        }
        self.last_output.insert(sid, now);
    }

    /// What to show next to a chat: working while it is writing, unread once it has gone quiet.
    pub fn mark(&self, id: &str) -> Mark {
        self.mark_at(id, Instant::now())
    }

    pub fn mark_at(&self, id: &str, now: Instant) -> Mark {
        let working = self.is_running(id) && self.last_output.get(id).is_some_and(|t| now.saturating_duration_since(*t) < WORKING);
        if working {
            Mark::Working
        } else if self.unread.contains(id) {
            Mark::Unread
        } else {
            Mark::None
        }
    }

    /// Ends every child, chats and shells: SIGHUP to each process group, up to 300ms of grace
    /// for all of them together, then SIGKILL. Returns once they are gone.
    pub fn shutdown(&mut self) {
        let mut groups = Vec::new();
        for (_, p, _) in &mut self.chats {
            groups.extend(p.take_group());
        }
        for p in self.shells.values_mut() {
            groups.extend(p.take_group());
        }
        crate::pane::terminate(groups, crate::pane::GRACE);
    }

    /// Chat pane pids only. Shell pids are excluded on purpose, so a claude
    /// started by hand in ct's shell still counts as live elsewhere.
    pub fn own_chat_pids(&self) -> HashSet<u32> {
        self.chats
            .iter()
            .map(|(_, p, _)| p)
            .filter_map(|p| p.pid())
            .collect()
    }
}
