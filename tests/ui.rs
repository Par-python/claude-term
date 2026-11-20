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
        empty: false,
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
    assert!(lines.iter().any(|l| l.starts_with(" MY-APP")), "{lines:#?}");
    assert!(lines.iter().any(|l| l.contains(" 0  alpha chat")), "{lines:#?}");
    assert!(lines.iter().any(|l| l.contains("  1  second chat")), "{lines:#?}");
    let row = lines.iter().find(|l| l.chars().take(32).collect::<String>().contains("alpha chat")).unwrap();
    assert_eq!(row.chars().nth(31), Some('●'), "{row}");
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
    let before = layout(area, app.sidebar_visible, app.zoomed, app.focus, app.env_visible());
    assert_eq!(before.chat.unwrap().width, 80 - 32 - 1);
    app.handle(prefix());
    app.handle(ch('b'));
    assert!(!app.sidebar_visible);
    let after = layout(area, app.sidebar_visible, app.zoomed, app.focus, app.env_visible());
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
    // p2 isn't running and history doesn't have it, so it goes too
    assert!(app.pending_new.is_empty(), "{:?}", app.pending_new);
    let ids: Vec<&str> = app.sidebar.folders.iter().flat_map(|f| f.chats.iter().map(|c| c.meta.id.as_str())).collect();
    assert!(ids.contains(&"p1") && !ids.contains(&"p2"), "{ids:?}");
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
    let row = lines.iter().find(|l| l.chars().take(32).collect::<String>().contains("alpha chat")).unwrap();
    assert_eq!(row.chars().nth(31), Some('•'), "{row}");
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
    let l = layout(Rect::new(0, 0, 80, 24), app.sidebar_visible, app.zoomed, app.focus, app.env_visible());
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
    // sidebar rows: title line, then 0 header, 1 a1, 2 a2
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

use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ct::theme;

fn buf_at(app: &App, w: u16, h: u16) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| draw(f, app)).unwrap();
    t.backend().buffer().clone()
}

fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

fn find_row(buf: &Buffer, needle: &str) -> Option<u16> {
    // sidebar columns only, so pane titles with the same text don't match
    (0..buf.area.height).find(|&y| row_text(buf, y).chars().take(32).collect::<String>().contains(needle))
}

#[test]
fn no_box_borders() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.handle(alt('1'));
    let text = render(&app).join("\n");
    for c in ['┌', '┐', '└', '┘'] {
        assert!(!text.contains(c), "found {c}");
    }
}

#[test]
fn vertical_divider_between_sidebar_and_panes() {
    let (_td, folders) = fixture();
    let app = App::for_test(folders);
    let buf = buf_at(&app, 80, 24);
    for y in 0..23 {
        assert_eq!(buf[(32, y)].symbol(), "│", "row {y}");
        assert_eq!(buf[(32, y)].fg, theme::DIVIDER, "row {y}");
    }
    // the right side is split by a horizontal divider
    let y = (0..23).find(|&y| buf[(40, y)].symbol() == "─").expect("horizontal divider");
    assert_eq!(buf[(40, y)].fg, theme::DIVIDER);
}

#[test]
fn folder_headers_uppercase_dim() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.handle(alt('1'));
    let buf = buf_at(&app, 80, 24);
    let y1 = find_row(&buf, "MY-APP").expect("first header");
    assert_eq!(buf[(1, y1)].symbol(), "M");
    assert_eq!(buf[(1, y1)].fg, theme::TEXT_DIM);
    let y2 = find_row(&buf, "OTHER").expect("second header");
    assert_eq!(buf[(1, y2)].fg, theme::TEXT_DIM);
    // blank row above the second header, none above the first
    assert_eq!(row_text(&buf, y2 - 1).chars().take(32).collect::<String>().trim(), "");
    assert_eq!(y1, 1);
    // a collapsed header gets a faint arrow
    let (_td2, mut folders2) = fixture();
    folders2[1].collapsed = true;
    let app2 = App::for_test(folders2);
    let b2 = buf_at(&app2, 80, 24);
    let y = find_row(&b2, "OTHER ▸").expect("collapsed header");
    let x = row_text(&b2, y).chars().position(|c| c == '▸').unwrap() as u16;
    assert_eq!(b2[(x, y)].fg, theme::TEXT_FAINT);
}

#[test]
fn selected_row_style() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.handle(alt('2'));
    let buf = buf_at(&app, 80, 24);
    let y = find_row(&buf, "second chat").unwrap();
    assert_eq!(buf[(0, y)].symbol(), "▌");
    assert_eq!(buf[(0, y)].fg, theme::ACCENT);
    for x in 0..32 {
        assert_eq!(buf[(x, y)].bg, theme::SELECTION_BG, "col {x}");
    }
    assert_eq!(buf[(32, y)].bg, Color::Reset);
    let tx = row_text(&buf, y).chars().position(|c| c == 's').unwrap() as u16;
    for x in tx..tx + 11 {
        assert_eq!(buf[(x, y)].fg, theme::TEXT_BRIGHT, "col {x}");
    }
    assert_eq!(buf[(2, y)].symbol(), "0");
    assert_eq!(buf[(2, y)].fg, theme::ACCENT);
    // an unselected chat row has no background and a faint number
    let y1 = find_row(&buf, "third chat").unwrap();
    assert_eq!(buf[(0, y1)].bg, Color::Reset);
    assert_eq!(buf[(2, y1)].fg, theme::TEXT_FAINT);
    assert_eq!(buf[(6, y1)].fg, theme::TEXT);
    // the cursor on a header gets the same treatment
    app.handle(prefix());
    app.handle(ch('h'));
    app.handle(ch('g'));
    app.handle(ch('g'));
    let buf = buf_at(&app, 80, 24);
    assert_eq!(buf[(0, 1)].symbol(), "▌");
    assert_eq!(buf[(5, 1)].bg, theme::SELECTION_BG);
}

#[test]
fn focus_title_marker() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.handle(alt('1'));
    assert_eq!(app.focus, Focus::Chat);
    let buf = buf_at(&app, 80, 24);
    assert_eq!(buf[(33, 0)].symbol(), "▸");
    assert_eq!(buf[(33, 0)].fg, theme::ACCENT);
    assert_eq!(buf[(35, 0)].fg, theme::TEXT_BRIGHT);
    let x = row_text(&buf, 0).find("chats").unwrap() as u16;
    assert_eq!(buf[(x, 0)].fg, theme::TEXT_DIM);
    assert!(!row_text(&buf, 0).chars().take(32).any(|c| c == '▸'));
    app.handle(prefix());
    app.handle(ch('h'));
    let buf = buf_at(&app, 80, 24);
    assert_eq!(buf[(0, 0)].symbol(), "▸");
    assert_eq!(buf[(0, 0)].fg, theme::ACCENT);
    assert_eq!(buf[(2, 0)].fg, theme::TEXT_BRIGHT);
    assert_ne!(buf[(33, 0)].symbol(), "▸");
    assert_eq!(buf[(35, 0)].fg, theme::TEXT_DIM);
}

#[test]
fn marks_are_accent() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.set_test_script("printf hi; sleep 5");
    app.handle(alt('1'));
    wait_for("output", || screen_text(&app, "a1").contains("hi"));
    let pid = app.procs.chat_pane("a1").unwrap().id;
    app.handle(AppEvent::Pty(ct::pane::Notify::Output(pid)));
    let buf = buf_at(&app, 80, 24);
    let y = find_row(&buf, "alpha chat").unwrap();
    assert_eq!(buf[(31, y)].symbol(), "●");
    assert_eq!(buf[(31, y)].fg, theme::ACCENT);
    app.procs.unread.insert("a2".into());
    let buf = buf_at(&app, 80, 24);
    let y = find_row(&buf, "second chat").unwrap();
    assert_eq!(buf[(31, y)].symbol(), "•");
    assert_eq!(buf[(31, y)].fg, theme::ACCENT);
}

#[test]
fn status_hint() {
    let (_td, folders) = fixture();
    let app = App::for_test(folders);
    let buf = buf_at(&app, 80, 24);
    let text = row_text(&buf, 23);
    let x = text.find("ctrl+space ? keys").expect("hint") as u16;
    assert_eq!(buf[(x, 23)].fg, theme::TEXT_DIM);
    for x in 0..80 {
        assert_eq!(buf[(x, 23)].bg, Color::Reset);
    }
    // the hint follows the configured prefix
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.set_config(ct::config::parse_config("prefix = \"ctrl+b\""));
    assert!(row_text(&buf_at(&app, 80, 24), 23).contains("ctrl+b ? keys"));

    // fork prompt is a warning in the accent color
    let (td, folders) = fixture();
    let root = td.path().join("projects");
    std::fs::create_dir_all(root.join("proj")).unwrap();
    std::fs::write(root.join("proj").join("a1.jsonl"), "{}\n").unwrap();
    let mut app = App::for_test(folders);
    app.set_root(root);
    app.handle(alt('1'));
    let buf = buf_at(&app, 80, 24);
    let x = row_text(&buf, 23).find("chat open elsewhere").unwrap() as u16;
    assert_eq!(buf[(x, 23)].fg, theme::ACCENT);
    for x in 0..80 {
        assert_eq!(buf[(x, 23)].bg, Color::Reset);
    }
}

#[test]
fn running_count_shows_at_right() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.set_test_script("sleep 5");
    app.handle(alt('1'));
    app.handle(alt('2'));
    // a1 is now in the background and running
    let text = row_text(&buf_at(&app, 80, 24), 23);
    assert!(text.trim_end().ends_with("1 running"), "{text}");
}

#[test]
fn long_title_keeps_mark() {
    let td = tempfile::tempdir().unwrap();
    let d = td.path().join("my-app");
    std::fs::create_dir_all(&d).unwrap();
    let long = "x".repeat(200);
    let folders = vec![folder(&d, vec![chat("a1", &d, &long, true)])];
    let mut app = App::for_test(folders);
    app.set_test_script("printf hi; sleep 5");
    app.handle(alt('1'));
    wait_for("output", || screen_text(&app, "a1").contains("hi"));
    let pid = app.procs.chat_pane("a1").unwrap().id;
    app.handle(AppEvent::Pty(ct::pane::Notify::Output(pid)));
    let buf = buf_at(&app, 80, 24);
    let y = find_row(&buf, "xxxx").unwrap();
    let line = row_text(&buf, y);
    assert!(line.chars().take(32).any(|c| c == '…'), "{line}");
    assert_eq!(buf[(31, y)].symbol(), "●");
    assert_eq!(buf[(32, y)].symbol(), "│");
}

#[test]
fn narrow_terminal_no_panic() {
    for (w, h) in [(30u16, 10u16), (12, 5), (3, 2), (1, 1)] {
        for hide in [false, true] {
            let (_td, folders) = fixture();
            let mut app = App::for_test(folders);
            app.set_area(Rect::new(0, 0, w, h));
            if hide {
                app.handle(prefix());
                app.handle(ch('b'));
            }
            app.handle(alt('1'));
            let _ = buf_at(&app, w, h);
        }
    }
}

#[test]
fn click_and_wheel_hit_right_targets() {
    use crossterm::event::{MouseButton, MouseEventKind::Down};
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.set_test_script("seq 1 100; sleep 5");
    let buf = buf_at(&app, 80, 24);
    // third chat, and a chat below the blank row above the second header
    let y3 = find_row(&buf, "third chat").unwrap();
    app.handle(mouse(Down(MouseButton::Left), 5, y3));
    assert_eq!(app.open_chat.as_deref(), Some("a3"));
    app.handle(prefix());
    app.handle(ch('h'));
    let yb = find_row(&buf_at(&app, 80, 24), "beta chat").unwrap();
    app.handle(mouse(Down(MouseButton::Left), 5, yb));
    assert_eq!(app.open_chat.as_deref(), Some("b1"));
    // wheel over the claude pane content scrolls that pane
    app.handle(prefix());
    app.handle(ch('h'));
    let ya = find_row(&buf_at(&app, 80, 24), "alpha chat").unwrap();
    app.handle(mouse(Down(MouseButton::Left), 5, ya));
    assert_eq!(app.open_chat.as_deref(), Some("a1"));
    wait_for("output", || screen_text(&app, "a1").contains("100"));
    let (x, y) = pane_origin(&app);
    wheel(&mut app, true, x + 1, y + 1);
    assert_eq!(offset(&app), 3);
    // wheel on the title line does not
    wheel(&mut app, true, x + 1, y - 1);
    assert_eq!(offset(&app), 3);
}

#[test]
fn sidebar_scroll_counts_blank_lines_and_keeps_context() {
    let td = tempfile::tempdir().unwrap();
    let folders: Vec<Folder> = (0..8)
        .map(|i| {
            let d = td.path().join(format!("f{i}"));
            folder(&d, vec![chat(&format!("c{i}"), &d, &format!("chat {i}"), true)])
        })
        .collect();
    let mut app = App::for_test(folders);
    app.set_area(Rect::new(0, 0, 80, 10));
    let height = 8; // 10 rows - status - title
    let line_of_cursor = |app: &App| {
        let rows = app.sidebar.rows();
        let cur = app.sidebar.cursor;
        let lines = ct::ui::sidebar_lines(&rows, app.sidebar.scroll, cur, height);
        let pos = lines.iter().position(|l| *l == Some(cur)).expect("cursor visible");
        (rows.len(), lines, pos)
    };
    loop {
        let (len, lines, pos) = line_of_cursor(&app);
        if app.sidebar.cursor + 1 < len {
            assert!(pos + 1 < lines.len(), "cursor stuck on the last line at {}: {lines:?}", app.sidebar.cursor);
        }
        if app.sidebar.cursor + 1 >= len {
            break;
        }
        app.handle(ch('j'));
    }
    // one k moves the cursor, not the window
    let before = app.sidebar.scroll;
    app.handle(ch('k'));
    assert_eq!(app.sidebar.scroll, before);
    // drawing and clicking use the same stored offset: the cursor row is drawn
    let buf = buf_at(&app, 80, 10);
    assert!(find_row(&buf, "▌").is_some());
}

#[test]
fn scroll_hints_are_plain_text() {
    let (_td, mut app) = scrolled_app();
    let (x, y) = pane_origin(&app);
    wheel(&mut app, true, x + 1, y + 1);
    let buf = buf_at(&app, 80, 24);
    let text = row_text(&buf, 23);
    let sx = text.find("scroll: 3 lines up").expect("hint") as u16;
    assert_eq!(buf[(sx, 23)].fg, theme::TEXT);
    app.handle(prefix());
    app.handle(ch('['));
    let buf = buf_at(&app, 80, 24);
    let text = row_text(&buf, 23);
    let sx = text.find("scroll: 3 lines up").expect("hint") as u16;
    assert_eq!(buf[(sx, 23)].fg, theme::TEXT);
}

fn open_help(app: &mut App) {
    app.handle(prefix());
    app.handle(ch('?'));
}

#[test]
fn help_opens_and_any_key_closes() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    assert!(!app.help_open());
    open_help(&mut app);
    assert!(app.help_open());
    // 'j' would move the sidebar cursor, so it must be swallowed
    let before = app.sidebar.cursor;
    assert!(app.handle(ch('j')));
    assert!(!app.help_open());
    assert_eq!(app.sidebar.cursor, before);
    // and alt+2 while open only closes it
    open_help(&mut app);
    app.handle(alt('2'));
    assert!(!app.help_open());
    assert_eq!(app.open_chat, None);
}

#[test]
fn help_key_is_not_forwarded_to_pane() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.set_test_script("cat");
    app.handle(alt('1'));
    open_help(&mut app);
    app.handle(ch('x'));
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!screen_text(&app, "a1").contains('x'));
}

#[test]
fn help_renders_sections() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    open_help(&mut app);
    let text = render(&app).join("\n");
    for want in ["SIDEBAR", "EVERYWHERE", "SCROLL MODE", "FORK PROMPT", "then q", "alt+1..9", "esc"] {
        assert!(text.contains(want), "missing {want}: {text}");
    }
    assert!(!text.contains("make the window taller"), "{text}");
    assert!(text.contains(" keys "), "{text}");
}

#[test]
fn click_closes_help() {
    use crossterm::event::{MouseButton, MouseEventKind::Down};
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    open_help(&mut app);
    app.handle(mouse(Down(MouseButton::Left), 4, 3));
    assert!(!app.help_open());
    assert_eq!(app.open_chat, None);
}

#[test]
fn paste_ignored_while_help_open() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    open_help(&mut app);
    app.handle(AppEvent::Input(Event::Paste("hi".into())));
    assert!(app.help_open());
}

#[test]
fn help_tiny_terminal_no_panic() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.focus = Focus::Chat;
    open_help(&mut app);
    for (w, h) in [(30u16, 10u16), (10, 4), (1, 1), (80, 24)] {
        app.handle(AppEvent::Input(Event::Resize(w, h)));
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, &app)).unwrap();
    }
}

#[test]
fn help_short_terminal_says_there_is_more() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.handle(AppEvent::Input(Event::Resize(60, 16)));
    open_help(&mut app);
    let mut t = Terminal::new(TestBackend::new(60, 16)).unwrap();
    t.draw(|f| draw(f, &app)).unwrap();
    let buf = t.backend().buffer();
    let text: String = buf.content().chunks(60).map(|r| r.iter().map(|c| c.symbol()).collect::<String>() + "\n").collect();
    assert!(text.contains("make the window taller to see all keys"), "{text}");
}

fn empty_meta(id: &str, cwd: &Path) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        cwd: cwd.to_path_buf(),
        title: "new chat".into(),
        modified: SystemTime::UNIX_EPOCH + Duration::from_secs(2000),
        empty: true,
    }
}

fn row_ids(app: &App) -> Vec<String> {
    app.sidebar.folders.iter().flat_map(|f| f.chats.iter().map(|c| c.meta.id.clone())).collect()
}

fn title_of(app: &App, id: &str) -> String {
    app.sidebar.folders.iter().flat_map(|f| f.chats.iter()).find(|c| c.meta.id == id).map(|c| c.meta.title.clone()).unwrap_or_default()
}

#[test]
fn empty_history_chat_hidden() {
    let (td, folders) = fixture();
    let mut app = App::for_test(folders);
    let cwd = td.path().join("my-app");
    app.handle(AppEvent::Scanned(vec![empty_meta("e1", &cwd)]));
    assert!(!row_ids(&app).contains(&"e1".to_string()), "{:?}", row_ids(&app));
}

#[test]
fn empty_chat_shown_while_running() {
    let (td, folders) = fixture();
    let cwd = td.path().join("my-app");
    let mut app = App::for_test(folders);
    app.handle(ch('n'));
    let id = app.open_chat.clone().unwrap();
    // history now has the (still empty) session
    app.handle(AppEvent::Scanned(vec![empty_meta(&id, &cwd)]));
    let rows = row_ids(&app);
    assert_eq!(rows.iter().filter(|r| **r == id).count(), 1, "{rows:?}");
    assert_eq!(title_of(&app, &id), "new chat");
    // moving the cursor away keeps it: ct is still running it
    app.handle(ch('j'));
    assert!(row_ids(&app).contains(&id));
}

#[test]
fn pending_new_dropped_when_not_running() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.set_test_script("exit 0");
    app.handle(ch('n'));
    let id = app.open_chat.clone().unwrap();
    wait_for("exit", || !app.procs.is_running(&id));
    // still open in the chat pane, so it stays
    app.handle(AppEvent::Tick);
    assert!(row_ids(&app).contains(&id));
    // opening something else lets it go, without a scan
    app.handle(alt('1'));
    assert!(app.pending_new.is_empty(), "{:?}", app.pending_new);
    assert!(!row_ids(&app).contains(&id));
}

#[test]
fn first_message_keeps_chat_visible() {
    let (td, folders) = fixture();
    let cwd = td.path().join("my-app");
    let mut app = App::for_test(folders);
    app.handle(ch('n'));
    let id = app.open_chat.clone().unwrap();
    let mut m = empty_meta(&id, &cwd);
    m.title = "hello".into();
    m.empty = false;
    app.handle(AppEvent::Scanned(vec![m]));
    let rows = row_ids(&app);
    assert_eq!(rows.iter().filter(|r| **r == id).count(), 1, "{rows:?}");
    assert_eq!(title_of(&app, &id), "hello");
}

#[test]
fn folder_header_name_does_not_shift_when_selected() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    let name_col = |app: &App| {
        let buf = buf_at(app, 80, 24);
        let y = find_row(&buf, "MY-APP").expect("header");
        row_text(&buf, y).chars().take(32).collect::<Vec<_>>().windows(6).position(|w| w.iter().collect::<String>() == "MY-APP").unwrap()
    };
    app.sidebar.cursor = 0;
    let selected = name_col(&app);
    app.sidebar.cursor = 1;
    let unselected = name_col(&app);
    assert_eq!(selected, unselected);
    // the edge marker sits in column 0 only when selected
    app.sidebar.cursor = 0;
    let buf = buf_at(&app, 80, 24);
    let y = find_row(&buf, "MY-APP").unwrap();
    assert_eq!(buf[(0, y)].symbol(), "▌");
}

#[test]
fn status_hint_uses_the_effective_prefix() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    app.set_config(ct::config::parse_config("prefix = \"ctrl+h\""));
    assert!(row_text(&buf_at(&app, 80, 24), 23).contains("ctrl+space ? keys"));
}

#[test]
fn help_lists_page_keys_in_scroll_mode() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    open_help(&mut app);
    let text = render(&app).join("\n");
    assert!(text.contains("pgup/dn"), "{text}");
    assert!(!text.contains("taller"), "{text}");
}

// ---- environment panel ----

fn env_fixture() -> (tempfile::TempDir, std::path::PathBuf, App) {
    let (td, folders) = fixture();
    let root = td.path().join("projects");
    std::fs::create_dir_all(root.join("proj")).unwrap();
    let mut app = App::for_test(folders);
    app.set_root(root.clone());
    app.set_area(Rect::new(0, 0, 120, 30));
    (td, root.join("proj"), app)
}

fn all_text(app: &App, w: u16, h: u16) -> String {
    let buf = buf_at(app, w, h);
    (0..h).map(|y| row_text(&buf, y)).collect::<Vec<_>>().join("\n")
}

fn chat_cols(app: &App, id: &str) -> u16 {
    app.procs.chat_pane(id).unwrap().screen_lock().screen().size().1.get()
}

#[test]
fn prefix_i_toggles_env() {
    let (_td, _p, mut app) = env_fixture();
    assert!(!app.env_visible());
    app.handle(prefix());
    app.handle(ch('i'));
    assert!(app.env_visible());
    app.handle(prefix());
    app.handle(ch('i'));
    assert!(!app.env_visible());
}

#[test]
fn env_hidden_when_narrow() {
    let (_td, _p, mut app) = env_fixture();
    app.handle(prefix());
    app.handle(ch('i'));
    app.set_area(Rect::new(0, 0, 99, 30));
    assert!(!app.env_visible());
    assert!(!all_text(&app, 99, 30).contains("environment"));
    app.set_area(Rect::new(0, 0, 100, 30));
    assert!(app.env_visible());
    assert!(all_text(&app, 100, 30).contains("environment"));
    // zoom hides it too, and the toggle is kept
    app.handle(prefix());
    app.handle(ch('z'));
    assert!(!app.env_visible());
    app.handle(prefix());
    app.handle(ch('z'));
    assert!(app.env_visible());
}

#[test]
fn env_panel_rows() {
    let (td, proj, mut app) = env_fixture();
    app.set_test_script("sleep 5");
    app.env.host = "mybox".into();
    app.handle(alt('1'));
    let session = proj.join("a1.jsonl");
    std::fs::write(
        &session,
        concat!(
            r#"{"type":"assistant","message":{"model":"claude-opus-4","usage":{"input_tokens":50000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0},"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/tmp/x/report.md"}},{"type":"tool_use","name":"Edit","input":{"file_path":"/tmp/x/main.rs"}}]}}"#,
            "\n"
        ),
    )
    .unwrap();
    app.handle(prefix());
    app.handle(ch('i'));
    app.handle(AppEvent::Tick);
    app.handle(AppEvent::GitBranch { chat: "a1".into(), branch: "feature/x".into() });
    let text = all_text(&app, 120, 30);
    for want in ["environment", "this computer · mybox", "my-app · ", "feature/x", "remote control", "off", "context", "25%", "connectors", "checking…", "OUTPUTS", "report.md", "main.rs"] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    let _ = td;
}

#[test]
fn env_toggle_resizes_panes() {
    let (_td, _p, mut app) = env_fixture();
    app.handle(alt('1'));
    let before = chat_cols(&app, "a1");
    app.handle(prefix());
    app.handle(ch('i'));
    assert_eq!(chat_cols(&app, "a1"), before - 35);
    app.handle(prefix());
    app.handle(ch('i'));
    assert_eq!(chat_cols(&app, "a1"), before);
    // narrowing the terminal under the panel also gives the width back
    app.handle(prefix());
    app.handle(ch('i'));
    app.handle(AppEvent::Input(Event::Resize(99, 30)));
    assert_eq!(chat_cols(&app, "a1"), 99 - 33);
}

#[test]
fn stale_branch_ignored() {
    let (_td, _p, mut app) = env_fixture();
    app.handle(alt('1'));
    app.handle(prefix());
    app.handle(ch('i'));
    app.handle(AppEvent::GitBranch { chat: "b1".into(), branch: "other-branch".into() });
    assert!(!all_text(&app, 120, 30).contains("other-branch"));
    app.handle(AppEvent::GitBranch { chat: "a1".into(), branch: "mine".into() });
    assert!(all_text(&app, 120, 30).contains("mine"));
    // switching chats clears the old chat's branch, and a late result for it is dropped
    app.handle(alt('3'));
    assert_eq!(app.open_chat.as_deref(), Some("b1"));
    assert!(!all_text(&app, 120, 30).contains("mine"));
    app.handle(AppEvent::GitBranch { chat: "a1".into(), branch: "late".into() });
    assert!(!all_text(&app, 120, 30).contains("late"));
}

#[test]
fn env_state_persists() {
    let td = tempfile::tempdir().unwrap();
    let p = td.path().join("state.toml");
    assert!(!ct::state::load_env_panel_from(&p));
    ct::state::save_env_panel_to(&p, true);
    assert!(ct::state::load_env_panel_from(&p));
    ct::state::save_last_to(&p, "abc");
    assert!(ct::state::load_env_panel_from(&p));
    ct::state::save_env_panel_to(&p, false);
    assert!(!ct::state::load_env_panel_from(&p));
}

#[test]
fn help_lists_env_key() {
    let (_td, folders) = fixture();
    let mut app = App::for_test(folders);
    open_help(&mut app);
    let text = render(&app).join("\n");
    assert!(text.contains("prefix, then i"), "{text}");
    assert!(text.contains("environment panel"), "{text}");
    assert!(!text.contains("taller"), "{text}");
}

#[test]
fn hidden_panel_does_no_session_lookups() {
    let (_td, _p, mut app) = env_fixture();
    app.handle(alt('1'));
    for _ in 0..3 {
        app.handle(AppEvent::Tick);
        app.handle(ch('x'));
    }
    assert_eq!(app.env.lookups, 0);
    app.handle(prefix());
    app.handle(ch('i'));
    // not on arbitrary events, only on ticks
    app.handle(ch('x'));
    assert_eq!(app.env.lookups, 0);
    app.handle(AppEvent::Tick);
    assert_eq!(app.env.lookups, 1);
}

#[test]
fn session_file_is_found_in_the_encoded_project_dir() {
    let (td, folders) = fixture();
    let cwd = td.path().join("my-app");
    let root = td.path().join("projects");
    let enc = ct::env_panel::encode_project_dir(&cwd);
    assert!(enc.starts_with('-') && !enc.contains('/') && !enc.contains('.') && !enc.contains('_'));
    std::fs::create_dir_all(root.join(&enc)).unwrap();
    std::fs::write(root.join(&enc).join("a1.jsonl"), "").unwrap();
    // an old file, so opening the chat doesn't look like another claude is using it
    let old = SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options().write(true).open(root.join(&enc).join("a1.jsonl")).unwrap().set_modified(old).unwrap();
    let mut app = App::for_test(folders);
    app.set_root(root.clone());
    app.set_area(Rect::new(0, 0, 120, 30));
    app.handle(prefix());
    app.handle(ch('i'));
    app.handle(alt('1'));
    // found at once on the switch, with no scan
    assert!(app.env.tracker.is_some());
    assert_eq!(ct::env_panel::locate_session(&root, &cwd, "nope", false), None);
}

#[test]
fn env_is_34_wide_at_100_columns_with_sidebar() {
    let (_td, _p, mut app) = env_fixture();
    app.handle(prefix());
    app.handle(ch('i'));
    let l = layout(Rect::new(0, 0, 100, 30), true, false, Focus::Sidebar, app.env_visible());
    assert_eq!(l.env.unwrap().width, 34);
}

#[test]
fn new_chat_gets_its_branch_after_a_switch_with_a_read_in_flight() {
    let mut p = ct::env_panel::EnvPanel::new();
    p.switch(Some("a".into()));
    let _stuck = p.branch_started();
    assert!(!p.branch_due());
    p.switch(Some("b".into()));
    assert!(p.branch_due());
}

#[test]
fn output_polls_are_rate_limited_but_ticks_are_not() {
    let (_td, proj, mut app) = env_fixture();
    app.handle(alt('1'));
    app.handle(prefix());
    app.handle(ch('i'));
    let session = proj.join("a1.jsonl");
    let line = |n: u64| format!("{{\"type\":\"assistant\",\"message\":{{\"model\":\"m\",\"usage\":{{\"input_tokens\":{n}}}}}}}\n");
    std::fs::write(&session, line(50000)).unwrap();
    app.handle(AppEvent::Tick);
    assert_eq!(app.env.context_pct(), Some(25));
    std::fs::write(&session, format!("{}{}", line(50000), line(100000))).unwrap();
    let pid = app.procs.chat_pane("a1").unwrap().id;
    app.handle(AppEvent::Pty(ct::pane::Notify::Output(pid)));
    assert_eq!(app.env.context_pct(), Some(25));
    app.handle(AppEvent::Tick);
    assert_eq!(app.env.context_pct(), Some(50));
}

#[test]
fn long_connector_names_keep_their_status_visible() {
    let (_td, _p, mut app) = env_fixture();
    app.handle(alt('1'));
    app.handle(prefix());
    app.handle(ch('i'));
    app.env.connectors = ct::env::Connectors::Ready(vec![ct::env::Connector {
        name: "a-very-long-connector-name-that-overflows".into(),
        status: ct::env::McpStatus::NeedsAuth,
    }]);
    app.env.expanded = true;
    let text = all_text(&app, 120, 30);
    assert!(text.contains("needs auth"), "{text}");
    assert!(text.contains("a-very-long"), "{text}");
}

// ---- environment panel focus and actions ----

fn env_on(app: &mut App) {
    app.handle(prefix());
    app.handle(ch('i'));
}

fn focus_env(app: &mut App) {
    app.handle(prefix());
    app.handle(ch('l'));
}

fn log_lines(p: &Path) -> Vec<String> {
    std::fs::read_to_string(p).map(|t| t.lines().map(String::from).collect()).unwrap_or_default()
}

fn pane_id(app: &App, id: &str) -> ct::pane::PaneId {
    app.procs.chat_pane(id).unwrap().id
}

#[test]
fn focus_moves_into_env() {
    let (_td, _p, mut app) = env_fixture();
    app.handle(alt('1'));
    // hidden: prefix l does nothing
    focus_env(&mut app);
    assert_eq!(app.focus, Focus::Chat);
    env_on(&mut app);
    focus_env(&mut app);
    assert_eq!(app.focus, Focus::Env);
    assert!(all_text(&app, 120, 30).contains("▸ environment"));
    app.handle(prefix());
    app.handle(ch('h'));
    assert_eq!(app.focus, Focus::Chat);
    // from the shell too
    app.handle(prefix());
    app.handle(ch('j'));
    focus_env(&mut app);
    assert_eq!(app.focus, Focus::Env);
    // keys in the panel don't reach the pane, and hiding the panel gives focus back
    env_on(&mut app);
    assert_eq!(app.focus, Focus::Chat);
}

#[test]
fn click_focuses_env() {
    use crossterm::event::{MouseButton, MouseEventKind::Down};
    let (_td, _p, mut app) = env_fixture();
    app.handle(alt('1'));
    env_on(&mut app);
    app.handle(mouse(Down(MouseButton::Left), 110, 5));
    assert_eq!(app.focus, Focus::Env);
}

#[test]
fn jk_moves_env_cursor_over_actionable_rows() {
    let (_td, proj, mut app) = env_fixture();
    app.handle(alt('1'));
    std::fs::write(
        proj.join("a1.jsonl"),
        concat!(
            r#"{"type":"assistant","message":{"model":"m","usage":{"input_tokens":1},"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/tmp/x/one.md"}},{"type":"tool_use","name":"Write","input":{"file_path":"/tmp/x/two.md"}}]}}"#,
            "\n"
        ),
    )
    .unwrap();
    env_on(&mut app);
    app.handle(AppEvent::Tick);
    focus_env(&mut app);
    assert_eq!(app.env.cursor, 0);
    app.handle(ch('k'));
    assert_eq!(app.env.cursor, 0);
    for want in [1, 2, 3, 3] {
        app.handle(ch('j'));
        assert_eq!(app.env.cursor, want);
    }
    app.handle(key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.env.cursor, 2);
    // the selected row carries the edge marker
    let text = all_text(&app, 120, 30);
    assert!(text.lines().any(|l| l.contains("▌") && l.contains("one.md") || l.contains("▌") && l.contains("two.md")), "{text}");
}

#[test]
fn enter_on_connectors_expands() {
    use ct::env::{Connector, Connectors, McpStatus};
    let (_td, _p, mut app) = env_fixture();
    app.handle(alt('1'));
    env_on(&mut app);
    app.handle(AppEvent::Connectors(Connectors::Ready(vec![
        Connector { name: "github".into(), status: McpStatus::Connected },
        Connector { name: "slack".into(), status: McpStatus::NeedsAuth },
        Connector { name: "db".into(), status: McpStatus::Failed },
    ])));
    focus_env(&mut app);
    app.handle(ch('j'));
    assert!(!all_text(&app, 120, 30).contains("needs auth"));
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    let text = all_text(&app, 120, 30);
    for want in ["github", "connected", "slack", "needs auth", "db", "failed"] {
        assert!(text.contains(want), "missing {want}\n{text}");
    }
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!all_text(&app, 120, 30).contains("needs auth"));
}

fn output_fixture(files: &[&Path]) -> (tempfile::TempDir, App) {
    let (td, proj, mut app) = env_fixture();
    app.handle(alt('1'));
    let blocks: Vec<String> = files
        .iter()
        .map(|f| format!(r#"{{"type":"tool_use","name":"Write","input":{{"file_path":"{}"}}}}"#, f.display()))
        .collect();
    std::fs::write(
        proj.join("a1.jsonl"),
        format!("{{\"type\":\"assistant\",\"message\":{{\"model\":\"m\",\"usage\":{{\"input_tokens\":1}},\"content\":[{}]}}}}\n", blocks.join(",")),
    )
    .unwrap();
    env_on(&mut app);
    app.handle(AppEvent::Tick);
    focus_env(&mut app);
    (td, app)
}

#[test]
fn enter_on_output_opens() {
    let td = tempfile::tempdir().unwrap();
    let file = td.path().join("report.md");
    std::fs::write(&file, "x").unwrap();
    let (_t, mut app) = output_fixture(&[&file]);
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::<PathBuf>::new()));
    let o = opened.clone();
    app.set_opener(move |p| o.borrow_mut().push(p.to_path_buf()));
    app.handle(ch('j'));
    app.handle(ch('j'));
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(*opened.borrow(), vec![file]);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn missing_output_shows_message() {
    let td = tempfile::tempdir().unwrap();
    let file = td.path().join("gone.md");
    let (_t, mut app) = output_fixture(&[&file]);
    let opened = std::rc::Rc::new(std::cell::RefCell::new(0));
    let o = opened.clone();
    app.set_opener(move |_| *o.borrow_mut() += 1);
    app.handle(ch('j'));
    app.handle(ch('j'));
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.modal, Modal::Message("file no longer exists".into()));
    assert_eq!(*opened.borrow(), 0);
}

fn remote_fixture(script_body: &str) -> (tempfile::TempDir, PathBuf, App) {
    let (td, _p, mut app) = env_fixture();
    let log = td.path().join("argv.log");
    app.set_test_script(&format!("echo \"$@\" >> {}; {script_body}", log.display()));
    app.handle(alt('1'));
    env_on(&mut app);
    focus_env(&mut app);
    (td, log, app)
}

#[test]
fn remote_toggle_restarts_with_flag() {
    let (_td, log, mut app) = remote_fixture("sleep 5");
    wait_for("first start", || log_lines(&log).len() == 1);
    assert!(!log_lines(&log)[0].contains("--remote-control"));
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Env);
    wait_for("restart", || log_lines(&log).len() == 2);
    assert!(log_lines(&log)[1].contains("--remote-control"), "{:?}", log_lines(&log));
    assert!(all_text(&app, 120, 30).contains("remote control"));
    assert!(app.remote.contains("a1"));
    assert_eq!(app.procs.own_chat_pids().len(), 1);
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    wait_for("second restart", || log_lines(&log).len() == 3);
    assert!(!log_lines(&log)[2].contains("--remote-control"));
    assert!(!app.remote.contains("a1"));
    assert_eq!(app.procs.own_chat_pids().len(), 1);
    assert_eq!(app.focus, Focus::Env);
    assert_eq!(log_lines(&log).len(), 3);
}

#[test]
fn remote_toggle_confirms_when_working() {
    let (_td, log, mut app) = remote_fixture("printf hi; sleep 5");
    wait_for("output", || screen_text(&app, "a1").contains("hi"));
    let id = pane_id(&app, "a1");
    app.handle(AppEvent::Pty(ct::pane::Notify::Output(id)));
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(app.modal, Modal::RemoteConfirm { .. }));
    assert!(render(&app).join("\n").contains("restart claude to toggle remote control? y / esc"));
    // esc cancels, nothing restarts
    app.handle(key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.modal, Modal::None);
    assert_eq!(pane_id(&app, "a1"), id);
    assert!(!app.remote.contains("a1"));
    // y confirms
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    app.handle(ch('y'));
    assert_eq!(app.modal, Modal::None);
    assert_ne!(pane_id(&app, "a1"), id);
    assert!(app.remote.contains("a1"));
    assert_eq!(app.focus, Focus::Env);
    wait_for("restart", || log_lines(&log).len() == 2);
}

#[test]
fn remote_toggle_on_exited_chat() {
    let (_td, log, mut app) = remote_fixture("exit 0");
    wait_for("exit", || app.procs.chat_pane("a1").unwrap().exited.load(std::sync::atomic::Ordering::SeqCst));
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.modal, Modal::None);
    wait_for("restart", || log_lines(&log).len() == 2);
    assert!(log_lines(&log)[1].contains("--remote-control"));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(log_lines(&log).len(), 2);
    assert_eq!(app.focus, Focus::Env);
}

#[test]
fn remote_flag_survives_enter_restart_in_chat() {
    let (_td, log, mut app) = remote_fixture("exit 0");
    wait_for("exit", || app.procs.chat_pane("a1").unwrap().exited.load(std::sync::atomic::Ordering::SeqCst));
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    wait_for("restart", || log_lines(&log).len() == 2);
    wait_for("exit again", || app.procs.chat_pane("a1").unwrap().exited.load(std::sync::atomic::Ordering::SeqCst));
    app.handle(prefix());
    app.handle(ch('h'));
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    wait_for("plain restart", || log_lines(&log).len() == 3);
    assert!(log_lines(&log)[2].contains("--remote-control"), "{:?}", log_lines(&log));
}

fn pid_alive(pid: &str) -> bool {
    std::process::Command::new("kill").args(["-0", pid]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

#[test]
fn remote_toggle_kills_old_claude_before_new_starts() {
    let (td, _p, mut app) = env_fixture();
    let log = td.path().join("argv.log");
    let pids = td.path().join("pids.log");
    // ignores HUP, so only the hard kill after the grace period ends it
    app.set_test_script(&format!("trap '' HUP; echo $$ >> {}; echo \"$@\" >> {}; while :; do sleep 1; done", pids.display(), log.display()));
    app.handle(alt('1'));
    env_on(&mut app);
    focus_env(&mut app);
    wait_for("first start", || log_lines(&pids).len() == 1);
    let old = log_lines(&pids)[0].clone();
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    // by the time the toggle returns the old process is gone
    assert!(!pid_alive(&old), "old claude {old} still alive");
    wait_for("restart", || log_lines(&pids).len() == 2);
    assert_ne!(log_lines(&pids)[1], old);
    assert_eq!(app.procs.own_chat_pids().len(), 1);
}

#[test]
fn remote_toggle_keeps_unwritten_fork() {
    let (td, _p, mut app) = env_fixture();
    let log = td.path().join("argv.log");
    app.set_test_script(&format!("echo \"$@\" >> {}; sleep 5", log.display()));
    app.modal = Modal::ForkPrompt { id: "a1".into() };
    app.handle(ch('f'));
    let fork = app.open_chat.clone().unwrap();
    assert_ne!(fork, "a1");
    env_on(&mut app);
    focus_env(&mut app);
    wait_for("fork start", || log_lines(&log).len() == 1);
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    wait_for("restart", || log_lines(&log).len() == 2);
    let line = &log_lines(&log)[1];
    assert!(line.contains("--resume a1 --fork-session --session-id"), "{line}");
    assert!(line.contains(&fork) && line.ends_with("--remote-control"), "{line}");
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    wait_for("toggle off", || log_lines(&log).len() == 3);
    let line = &log_lines(&log)[2];
    assert!(line.contains("--fork-session") && !line.contains("--remote-control"), "{line}");
}

#[test]
fn output_selection_stays_on_its_file() {
    let td = tempfile::tempdir().unwrap();
    let (one, two, three) = (td.path().join("one.md"), td.path().join("two.md"), td.path().join("three.md"));
    for f in [&one, &two, &three] {
        std::fs::write(f, "x").unwrap();
    }
    let (t, mut app) = output_fixture(&[&one, &two]);
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::<PathBuf>::new()));
    let o = opened.clone();
    app.set_opener(move |p| o.borrow_mut().push(p.to_path_buf()));
    app.handle(ch('j'));
    app.handle(ch('j'));
    let want = app.env.outputs()[app.env.cursor - 2].clone();
    // a new output arrives and takes the top of the list
    let session = std::fs::read_dir(t.path().join("projects/proj")).unwrap().next().unwrap().unwrap().path();
    let mut text = std::fs::read_to_string(&session).unwrap();
    text.push_str(&format!(
        "{{\"type\":\"assistant\",\"message\":{{\"model\":\"m\",\"usage\":{{\"input_tokens\":1}},\"content\":[{{\"type\":\"tool_use\",\"name\":\"Write\",\"input\":{{\"file_path\":\"{}\"}}}}]}}}}\n",
        three.display()
    ));
    std::fs::write(&session, text).unwrap();
    app.handle(AppEvent::Tick);
    assert_eq!(app.env.outputs()[0], three);
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(*opened.borrow(), vec![want]);
}

#[test]
fn remote_toggle_spawn_failure_keeps_preference() {
    let (td, log, mut app) = remote_fixture("sleep 5");
    wait_for("start", || log_lines(&log).len() == 1);
    let _ = td;
    app.set_fail_spawn(true);
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(&app.modal, Modal::Message(m) if m.contains("could not start claude")), "{:?}", app.modal);
    assert!(!app.remote.contains("a1"));
    assert_eq!(app.focus, Focus::Env);
    assert_eq!(app.procs.own_chat_pids().len(), 0);
}

#[test]
fn remote_flag_survives_enter_restart_with_history() {
    let (td, proj, mut app) = env_fixture();
    let f = std::fs::File::create(proj.join("a1.jsonl")).unwrap();
    // old enough not to look like another claude is using it
    f.set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
    let log = td.path().join("argv.log");
    app.set_test_script(&format!("echo \"$@\" >> {}; exit 0", log.display()));
    app.handle(alt('1'));
    env_on(&mut app);
    focus_env(&mut app);
    wait_for("start", || log_lines(&log).len() == 1);
    assert_eq!(log_lines(&log)[0], "--resume a1");
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    wait_for("toggle", || log_lines(&log).len() == 2);
    assert_eq!(log_lines(&log)[1], "--resume a1 --remote-control");
    wait_for("exit", || app.procs.chat_pane("a1").unwrap().exited.load(std::sync::atomic::Ordering::SeqCst));
    app.handle(prefix());
    app.handle(ch('h'));
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    wait_for("plain restart", || log_lines(&log).len() == 3);
    assert_eq!(log_lines(&log)[2], "--resume a1 --remote-control");
}

#[test]
fn env_rule_and_header_keep_their_colors() {
    let td = tempfile::tempdir().unwrap();
    let file = td.path().join("report.md");
    std::fs::write(&file, "x").unwrap();
    let (_t, app) = output_fixture(&[&file]);
    let buf = buf_at(&app, 120, 30);
    let y = (0..30).find(|&y| row_text(&buf, y).contains('─')).expect("a rule");
    let x = (0..120).find(|&x| buf[(x, y)].symbol() == "─").unwrap();
    assert_eq!(buf[(x, y)].fg, theme::DIVIDER);
    let y = (0..30).find(|&y| row_text(&buf, y).contains("OUTPUTS")).expect("header");
    let x = (0..120).find(|&x| row_text(&buf, y).chars().nth(x as usize) == Some('O')).unwrap();
    assert_eq!(buf[(x, y)].fg, theme::TEXT_DIM);
}

#[test]
fn env_panel_scrolls_to_last_output_and_opens_it() {
    let td = tempfile::tempdir().unwrap();
    let files: Vec<PathBuf> = (0..40)
        .map(|i| {
            let f = td.path().join(format!("out{i:02}.md"));
            std::fs::write(&f, "x").unwrap();
            f
        })
        .collect();
    let refs: Vec<&Path> = files.iter().map(|f| f.as_path()).collect();
    let (_t, mut app) = output_fixture(&refs);
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::<PathBuf>::new()));
    let o = opened.clone();
    app.set_opener(move |p| o.borrow_mut().push(p.to_path_buf()));
    for _ in 0..41 {
        app.handle(ch('j'));
    }
    let last = app.env.outputs().last().unwrap().clone();
    let name = last.file_name().unwrap().to_string_lossy().into_owned();
    let text = all_text(&app, 120, 30);
    assert!(text.contains(&name), "{name} not visible in\n{text}");
    app.handle(key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(*opened.borrow(), vec![last]);
    // and back up to the top: the first rows show again
    for _ in 0..45 {
        app.handle(ch('k'));
    }
    assert!(all_text(&app, 120, 30).contains("remote control"));
}

#[test]
fn h_and_esc_leave_the_env_panel() {
    for k in [ch('h'), key(KeyCode::Esc, KeyModifiers::NONE)] {
        let (_t, mut app) = output_fixture(&[]);
        assert_eq!(app.focus, ct::app::Focus::Env);
        app.handle(k);
        assert_eq!(app.focus, ct::app::Focus::Chat);
    }
}
