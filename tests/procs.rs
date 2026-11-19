use ct::event::AppEvent;
use ct::pane::{Pane, PaneSpec};
use ct::procs::Procs;
use std::sync::mpsc::{channel, Sender};
use std::time::{Duration, Instant};

fn sleeper() -> PaneSpec {
    PaneSpec { program: "sh".into(), args: vec!["-c".into(), "sleep 5".into()], cwd: std::env::temp_dir() }
}

fn mk(p: &mut Procs, tx: &Sender<AppEvent>) -> Pane {
    let id = p.next_id();
    Pane::spawn(id, sleeper(), 5, 20, tx.clone()).unwrap()
}

fn alive(pid: u32) -> bool {
    std::process::Command::new("kill").args(["-0", &pid.to_string()]).output().unwrap().status.success()
}

#[test]
fn lru_evicts_oldest_beyond_cap() {
    let (tx, _rx) = channel();
    let mut p = Procs::new(2);
    let a = mk(&mut p, &tx);
    let apid = a.pid().unwrap();
    p.insert_chat("a".into(), a);
    std::thread::sleep(Duration::from_millis(5));
    let b = mk(&mut p, &tx);
    p.insert_chat("b".into(), b);
    std::thread::sleep(Duration::from_millis(5));
    let c = mk(&mut p, &tx);
    p.insert_chat("c".into(), c);
    // cap 2 background + the focused one = 3 total, nothing evicted yet
    assert!(p.is_running("a"));
    std::thread::sleep(Duration::from_millis(5));
    let d = mk(&mut p, &tx);
    p.insert_chat("d".into(), d);
    assert!(!p.is_running("a"));
    assert!(p.is_running("b") && p.is_running("c") && p.is_running("d"));
    let end = Instant::now() + Duration::from_secs(2);
    while alive(apid) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive(apid));
    assert_eq!(p.own_chat_pids().len(), 3);
}

#[test]
fn background_output_marks_unread() {
    let (tx, _rx) = channel();
    let mut p = Procs::new(3);
    let a = mk(&mut p, &tx);
    let aid = a.id;
    p.insert_chat("a".into(), a);
    p.on_output(aid, Some("a"));
    assert!(!p.unread.contains("a"));
    p.on_output(aid, Some("other"));
    assert!(p.unread.contains("a"));
    assert!(p.chat("a").is_some());
    assert!(!p.unread.contains("a"));
}

#[test]
fn shell_reused_per_folder() {
    let (tx, _rx) = channel();
    let mut p = Procs::new(3);
    let d = std::env::temp_dir();
    let id1 = p.shell_for(&d, 5, 20, tx.clone()).unwrap().id;
    let id2 = p.shell_for(&d, 5, 20, tx.clone()).unwrap().id;
    assert_eq!(id1, id2);
    assert!(p.own_chat_pids().is_empty());
}

#[test]
fn lru_keeps_recently_touched_not_fifo() {
    let (tx, _rx) = channel();
    let mut p = Procs::new(1);
    let a = mk(&mut p, &tx);
    p.insert_chat("a".into(), a);
    std::thread::sleep(Duration::from_millis(5));
    let b = mk(&mut p, &tx);
    p.insert_chat("b".into(), b);
    std::thread::sleep(Duration::from_millis(5));
    assert!(p.chat("a").is_some());
    std::thread::sleep(Duration::from_millis(5));
    let c = mk(&mut p, &tx);
    p.insert_chat("c".into(), c);
    assert!(!p.is_running("b"));
    assert!(p.is_running("a") && p.is_running("c"));
}

#[test]
fn background_output_is_working_then_unread_then_cleared_by_focus() {
    use ct::procs::{Mark, WORKING};
    let (tx, _rx) = channel();
    let mut p = Procs::new(3);
    let a = mk(&mut p, &tx);
    let aid = a.id;
    p.insert_chat("a".into(), a);
    let t0 = Instant::now();
    assert_eq!(p.mark_at("a", t0), Mark::None);
    p.on_output_at(aid, Some("other"), t0);
    assert_eq!(p.mark_at("a", t0 + Duration::from_millis(500)), Mark::Working);
    assert_eq!(p.mark_at("a", t0 + WORKING - Duration::from_millis(1)), Mark::Working);
    // quiet for 2s: it had output while away, so it's unread
    assert_eq!(p.mark_at("a", t0 + WORKING), Mark::Unread);
    assert_eq!(p.mark_at("a", t0 + Duration::from_secs(60)), Mark::Unread);
    // more output makes it working again
    p.on_output_at(aid, Some("other"), t0 + Duration::from_secs(60));
    assert_eq!(p.mark_at("a", t0 + Duration::from_secs(61)), Mark::Working);
    // focusing clears unread
    assert!(p.chat("a").is_some());
    assert_eq!(p.mark_at("a", t0 + Duration::from_secs(70)), Mark::None);
}

#[test]
fn focused_chat_output_is_working_but_never_unread() {
    use ct::procs::Mark;
    let (tx, _rx) = channel();
    let mut p = Procs::new(3);
    let a = mk(&mut p, &tx);
    let aid = a.id;
    p.insert_chat("a".into(), a);
    let t0 = Instant::now();
    p.on_output_at(aid, Some("a"), t0);
    assert_eq!(p.mark_at("a", t0), Mark::Working);
    assert_eq!(p.mark_at("a", t0 + Duration::from_secs(5)), Mark::None);
}

fn marker_running(marker: &str) -> bool {
    let out = std::process::Command::new("pgrep").args(["-f", marker]).output().unwrap();
    !String::from_utf8_lossy(&out.stdout).trim().is_empty()
}

fn spec_sh(script: &str) -> PaneSpec {
    PaneSpec { program: "sh".into(), args: vec!["-c".into(), script.into()], cwd: std::env::temp_dir() }
}

fn wait_marker(marker: &str, want: bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while marker_running(marker) != want && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn shutdown_kills_whole_process_groups_before_returning() {
    let (tx, _rx) = channel();
    let mut p = Procs::new(3);
    // a unique sleep time works as a marker in `pgrep -f`
    let marker = format!("sleep 4{}.5", std::process::id());
    // one chat with a background grandchild, one that ignores SIGHUP (so it needs SIGKILL)
    let id = p.next_id();
    let a = Pane::spawn(id, spec_sh(&format!("{marker} & {marker}")), 5, 20, tx.clone()).unwrap();
    p.insert_chat("a".into(), a);
    let id = p.next_id();
    let b = Pane::spawn(id, spec_sh(&format!("trap '' HUP; exec {marker}1")), 5, 20, tx.clone()).unwrap();
    p.insert_chat("b".into(), b);
    wait_marker(&marker, true);
    assert!(marker_running(&marker));
    let t = Instant::now();
    p.shutdown();
    // gone as soon as shutdown returns, within the grace period plus slack
    assert!(!marker_running(&marker), "leftover processes");
    assert!(t.elapsed() < Duration::from_millis(1500), "{:?}", t.elapsed());
}
