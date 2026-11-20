use crossterm::cursor::Show;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement,
};
use ct::app::App;
use ct::config::load_config;
use ct::event::AppEvent;
use ct::history::{ScanCache, projects_root};
use ct::state::{load_closed_at, load_last, save_closed_now};
use ct::tree::group;
use ct::ui::draw;
use ct::watch::{spawn_scanner, spawn_watcher};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use std::io::stdout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;

static ENHANCED: AtomicBool = AtomicBool::new(false);
static MOUSE: AtomicBool = AtomicBool::new(false);

fn restore() {
    if ENHANCED.swap(false, Ordering::SeqCst) {
        let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    }
    if MOUSE.swap(false, Ordering::SeqCst) {
        let _ = execute!(stdout(), DisableMouseCapture);
    }
    let _ = execute!(stdout(), DisableBracketedPaste, Show, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--dump") {
        let all = ScanCache::default().scan(&projects_root());
        println!("{} sessions", all.len());
        for s in all.iter().take(5) {
            println!("{}  {}  {}", s.id, s.cwd.display(), s.title);
        }
        return Ok(());
    }

    let cfg = load_config();
    let app_mouse = cfg.mouse;
    // the first scan is synchronous; later ones run on a worker with this cache
    let mut cache = ScanCache::default();
    let folders = group(cache.scan(&projects_root()));
    let (tx, rx) = channel::<AppEvent>();
    let mut app = App::new(cfg, folders, tx.clone());
    if let Some(c) = load_closed_at() {
        app.set_closed_at(c);
    }
    app.set_env_panel(ct::state::load_env_panel());
    app.env.host = ct::env::hostname_short();

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        save_closed_now();
        ct::env::kill_running_mcp();
        prev(info);
        // a panic on any thread ends the app; children die when their ptys close
        std::process::exit(101);
    }));

    enable_raw_mode()?;
    if let Err(e) = execute!(stdout(), EnterAlternateScreen, EnableBracketedPaste) {
        restore();
        return Err(e.into());
    }
    if app_mouse && execute!(stdout(), EnableMouseCapture).is_ok() {
        MOUSE.store(true, Ordering::SeqCst);
    }
    if supports_keyboard_enhancement().unwrap_or(false) {
        let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;
        if execute!(stdout(), PushKeyboardEnhancementFlags(flags)).is_ok() {
            ENHANCED.store(true, Ordering::SeqCst);
        }
    }
    let result = run(&mut app, &mut cache, &tx, rx);
    restore();
    // hang up every child's process group, kill what ignores it, and only then exit
    app.shutdown();
    ct::env::kill_running_mcp();
    drop(app);
    save_closed_now();
    result
}

fn run(app: &mut App, cache: &mut ScanCache, tx: &std::sync::mpsc::Sender<AppEvent>, rx: std::sync::mpsc::Receiver<AppEvent>) -> anyhow::Result<()> {
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let size = terminal.size()?;
    app.set_area(Rect::new(0, 0, size.width, size.height));

    // input thread
    let itx = tx.clone();
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if !ct::event::wanted(&ev) {
                continue;
            }
            if itx.send(AppEvent::Input(ev)).is_err() {
                break;
            }
        }
    });
    // a signal from outside leaves through the normal quit path, so the terminal is restored
    if let Ok(mut signals) = signal_hook::iterator::Signals::new([signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT, signal_hook::consts::SIGHUP]) {
        let stx = tx.clone();
        std::thread::spawn(move || {
            if signals.forever().next().is_some() {
                let _ = stx.send(AppEvent::Quit);
            }
        });
    }
    // a beat so the working/unread marks update when output stops
    let ttx = tx.clone();
    std::thread::spawn(move || {
        while ttx.send(AppEvent::Tick).is_ok() {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });
    ct::env::spawn_mcp_worker(ct::env::real_mcp_runner(), tx.clone(), std::time::Duration::from_secs(300));
    // keep alive until we return; a missing projects dir just means no live updates
    app.set_scanner(spawn_scanner(projects_root(), std::mem::take(cache), tx.clone()));
    let _watcher = spawn_watcher(&projects_root(), tx.clone()).ok();

    if let Some(id) = load_last()
        && app.sidebar.select_by_id(&id) {
            app.open_selected();
        }

    loop {
        terminal.draw(|f| draw(f, app))?;
        if !ct::frame::collect(app, &rx) {
            break;
        }
    }
    Ok(())
}
