use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ct::app::{App, Focus, Modal};
use ct::event::AppEvent;
use ct::history::SessionMeta;
use ct::tree::{Chat, Folder};
use ct::ui::{draw, layout};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

fn chat(id: &str, cwd: &Path, title: &str, available: bool) -> Chat {
    Chat {
        meta: SessionMeta {
            id: id.into(),
            cwd: cwd.to_path_buf(),
            title: title.into(),
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(1000),
        },
        available,
    }
}

fn folder(path: &Path, chats: Vec<Chat>) -> Folder {
    Folder {
        path: path.to_path_buf(),
        name: path.file_name().unwrap().to_string_lossy().into_owned(),
        chats,
        collapsed: false,
    }
}

fn fixture() -> (tempfile::TempDir, Vec<Folder>) {
    let td = tempfile::tempdir().unwrap();
    let ce = td.path().join("my-app");
    let other = td.path().join("other");
    std::fs::create_dir_all(&ce).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let folders = vec![
        folder(
            &ce,
            vec![
                chat("a1", &ce, "alpha chat", true),
                chat("a2", &ce, "second chat", true),
                chat("a3", &ce, "third chat", true),
            ],
        ),
        folder(&other, vec![chat("b1", &other, "beta chat", true)]),
    ];
    (td, folders)
}

fn key(code: KeyCode, m: KeyModifiers) -> AppEvent {
    AppEvent::Input(Event::Key(KeyEvent::new(code, m)))
}
fn ch(c: char) -> AppEvent {
    key(KeyCode::Char(c), KeyModifiers::NONE)
}
fn alt(c: char) -> AppEvent {
    key(KeyCode::Char(c), KeyModifiers::ALT)
}
fn prefix() -> AppEvent {
    key(KeyCode::Char(' '), KeyModifiers::CONTROL)
}

fn render(app: &App) -> Vec<String> {
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    t.draw(|f| draw(f, app)).unwrap();
    let buf = t.backend().buffer();
    buf.content().chunks(80).map(|r| r.iter().map(|c| c.symbol()).collect::<String>()).collect()
}

#[test]
fn sidebar_renders_headers_numbers_and_markers() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.set_test_script("printf hi; sleep 5");
    assert!(app.handle(alt('1')));
    wait_for("output", || screen_text(&app, "a1").contains("hi"));
    // the loop would have been told about the output
    let pid = app.procs.chat_pane("a1").unwrap().id;
    app.handle(AppEvent::Pty(ct::pane::Notify::Output(pid)));
    let lines = render(&app);
    assert!(lines.iter().any(|l| l.contains("my-app ▾")), "{lines:#?}");
    assert!(lines.iter().any(|l| l.contains("  0 ● alpha chat")), "{lines:#?}");
    assert!(lines.iter().any(|l| l.contains("  1   second chat")), "{lines:#?}");
}

#[test]
fn empty_state_message() {
    let app = App::for_test(vec![]);
    // the sidebar is 32 wide, so the message wraps; read just that column
    let col: Vec<String> = render(&app).iter().map(|l| l.chars().take(32).collect()).collect();
    let text = col.iter().map(|l| l.trim_matches(|c| c == '│' || c == ' ')).collect::<Vec<_>>().join(" ");
    assert!(text.contains("no chats yet. press N to start one"), "{text}");
}

#[test]
fn hidden_sidebar_gives_full_width_to_panes() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    let area = Rect::new(0, 0, 80, 24);
    let before = layout(area, app.sidebar_visible, app.zoomed, app.focus);
    assert_eq!(before.chat.unwrap().width, 80 - 32);
    app.handle(prefix());
    app.handle(ch('b'));
    assert!(!app.sidebar_visible);
    let after = layout(area, app.sidebar_visible, app.zoomed, app.focus);
    assert!(after.sidebar.is_none());
    assert_eq!(after.chat.unwrap().width, 80);
    assert_eq!(after.shell.unwrap().width, 80);
}

#[test]
fn jump_moves_and_opens() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.handle(alt('2'));
    assert_eq!(app.open_chat.as_deref(), Some("a2"));
    assert_eq!(app.focus, Focus::Chat);
    // jump up with the shifted symbol
    app.handle(alt('!'));
    assert_eq!(app.open_chat.as_deref(), Some("a1"));
}

#[test]
fn unavailable_chat_shows_message() {
    let td = tempfile::tempdir().unwrap();
    let gone = td.path().join("gone");
    let folders = vec![folder(&gone, vec![chat("g1", &gone, "lost chat", false)])];
    let mut app = App::for_test(folders);
    app.handle(alt('1'));
    assert_eq!(app.open_chat, None);
    assert!(matches!(&app.modal, Modal::Message(m) if m == "folder is gone"), "{:?}", app.modal);
    let text = render(&app).join("\n");
    assert!(text.contains("folder is gone"));
}

#[test]
fn recent_session_file_triggers_fork_prompt_then_f_forks() {
    let (td, folders) = fixture();
    let root = td.path().join("projects");
    std::fs::create_dir_all(root.join("proj")).unwrap();
    std::fs::write(root.join("proj").join("a1.jsonl"), "{}\n").unwrap();
    let mut app = App::for_test(folders);
    app.set_root(root);
    app.handle(alt('1'));
    assert!(matches!(&app.modal, Modal::ForkPrompt { id } if id == "a1"), "{:?}", app.modal);
    assert_eq!(app.open_chat, None);
    let text = render(&app).join("\n");
    assert!(text.contains("chat open elsewhere: f = open a copy, esc = cancel"));
    app.handle(ch('f'));
    assert!(matches!(app.modal, Modal::None));
    assert!(app.open_chat.is_some());
    assert_eq!(app.focus, Focus::Chat);
}

#[test]
fn esc_cancels_fork_prompt() {
    let (td, folders) = fixture();
    let root = td.path().join("projects");
    std::fs::create_dir_all(root.join("proj")).unwrap();
    std::fs::write(root.join("proj").join("a1.jsonl"), "{}\n").unwrap();
    let mut app = App::for_test(folders);
    app.set_root(root);
    app.handle(alt('1'));
    app.handle(key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(matches!(app.modal, Modal::None));
    assert_eq!(app.open_chat, None);
}

#[test]
fn n_without_folders_tells_user_to_press_capital_n() {
    let mut app = App::for_test(vec![]);
    app.handle(ch('n'));
    assert!(matches!(&app.modal, Modal::Message(m) if m.contains("N")), "{:?}", app.modal);
}

#[test]
fn capital_n_creates_folder_and_starts_chat() {
    let td = tempfile::tempdir().unwrap();
    let mut app = App::for_test(vec![]);
    app.handle(ch('N'));
    assert!(matches!(app.modal, Modal::NewFolder(_)));
    let dir: PathBuf = td.path().join("fresh");
    for c in dir.to_string_lossy().chars() {
        app.handle(ch(c));
    }
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(dir.is_dir());
    assert_eq!(app.pending_new.len(), 1);
    assert_eq!(app.open_chat.as_deref(), Some(app.pending_new[0].1.as_str()));
    assert_eq!(app.sidebar.folders.len(), 1);
    assert_eq!(app.sidebar.folders[0].path, dir);
}

#[test]
fn history_scan_clears_pending_new() {
    // scan skips sessions under the system temp dir, so work under target/
    let td = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/target")).unwrap();
    let cwd = td.path().join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let root = td.path().join("projects");
    std::fs::create_dir_all(root.join("proj")).unwrap();
    let line = format!("{{\"type\":\"user\",\"cwd\":{:?},\"message\":{{\"content\":\"hi\"}}}}\n", cwd.to_string_lossy());
    std::fs::write(root.join("proj").join("p1.jsonl"), line).unwrap();
    let mut app = App::for_test(vec![]);
    app.set_root(root);
    app.pending_new.push((cwd.clone(), "p1".into()));
    app.pending_new.push((cwd.clone(), "p2".into()));
    app.handle(AppEvent::HistoryChanged);
    assert_eq!(app.pending_new, vec![(cwd, "p2".to_string())]);
    let ids: Vec<&str> = app.sidebar.folders.iter().flat_map(|f| f.chats.iter().map(|c| c.meta.id.as_str())).collect();
    assert!(ids.contains(&"p1") && ids.contains(&"p2"), "{ids:?}");
}

#[test]
fn sidebar_keys_only_work_with_sidebar_focus_and_esc_clears_filter() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.handle(ch('/'));
    app.handle(ch('b'));
    app.handle(ch('e'));
    assert_eq!(app.sidebar.filter(), "be");
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(app.modal, Modal::None));
    assert_eq!(app.sidebar.filter(), "be");
    app.handle(key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.sidebar.filter(), "");
    // focus moves away: 'j' no longer moves the sidebar cursor
    app.handle(prefix());
    app.handle(ch('l'));
    assert_eq!(app.focus, Focus::Chat);
    let cur = app.sidebar.cursor;
    app.handle(ch('j'));
    assert_eq!(app.sidebar.cursor, cur);
}

#[test]
fn prefix_q_quits() {
    let mut app = App::for_test(vec![]);
    app.handle(prefix());
    assert!(!app.handle(ch('q')));
}

fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..300 {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

fn screen_text(app: &App, id: &str) -> String {
    app.procs.chat_pane(id).unwrap().screen_lock().screen().contents()
}

#[test]
fn option_symbols_jump_only_with_sidebar_focus() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.handle(alt('1'));
    assert_eq!(app.focus, Focus::Chat);
    let cursor = app.sidebar.cursor;
    // typing a pound sign into the chat must not jump
    app.handle(ch('£'));
    assert_eq!(app.sidebar.cursor, cursor);
    assert_eq!(app.open_chat.as_deref(), Some("a1"));
    // with the sidebar focused it jumps three chats down
    app.handle(prefix());
    app.handle(ch('h'));
    app.handle(ch('£'));
    assert_eq!(app.open_chat.as_deref(), Some("b1"));
}

#[test]
fn unread_dot_wins_over_running_dot() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.handle(alt('1'));
    app.procs.unread.insert("a1".into());
    let lines = render(&app);
    assert!(lines.iter().any(|l| l.contains("  0 • alpha chat")), "{lines:#?}");
}

#[test]
fn paste_is_bracketed_only_when_the_pane_asked() {
    for bracketed in [true, false] {
        let (td, folders) = fixture();
        let out = td.path().join("pasted");
        let enable = if bracketed { "printf '\\033[?2004h'; " } else { "" };
        let n = if bracketed { 14 } else { 2 };
        let script = format!("stty raw -echo; {enable}printf ready; head -c {n} > {}", out.display());
        let mut app = App::for_test(folders);
        app.set_test_script(&script);
        app.handle(alt('1'));
        wait_for("pane ready", || screen_text(&app, "a1").contains("ready"));
        assert_eq!(app.procs.chat_pane("a1").unwrap().screen_lock().screen().bracketed_paste(), bracketed);
        app.handle(AppEvent::Input(Event::Paste("hi".into())));
        wait_for("pasted bytes", || std::fs::metadata(&out).map(|m| m.len() >= n as u64).unwrap_or(false));
        let got = std::fs::read(&out).unwrap();
        let want: &[u8] = if bracketed { b"\x1b[200~hi\x1b[201~" } else { b"hi" };
        assert_eq!(got, want);
    }
}

#[test]
fn paste_goes_into_prompts_and_never_triggers_jumps() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.handle(ch('/'));
    app.handle(AppEvent::Input(Event::Paste("beta".into())));
    assert_eq!(app.sidebar.filter(), "beta");
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    // sidebar focus, no modal: ignored (and no jump from the symbols inside)
    app.handle(AppEvent::Input(Event::Paste("¡™£".into())));
    assert_eq!(app.open_chat, None);
    app.handle(ch('N'));
    app.handle(AppEvent::Input(Event::Paste("/tmp/x".into())));
    assert!(matches!(&app.modal, Modal::NewFolder(s) if s == "/tmp/x"));
}

fn pane_origin(app: &App) -> (u16, u16) {
    let l = layout(Rect::new(0, 0, 80, 24), app.sidebar_visible, app.zoomed, app.focus);
    let i = ct::ui::inner(l.chat.unwrap());
    (i.x, i.y)
}

fn buffer_of(app: &App) -> ratatui::buffer::Buffer {
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    t.draw(|f| draw(f, app)).unwrap();
    t.backend().buffer().clone()
}

#[test]
fn pane_cells_render_color_and_wide_chars() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.set_test_script("printf '\\033[31mhi 中\\033[0mx'; sleep 5");
    app.handle(alt('1'));
    wait_for("output", || screen_text(&app, "a1").contains("hi"));
    let buf = buffer_of(&app);
    let (x, y) = pane_origin(&app);
    assert_eq!(buf[(x, y)].symbol(), "h");
    assert_eq!(buf[(x, y)].fg, ratatui::style::Color::Indexed(1));
    assert_eq!(buf[(x + 1, y)].symbol(), "i");
    assert_eq!(buf[(x + 1, y)].fg, ratatui::style::Color::Indexed(1));
    assert_eq!(buf[(x + 3, y)].symbol(), "中");
    // the wide char covers two cells, so the next char lands at +5
    assert_eq!(buf[(x + 5, y)].symbol(), "x");
    assert_eq!(buf[(x + 5, y)].fg, ratatui::style::Color::Reset);
}

#[test]
fn exited_pane_shows_banner_and_enter_restarts_it() {
    use std::sync::atomic::Ordering;
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.set_test_script("printf done");
    app.handle(alt('1'));
    wait_for("exit", || app.procs.chat_pane("a1").unwrap().exited.load(Ordering::SeqCst));
    let text = render(&app).join("\n");
    assert!(text.contains("[process exited, press enter to restart]"), "{text}");
    let old_pid = app.procs.chat_pane("a1").unwrap().pid();
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    // restart runs the pane's spec again, so check for a new child
    let new_pid = app.procs.chat_pane("a1").unwrap().pid();
    assert!(new_pid.is_some());
    assert_ne!(new_pid, old_pid);
}

#[test]
fn chat_ct_just_evicted_does_not_trigger_fork_prompt() {
    let (td, folders) = fixture();
    let root = td.path().join("projects");
    std::fs::create_dir_all(root.join("proj")).unwrap();
    std::fs::write(root.join("proj").join("a3.jsonl"), "{}\n").unwrap();
    let mut app = App::for_test(folders);
    app.procs = ct::procs::Procs::new(0);
    app.set_root(root.clone());
    app.handle(alt('1'));
    assert_eq!(app.open_chat.as_deref(), Some("a1"));
    // opening a2 evicts a1; its file is then written fresh (as claude would)
    app.handle(alt('1'));
    assert_eq!(app.open_chat.as_deref(), Some("a2"));
    std::fs::write(root.join("proj").join("a1.jsonl"), "{}\n").unwrap();
    app.handle(alt('!'));
    assert_eq!(app.open_chat.as_deref(), Some("a1"), "{:?}", app.modal);
    // a3 was never run by ct, so its fresh file still counts as elsewhere
    app.handle(alt('2'));
    assert!(matches!(&app.modal, Modal::ForkPrompt { id } if id == "a3"), "{:?}", app.modal);
}

#[test]
fn fork_is_listed_and_keyed_by_its_new_id() {
    let (td, folders) = fixture();
    let root = td.path().join("projects");
    std::fs::create_dir_all(root.join("proj")).unwrap();
    std::fs::write(root.join("proj").join("a1.jsonl"), "{}\n").unwrap();
    let mut app = App::for_test(folders);
    app.set_root(root);
    app.handle(alt('1'));
    app.handle(ch('f'));
    let id = app.open_chat.clone().unwrap();
    assert_ne!(id, "a1");
    assert_eq!(app.pending_new.len(), 1);
    assert_eq!(app.pending_new[0].1, id);
    assert!(app.procs.is_running(&id));
    assert_eq!(app.sidebar.selected_chat().unwrap().meta.id, id);
}

#[test]
fn scanner_worker_replies_with_scanned() {
    let td = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/target")).unwrap();
    let cwd = td.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();
    let root = td.path().join("projects");
    std::fs::create_dir_all(root.join("p")).unwrap();
    let line = format!("{{\"type\":\"user\",\"cwd\":{:?},\"message\":{{\"content\":\"hi\"}}}}\n", cwd.to_string_lossy());
    std::fs::write(root.join("p").join("s1.jsonl"), line).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let req = ct::watch::spawn_scanner(root, ct::history::ScanCache::default(), tx);
    for _ in 0..5 {
        req.send(()).unwrap();
    }
    let AppEvent::Scanned(metas) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else { panic!("not scanned") };
    assert_eq!(metas.len(), 1);
    // a Scanned event updates the app like a rescan does
    let mut app = App::for_test(vec![]);
    app.handle(AppEvent::Scanned(metas));
    assert_eq!(app.sidebar.folders.len(), 1);
}

#[test]
fn reopening_an_evicted_unused_new_chat_uses_session_id() {
    let (td, folders) = fixture();
    let root = td.path().join("projects");
    std::fs::create_dir_all(root.join("proj")).unwrap();
    let mut app = App::for_test(folders);
    app.procs = ct::procs::Procs::new(0);
    app.set_root(root);
    app.handle(ch('n'));
    let id = app.open_chat.clone().unwrap();
    assert_eq!(app.last_spec().unwrap().args, vec!["--session-id".to_string(), id.clone()]);
    // opening another chat evicts the new one before claude ever wrote history
    app.sidebar.select_by_id("b1");
    app.open_selected();
    assert!(app.procs.chat_pane(&id).is_none());
    app.sidebar.select_by_id(&id);
    app.open_selected();
    assert_eq!(app.last_spec().unwrap().args, vec!["--session-id".to_string(), id.clone()], "{:?}", app.modal);
    // an existing chat with history still resumes
    let f = td.path().join("projects").join("proj").join("a1.jsonl");
    std::fs::write(&f, "{}\n").unwrap();
    // old enough that it doesn't look in use elsewhere
    std::fs::File::options().write(true).open(&f).unwrap().set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
    app.sidebar.select_by_id("a1");
    app.open_selected();
    assert_eq!(app.last_spec().unwrap().args, vec!["--resume", "a1"]);
}

#[test]
fn restarting_a_chat_that_now_has_history_resumes_it() {
    use std::sync::atomic::Ordering;
    let (td, folders) = fixture();
    let root = td.path().join("projects");
    std::fs::create_dir_all(root.join("proj")).unwrap();
    let mut app = App::for_test(folders);
    app.set_root(root.clone());
    app.set_test_script("exit 0");
    app.handle(ch('n'));
    let id = app.open_chat.clone().unwrap();
    wait_for("exit", || app.procs.chat_pane(&id).unwrap().exited.load(Ordering::SeqCst));
    // claude wrote its history before dying
    std::fs::write(root.join("proj").join(format!("{id}.jsonl")), "{}\n").unwrap();
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.last_spec().unwrap().args, vec!["--resume".to_string(), id]);
}

#[test]
fn quick_relaunch_skips_fork_prompt_for_files_from_the_last_run() {
    let (td, folders) = fixture();
    let root = td.path().join("projects");
    std::fs::create_dir_all(root.join("proj")).unwrap();
    let f = root.join("proj").join("a1.jsonl");
    std::fs::write(&f, "{}\n").unwrap();
    let mut app = App::for_test(folders);
    app.set_root(root);
    // ct closed just now: the fresh file is ours
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
    app.set_closed_at(now);
    app.handle(alt('1'));
    assert_eq!(app.open_chat.as_deref(), Some("a1"), "{:?}", app.modal);
    // a file written well after that close is someone else's
    let (td2, folders2) = fixture();
    let root2 = td2.path().join("projects");
    std::fs::create_dir_all(root2.join("proj")).unwrap();
    std::fs::write(root2.join("proj").join("a1.jsonl"), "{}\n").unwrap();
    let mut app2 = App::for_test(folders2);
    app2.set_root(root2);
    app2.set_closed_at(now - 100);
    app2.handle(alt('1'));
    assert!(matches!(&app2.modal, Modal::ForkPrompt { id } if id == "a1"), "{:?}", app2.modal);
}

#[test]
fn message_is_dismissed_and_the_key_still_works() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.modal = Modal::Message("folder is gone".into());
    let before = app.sidebar.cursor;
    app.handle(ch('j'));
    assert!(matches!(app.modal, Modal::None));
    assert_ne!(app.sidebar.cursor, before, "the j that dismissed the message must also move the cursor");
}

fn mouse(kind: crossterm::event::MouseEventKind, x: u16, y: u16) -> AppEvent {
    AppEvent::Input(Event::Mouse(crossterm::event::MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE }))
}

fn wheel(app: &mut App, up: bool, x: u16, y: u16) {
    use crossterm::event::MouseEventKind::{ScrollDown, ScrollUp};
    app.handle(mouse(if up { ScrollUp } else { ScrollDown }, x, y));
}

fn offset(app: &App) -> usize {
    app.procs.chat_pane("a1").unwrap().scroll_offset()
}

/// an app with a chat that printed 100 lines
fn scrolled_app() -> (tempfile::TempDir, App) {
    let (td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.set_test_script("seq 1 100; sleep 5");
    app.handle(alt('1'));
    wait_for("output", || screen_text(&app, "a1").contains("100"));
    (td, app)
}

#[test]
fn wheel_over_a_pane_scrolls_its_history() {
    let (_td, mut app) = scrolled_app();
    let (x, y) = pane_origin(&app);
    let live = render(&app).join("\n");
    assert!(live.contains("100"));
    wheel(&mut app, true, x + 1, y + 1);
    assert_eq!(offset(&app), 3);
    // the view moved back: the newest lines are gone
    let scrolled = render(&app);
    assert!(!scrolled.join("\n").contains("100"), "{scrolled:#?}");
    assert!(scrolled.iter().any(|l| l.contains("scroll: 3 lines up")), "{scrolled:#?}");
    wheel(&mut app, false, x + 1, y + 1);
    assert_eq!(offset(&app), 0);
    // never below the live screen
    wheel(&mut app, false, x + 1, y + 1);
    assert_eq!(offset(&app), 0);
}

#[test]
fn wheel_over_the_sidebar_moves_the_cursor_not_the_pane() {
    let (_td, mut app) = scrolled_app();
    let before = app.sidebar.cursor;
    wheel(&mut app, false, 3, 3);
    assert!(app.sidebar.cursor > before);
    assert_eq!(offset(&app), 0);
}

#[test]
fn wheel_goes_to_the_program_when_it_wants_mouse_events() {
    let (td, folders) = fixture();
    let out = td.path().join("mouse");
    let script = format!("stty raw -echo; printf '\\033[?1000h\\033[?1006h'; printf ready; head -c 20 > {}", out.display());
    let mut app = App::for_test(folders);
    app.set_test_script(&script);
    app.handle(alt('1'));
    wait_for("pane ready", || screen_text(&app, "a1").contains("ready"));
    let (x, y) = pane_origin(&app);
    wheel(&mut app, true, x + 2, y + 3);
    wheel(&mut app, false, x + 2, y + 3);
    wait_for("mouse bytes", || std::fs::metadata(&out).map(|m| m.len() >= 20).unwrap_or(false));
    assert_eq!(std::fs::read(&out).unwrap(), b"\x1b[<64;3;4M\x1b[<65;3;4M");
    // and nothing scrolled locally
    assert_eq!(offset(&app), 0);
}

#[test]
fn prefix_bracket_enters_scroll_mode_and_q_returns_to_the_bottom() {
    let (_td, mut app) = scrolled_app();
    app.handle(prefix());
    app.handle(ch('['));
    assert!(app.scroll_mode());
    app.handle(ch('k'));
    app.handle(ch('k'));
    assert_eq!(offset(&app), 2);
    app.handle(ch('j'));
    assert_eq!(offset(&app), 1);
    app.handle(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert!(offset(&app) > 1);
    app.handle(ch('G'));
    assert_eq!(offset(&app), 0);
    app.handle(ch('g'));
    let max = app.procs.chat_pane("a1").unwrap().scroll_max();
    assert!(max > 50 && offset(&app) == max, "{} {max}", offset(&app));
    let text = render(&app).join("\n");
    assert!(text.contains(&format!("scroll: {max} lines up")), "{text}");
    app.handle(ch('q'));
    assert!(!app.scroll_mode());
    assert_eq!(offset(&app), 0);
}

#[test]
fn typing_into_a_pane_snaps_back_to_the_bottom() {
    let (_td, mut app) = scrolled_app();
    let (x, y) = pane_origin(&app);
    wheel(&mut app, true, x, y);
    assert_eq!(offset(&app), 3);
    app.handle(ch('x'));
    assert_eq!(offset(&app), 0);
}

#[test]
fn scroll_mode_keys_do_not_reach_the_pane() {
    let (_td, mut app) = scrolled_app();
    app.handle(prefix());
    app.handle(ch('['));
    app.handle(ch('x'));
    assert!(!screen_text(&app, "a1").contains('x'));
}

#[test]
fn clicks_focus_panes_and_open_sidebar_chats() {
    use crossterm::event::{MouseButton, MouseEventKind::Down};
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    // sidebar rows: 0 header, 1 a1, 2 a2 inside a 1-cell border
    app.handle(mouse(Down(MouseButton::Left), 4, 1 + 2));
    assert_eq!(app.open_chat.as_deref(), Some("a2"));
    assert_eq!(app.focus, Focus::Chat);
    app.handle(prefix());
    app.handle(ch('h'));
    assert_eq!(app.focus, Focus::Sidebar);
    let (x, y) = pane_origin(&app);
    app.handle(mouse(Down(MouseButton::Left), x + 1, y + 1));
    assert_eq!(app.focus, Focus::Chat);
}

#[test]
fn mouse_can_be_turned_off() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.set_config(ct::config::parse_config("mouse = false"));
    let before = app.sidebar.cursor;
    wheel(&mut app, false, 3, 3);
    assert_eq!(app.sidebar.cursor, before);
}

#[test]
fn drawing_is_held_while_a_visible_pane_is_mid_sync_block() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.set_test_script("printf '\\033[?2026hhalf'; sleep 5");
    assert!(!app.hold_draw());
    app.handle(alt('1'));
    wait_for("sync block", || screen_text(&app, "a1").contains("half"));
    assert!(app.hold_draw());
}
