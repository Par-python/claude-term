use ct::event::AppEvent;
use ct::pane::{Notify, Pane, PaneSpec};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

fn spec(program: &str, args: &[&str]) -> PaneSpec {
    PaneSpec {
        program: program.into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        cwd: std::env::temp_dir(),
    }
}

fn wait_exited(rx: &Receiver<AppEvent>) -> bool {
    wait_exited_for(rx, 2000)
}

fn wait_exited_for(rx: &Receiver<AppEvent>, ms: u64) -> bool {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        if let Ok(AppEvent::Pty(Notify::Exited(_))) = rx.recv_timeout(Duration::from_millis(20)) {
            return true;
        }
    }
    false
}

fn wait_screen(pane: &Pane, rx: &Receiver<AppEvent>, needle: &str) -> bool {
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end {
        if pane.screen_lock().screen().contents().contains(needle) {
            return true;
        }
        let _ = rx.recv_timeout(Duration::from_millis(20));
    }
    false
}

#[test]
fn spawn_echo_shows_output() {
    let (tx, rx) = channel();
    let pane = Pane::spawn(1, spec("sh", &["-c", "echo hello-ct"]), 24, 80, tx).unwrap();
    assert!(wait_exited(&rx));
    assert!(pane.screen_lock().screen().contents().contains("hello-ct"));
}

#[test]
fn write_reaches_child() {
    let (tx, rx) = channel();
    let mut pane = Pane::spawn(2, spec("cat", &[]), 24, 80, tx).unwrap();
    pane.write(b"ping\r");
    assert!(wait_screen(&pane, &rx, "ping"));
}

#[test]
fn resize_updates_parser() {
    let (tx, _rx) = channel();
    let mut pane = Pane::spawn(3, spec("cat", &[]), 24, 80, tx).unwrap();
    pane.resize(10, 40);
    let (r, c) = pane.screen_lock().screen().size();
    assert_eq!((r.get(), c.get()), (10, 40));
}

#[test]
fn exit_sets_flag_and_restart_works() {
    let (tx, rx) = channel();
    let mut pane = Pane::spawn(4, spec("sh", &["-c", "exit 0"]), 24, 80, tx.clone()).unwrap();
    assert!(wait_exited(&rx));
    assert!(pane.exited.load(Ordering::SeqCst));
    pane.spec = spec("cat", &[]);
    pane.restart(tx).unwrap();
    assert!(!pane.exited.load(Ordering::SeqCst));
    pane.kill();
    assert!(wait_exited(&rx));
    assert!(pane.exited.load(Ordering::SeqCst));
}

#[test]
fn drop_returns_quickly() {
    let (tx, _rx) = channel();
    let pane = Pane::spawn(7, spec("sleep", &["5"]), 24, 80, tx).unwrap();
    let t = Instant::now();
    drop(pane);
    assert!(t.elapsed() < Duration::from_millis(50), "{:?}", t.elapsed());
}

#[test]
fn zero_size_is_clamped() {
    let (tx, _rx) = channel();
    let mut pane = Pane::spawn(8, spec("cat", &[]), 0, 0, tx).unwrap();
    let (r, c) = pane.screen_lock().screen().size();
    assert_eq!((r.get(), c.get()), (1, 1));
    pane.resize(0, 0);
    let (r, c) = pane.screen_lock().screen().size();
    assert_eq!((r.get(), c.get()), (1, 1));
}

#[test]
fn late_eof_of_old_run_does_not_mark_new_run_exited() {
    let (tx, rx) = channel();
    let mut pane = Pane::spawn(5, spec("cat", &[]), 24, 80, tx.clone()).unwrap();
    pane.restart(tx).unwrap(); // old cat is killed; its EOF arrives late
    assert!(!wait_exited_for(&rx, 300));
    assert!(!pane.exited.load(Ordering::SeqCst));
}

#[test]
fn child_gets_term_env() {
    let (tx, rx) = channel();
    let pane = Pane::spawn(6, spec("sh", &["-c", "echo T=$TERM"]), 24, 80, tx).unwrap();
    assert!(wait_exited(&rx));
    assert!(pane.screen_lock().screen().contents().contains("T=xterm-256color"));
}

/// Runs a shell script in a pane and returns the screen once `needle` shows up.
fn run_script(script: &str, needle: &str) -> String {
    let (tx, rx) = channel();
    let pane = Pane::spawn(20, spec("sh", &["-c", script]), 24, 80, tx).unwrap();
    assert!(wait_screen(&pane, &rx, needle), "never saw {needle:?}; screen: {:?}", pane.screen_lock().screen().contents());
    let text = pane.screen_lock().screen().contents();
    drop(pane);
    text
}

#[test]
fn cursor_position_query_is_answered() {
    // raw mode so the reply isn't echoed; cat -v makes the escape visible
    let s = run_script("stty raw -echo; printf 'ab\\033[6n'; head -c 6 | cat -v", "^[[1;3R");
    assert!(s.contains("^[[1;3R"), "{s}");
}

#[test]
fn status_and_device_attribute_queries_are_answered() {
    let s = run_script("stty raw -echo; printf '\\033[5n'; head -c 4 | cat -v; printf '\\033[c'; head -c 9 | cat -v", "^[[?62;22c");
    assert!(s.contains("^[[0n"), "{s}");
    assert!(s.contains("^[[?62;22c"), "{s}");
}

#[test]
fn osc_color_queries_are_answered() {
    let s = run_script(
        "stty raw -echo; printf '\\033]11;?\\033\\\\'; head -c 25 | cat -v; printf '\\033]10;?\\007'; head -c 25 | cat -v",
        "^[]10;rgb:ffff/ffff/ffff^[\\",
    );
    assert!(s.contains("^[]11;rgb:0000/0000/0000^[\\"), "{s}");
}

#[test]
fn synchronized_output_block_is_tracked_and_capped() {
    let (tx, rx) = channel();
    let mut pane = Pane::spawn(21, spec("sh", &["-c", "stty raw -echo; printf '\\033[?2026hAAA'; head -c 1 >/dev/null; printf '\\033[?2026lBBB'; sleep 5"]), 24, 80, tx).unwrap();
    assert!(wait_screen(&pane, &rx, "AAA"));
    assert!(pane.in_sync_block());
    // a lost end marker can't hold drawing forever
    assert!(!pane.in_sync_block_at(Instant::now() + Duration::from_millis(150)));
    pane.write(b"x");
    assert!(wait_screen(&pane, &rx, "BBB"));
    assert!(!pane.in_sync_block());
}

#[test]
fn dropping_a_pane_ends_its_background_grandchildren_too() {
    let (tx, _rx) = channel();
    let marker = format!("sleep 5{}.5", std::process::id());
    let pane = Pane::spawn(30, spec("sh", &["-c", &format!("{marker} & {marker}")]), 24, 80, tx).unwrap();
    let running = |m: &str| {
        let out = std::process::Command::new("pgrep").args(["-f", m]).output().unwrap();
        !String::from_utf8_lossy(&out.stdout).trim().is_empty()
    };
    let end = Instant::now() + Duration::from_secs(3);
    while !running(&marker) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(running(&marker));
    drop(pane);
    let end = Instant::now() + Duration::from_secs(2);
    while running(&marker) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!running(&marker), "grandchildren survived the drop");
}

#[test]
fn child_env_has_truecolor_and_no_outer_terminal_name() {
    // SAFETY: only this test reads or writes TERM_PROGRAM in this test binary.
    unsafe {
        std::env::set_var("TERM_PROGRAM", "outer-term");
        std::env::set_var("TERM_PROGRAM_VERSION", "9");
    }
    let (tx, rx) = channel();
    let pane = Pane::spawn(40, spec("sh", &["-c", "echo C=$COLORTERM P=${TERM_PROGRAM-unset} V=${TERM_PROGRAM_VERSION-unset}"]), 24, 80, tx).unwrap();
    assert!(wait_exited(&rx));
    let text = pane.screen_lock().screen().contents();
    assert!(text.contains("C=truecolor P=unset V=unset"), "{text}");
}
