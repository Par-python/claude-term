use crate::config::Config;
use crate::event::AppEvent;
use crate::history::{ScanCache, SessionMeta, projects_root};
use crate::launch::{fork_spec, new_chat_spec, restart_spec, start_spec};
use crate::live::is_live_elsewhere;
use crate::pane::{Notify, Pane, PaneId, PaneSpec};
use crate::procs::Procs;
use crate::sidebar::Sidebar;
use crate::state;
use crate::tree::{Chat, Folder, group, group_key};
use crossterm::event::Event;
use ratatui::layout::Rect;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Chat,
    Shell,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Modal {
    None,
    Filter(String),
    NewFolder(String),
    ForkPrompt { id: String },
    Message(String),
}

/// A session file touched this recently counts as in use by another claude.
const RECENT: Duration = Duration::from_secs(30);

pub struct App {
    pub sidebar: Sidebar,
    pub procs: Procs,
    pub focus: Focus,
    pub modal: Modal,
    pub zoomed: bool,
    pub sidebar_visible: bool,
    pub open_chat: Option<String>,
    pub pending_new: Vec<(PathBuf, String)>,
    /// folder whose shell is shown below the chat
    pub shell_cwd: Option<PathBuf>,
    pub area: Rect,
    pub(crate) cfg: Config,
    pub(crate) tx: Sender<AppEvent>,
    pub(crate) awaiting_prefix: bool,
    pub(crate) scroll_mode: bool,
    root: PathBuf,
    // test switches: no real claude/shell, no ps, no state file
    test_mode: bool,
    check_ps: bool,
    persist: bool,
    sizes: HashMap<PaneId, (u16, u16)>,
    // scan requests go to the worker thread when there is one
    scan_req: Option<Sender<()>>,
    cache: ScanCache,
    test_script: String,
    last_spec: Option<PaneSpec>,
    /// when the previous ct run exited, from state.toml
    closed_at: Option<SystemTime>,
}

impl App {
    pub fn new(cfg: Config, folders: Vec<Folder>, tx: Sender<AppEvent>) -> App {
        App {
            sidebar: Sidebar::new(folders),
            procs: Procs::new(cfg.max_background),
            focus: Focus::Sidebar,
            modal: Modal::None,
            zoomed: false,
            sidebar_visible: true,
            open_chat: None,
            pending_new: vec![],
            shell_cwd: None,
            area: Rect::new(0, 0, 80, 24),
            cfg,
            tx,
            awaiting_prefix: false,
            scroll_mode: false,
            root: projects_root(),
            test_mode: false,
            check_ps: true,
            persist: true,
            sizes: HashMap::new(),
            scan_req: None,
            cache: ScanCache::default(),
            test_script: "sleep 5".into(),
            last_spec: None,
            closed_at: None,
        }
    }

    /// Spawns nothing on its own. Chats it opens run `sh -c "sleep 5"` and no
    /// shell panes are made. It never reads ps or writes the state file.
    pub fn for_test(folders: Vec<Folder>) -> App {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(Config::default(), folders, tx);
        app.test_mode = true;
        app.check_ps = false;
        app.persist = false;
        app.root = std::env::temp_dir().join("ct-no-such-projects-root");
        app
    }

    /// Swaps the config (tests).
    pub fn set_config(&mut self, cfg: Config) {
        self.cfg = cfg;
    }

    /// Where session files are read from (the live-elsewhere check and rescans).
    pub fn set_root(&mut self, root: PathBuf) {
        self.root = root;
    }

    /// Changes what chats run in test mode: `sh -c <script>`.
    pub fn set_test_script(&mut self, script: &str) {
        self.test_script = script.into();
    }

    /// When the previous ct run exited (unix seconds). Session files last written
    /// up to 2s after that came from our own children, not another claude.
    pub fn set_closed_at(&mut self, secs: u64) {
        self.closed_at = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs));
    }

    /// Hands rescans to a worker thread; without one they run inline.
    pub fn set_scanner(&mut self, req: Sender<()>) {
        self.scan_req = Some(req);
    }

    pub fn set_area(&mut self, area: Rect) {
        self.area = area;
        self.resize_panes();
    }

    /// Returns false when the app should quit.
    pub fn handle(&mut self, ev: AppEvent) -> bool {
        match ev {
            AppEvent::Input(Event::Key(k)) => {
                if k.kind != crossterm::event::KeyEventKind::Release && !self.on_key(k) {
                    return false;
                }
            }
            AppEvent::Input(Event::Resize(w, h)) => self.area = Rect::new(0, 0, w, h),
            AppEvent::Input(Event::Paste(text)) => self.on_paste(&text),
            AppEvent::Input(Event::Mouse(m)) => self.on_mouse(m),
            AppEvent::Input(_) => {}
            AppEvent::Pty(Notify::Output(id)) => self.procs.on_output(id, self.open_chat.as_deref()),
            AppEvent::Pty(Notify::Exited(_)) => {}
            AppEvent::HistoryChanged => match &self.scan_req {
                Some(req) => {
                    let _ = req.send(());
                }
                None => {
                    let metas = self.cache.scan(&self.root);
                    self.apply_scan(metas);
                }
            },
            AppEvent::Scanned(metas) => self.apply_scan(metas),
            AppEvent::Tick => {}
            AppEvent::Quit => return false,
        }
        self.resize_panes();
        self.sync_sidebar_scroll();
        true
    }

    /// Keeps the sidebar's scroll position in step with its cursor and the window height.
    fn sync_sidebar_scroll(&mut self) {
        let l = crate::ui::layout(self.area, self.sidebar_visible, self.zoomed, self.focus);
        if let Some(r) = l.sidebar {
            self.sidebar.ensure_visible(crate::ui::inner(r).height as usize);
        }
    }

    /// Stops every child process and waits for them. Call before exiting.
    pub fn shutdown(&mut self) {
        self.procs.shutdown();
    }

    // ---- history ----

    fn apply_scan(&mut self, metas: Vec<SessionMeta>) {
        let folders = group(metas);
        let found: Vec<&str> = folders.iter().flat_map(|f| f.chats.iter().map(|c| c.meta.id.as_str())).collect();
        self.pending_new.retain(|(_, id)| !found.contains(&id.as_str()));
        self.apply_folders(folders);
    }

    /// Replaces the sidebar folders, adding chats ct started that history doesn't know yet.
    fn apply_folders(&mut self, mut folders: Vec<Folder>) {
        for (cwd, id) in &self.pending_new {
            if folders.iter().any(|f| f.chats.iter().any(|c| &c.meta.id == id)) {
                continue;
            }
            let chat = Chat {
                meta: SessionMeta {
                    id: id.clone(),
                    cwd: cwd.clone(),
                    title: "new chat".into(),
                    modified: SystemTime::now(),
                },
                available: cwd.exists(),
            };
            let key = group_key(cwd);
            match folders.iter_mut().find(|f| f.path == key) {
                Some(f) => f.chats.insert(0, chat),
                None => {
                    let name = key
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| key.to_string_lossy().into_owned());
                    folders.insert(0, Folder { path: key, name, chats: vec![chat], collapsed: false });
                }
            }
        }
        self.sidebar.replace_folders(folders);
    }

    pub(crate) fn find_chat(&self, id: &str) -> Option<&Chat> {
        self.sidebar.folders.iter().flat_map(|f| f.chats.iter()).find(|c| c.meta.id == id)
    }

    // ---- opening and starting chats ----

    /// Opens the chat under the sidebar cursor.
    pub fn open_selected(&mut self) {
        let Some(chat) = self.sidebar.selected_chat().cloned() else { return };
        if !chat.available {
            self.modal = Modal::Message("folder is gone".into());
            return;
        }
        let id = chat.meta.id;
        let cwd = chat.meta.cwd;
        if self.procs.chat(&id).is_some() {
            self.show(id, cwd, true);
        } else if self.live_elsewhere(&id) {
            self.modal = Modal::ForkPrompt { id };
        } else {
            let spec = start_spec(&id, &cwd, &self.root);
            self.start_chat(id, spec, cwd, true);
        }
    }

    pub(crate) fn start_fork(&mut self, id: &str) {
        let Some(cwd) = self.find_chat(id).map(|c| c.meta.cwd.clone()) else { return };
        let (new_id, spec) = fork_spec(id, &cwd);
        self.start_new(new_id, spec, cwd);
    }

    /// `n`: new chat in the selected folder.
    pub(crate) fn new_chat_here(&mut self) {
        match self.sidebar.selected_folder().map(|f| f.path.clone()) {
            None => self.modal = Modal::Message("no folders yet. press N to start one".into()),
            Some(p) if !p.exists() => self.modal = Modal::Message("folder is gone".into()),
            Some(p) => self.new_chat_in(p),
        }
    }

    /// Enter in the new-folder prompt.
    pub(crate) fn new_folder(&mut self, typed: &str) {
        let typed = typed.trim();
        if typed.is_empty() {
            return;
        }
        let path = expand_tilde(typed);
        match std::fs::create_dir_all(&path) {
            Ok(()) => self.new_chat_in(path),
            Err(e) => self.modal = Modal::Message(format!("could not create folder: {e}")),
        }
    }

    fn new_chat_in(&mut self, cwd: PathBuf) {
        let (id, spec) = new_chat_spec(&cwd);
        self.start_new(id, spec, cwd);
    }

    /// Starts a chat with a session id we chose (new chat or fork) and lists it in the sidebar.
    fn start_new(&mut self, id: String, spec: PaneSpec, cwd: PathBuf) {
        self.start_chat(id.clone(), spec, cwd.clone(), true);
        if self.open_chat.as_deref() == Some(id.as_str()) {
            self.pending_new.push((cwd, id.clone()));
            let folders = self.sidebar.folders.clone();
            self.apply_folders(folders);
            self.sidebar.select_by_id(&id);
        }
    }

    fn live_elsewhere(&self, id: &str) -> bool {
        (self.check_ps && is_live_elsewhere(id, &self.procs.own_chat_pids())) || (self.modified_recently(id) && !self.procs.closed_recently(id, RECENT))
    }

    /// New desktop-app chats don't put their id in the ps args, but they write the jsonl.
    fn modified_recently(&self, id: &str) -> bool {
        let Ok(dirs) = std::fs::read_dir(&self.root) else { return false };
        let name = format!("{id}.jsonl");
        dirs.flatten().any(|d| {
            std::fs::metadata(d.path().join(&name))
                .and_then(|m| m.modified())
                .is_ok_and(|t| t.elapsed().is_ok_and(|e| e < RECENT) && !self.ours_from_last_run(t))
        })
    }

    /// The spec the last chat start or restart asked for, before test mode swaps the program.
    pub fn last_spec(&self) -> Option<&PaneSpec> {
        self.last_spec.as_ref()
    }

    fn ours_from_last_run(&self, mtime: SystemTime) -> bool {
        self.closed_at.is_some_and(|c| mtime <= c + Duration::from_secs(2))
    }

    fn adapt(&self, spec: PaneSpec) -> PaneSpec {
        if self.test_mode {
            PaneSpec { program: "sh".into(), args: vec!["-c".into(), self.test_script.clone()], cwd: spec.cwd }
        } else {
            spec
        }
    }

    fn start_chat(&mut self, key: String, spec: PaneSpec, cwd: PathBuf, save: bool) {
        self.last_spec = Some(spec.clone());
        let spec = self.adapt(spec);
        let (rows, cols) = self.pane_size(Focus::Chat);
        let id = self.procs.next_id();
        match Pane::spawn(id, spec, rows, cols, self.tx.clone()) {
            Ok(p) => {
                self.sizes.insert(id, (rows, cols));
                self.procs.insert_chat(key.clone(), p);
                self.show(key, cwd, save);
            }
            Err(e) => self.modal = Modal::Message(format!("could not start claude: {e}")),
        }
    }

    /// Shows a running chat with the shell for its folder, and focuses it.
    fn show(&mut self, key: String, cwd: PathBuf, save: bool) {
        self.procs.chat(&key);
        self.open_chat = Some(key.clone());
        self.focus = Focus::Chat;
        self.scroll_mode = false;
        self.ensure_shell(&cwd);
        self.shell_cwd = Some(cwd);
        if save && self.persist {
            state::save_last(&key);
        }
    }

    fn ensure_shell(&mut self, cwd: &Path) {
        if self.test_mode {
            return;
        }
        let (rows, cols) = self.pane_size(Focus::Shell);
        match self.procs.shell_for(cwd, rows, cols, self.tx.clone()) {
            Ok(p) => {
                let id = p.id;
                self.sizes.entry(id).or_insert((rows, cols));
            }
            Err(e) => self.modal = Modal::Message(format!("could not start shell: {e}")),
        }
    }

    pub(crate) fn restart_focused(&mut self) {
        let tx = self.tx.clone();
        // a chat that now has history resumes instead of repeating its creation flags
        let new_spec = match (self.focus, &self.open_chat) {
            (Focus::Chat, Some(id)) => self
                .procs
                .chat_pane(id)
                .map(|p| restart_spec(&p.spec, id, &p.spec.cwd, &self.root)),
            _ => None,
        };
        if let Some(s) = &new_spec {
            self.last_spec = Some(s.clone());
        }
        let new_spec = new_spec.map(|s| self.adapt(s));
        let res = match (self.focus, &self.open_chat, &self.shell_cwd) {
            (Focus::Chat, Some(id), _) => self.procs.chat(id).map(|p| {
                if let Some(s) = new_spec {
                    p.spec = s;
                }
                p.restart(tx)
            }),
            (Focus::Shell, _, Some(cwd)) => self.procs.shell_pane_mut(cwd).map(|p| p.restart(tx)),
            _ => None,
        };
        if let Some(Err(e)) = res {
            self.modal = Modal::Message(format!("could not restart: {e}"));
        }
    }

    pub(crate) fn focused_exited(&self) -> bool {
        use std::sync::atomic::Ordering;
        match self.focus {
            Focus::Chat => self.open_chat.as_deref().and_then(|id| self.procs.chat_pane(id)),
            Focus::Shell => self.shell_cwd.as_deref().and_then(|c| self.procs.shell_pane(c)),
            Focus::Sidebar => None,
        }
        .is_some_and(|p| p.exited.load(Ordering::SeqCst))
    }

    pub(crate) fn write_focused(&mut self, bytes: &[u8]) {
        let pane = match self.focus {
            Focus::Chat => match &self.open_chat {
                Some(id) => self.procs.chat(id),
                None => None,
            },
            Focus::Shell => match &self.shell_cwd {
                Some(c) => self.procs.shell_pane_mut(c),
                None => None,
            },
            Focus::Sidebar => None,
        };
        if let Some(p) = pane {
            // typing goes to the live screen
            p.scroll_to(0);
            p.write(bytes);
        }
    }

    /// Whether the focused pane asked for bracketed paste.
    fn focused_wants_bracketed_paste(&self) -> bool {
        self.focused_pane().is_some_and(|p| p.screen_lock().screen().bracketed_paste())
    }

    pub(crate) fn on_paste(&mut self, text: &str) {
        match &mut self.modal {
            Modal::Filter(s) => {
                s.push_str(text);
                let f = s.clone();
                self.sidebar.set_filter(&f);
            }
            Modal::NewFolder(s) => s.push_str(text),
            Modal::None => {
                if self.focus == Focus::Sidebar {
                    return;
                }
                let mut bytes = Vec::new();
                let wrap = self.focused_wants_bracketed_paste();
                if wrap {
                    bytes.extend_from_slice(b"\x1b[200~");
                }
                bytes.extend_from_slice(text.as_bytes());
                if wrap {
                    bytes.extend_from_slice(b"\x1b[201~");
                }
                self.write_focused(&bytes);
            }
            _ => {}
        }
    }

    /// True when a pane on screen is mid synchronized-output block, so drawing should wait.
    pub fn hold_draw(&self) -> bool {
        let l = crate::ui::layout(self.area, self.sidebar_visible, self.zoomed, self.focus);
        let chat = l.chat.and(self.open_chat.as_deref()).and_then(|id| self.procs.chat_pane(id));
        let shell = l.shell.and(self.shell_cwd.as_deref()).and_then(|c| self.procs.shell_pane(c));
        chat.into_iter().chain(shell).any(|p| p.in_sync_block())
    }

    // ---- sizing ----

    fn pane_size(&self, which: Focus) -> (u16, u16) {
        let l = crate::ui::layout(self.area, self.sidebar_visible, self.zoomed, which);
        let r = if which == Focus::Chat { l.chat } else { l.shell };
        r.map(crate::ui::inner).map_or((24, 80), |i| (i.height, i.width))
    }

    /// Resizes each visible pane to its inner rect, only when the size changed.
    pub fn resize_panes(&mut self) {
        let l = crate::ui::layout(self.area, self.sidebar_visible, self.zoomed, self.focus);
        if let (Some(r), Some(id)) = (l.chat, self.open_chat.as_deref()) {
            let i = crate::ui::inner(r);
            if let Some(p) = self.procs.chat(id)
                && self.sizes.get(&p.id) != Some(&(i.height, i.width)) {
                    p.resize(i.height, i.width);
                    self.sizes.insert(p.id, (i.height, i.width));
                }
        }
        if let (Some(r), Some(cwd)) = (l.shell, self.shell_cwd.as_deref()) {
            let i = crate::ui::inner(r);
            if let Some(p) = self.procs.shell_pane_mut(cwd)
                && self.sizes.get(&p.id) != Some(&(i.height, i.width)) {
                    p.resize(i.height, i.width);
                    self.sizes.insert(p.id, (i.height, i.width));
                }
        }
    }
}

fn expand_tilde(s: &str) -> PathBuf {
    if s == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    }
    match (s.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(h)) => h.join(rest),
        _ => PathBuf::from(s),
    }
}
