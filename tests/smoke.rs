//! Spawns the real binary in a pty with a fake HOME and drives it a little.
//! The first test never opens a chat. The second opens one with a fake `claude` on PATH
//! (a shell script inside the fake HOME), so the real `claude` is never started.

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn write_session(home: &Path, proj: &str, id: &str, cwd: &Path, title: &str) {
    let dir = home.join(".claude").join("projects").join(proj);
    std::fs::create_dir_all(&dir).unwrap();
    let cwd = cwd.display();
    let text = format!(
        "{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"message\":{{\"role\":\"user\",\"content\":\"hello\"}}}}\n\
         {{\"type\":\"custom-title\",\"customTitle\":\"{title}\"}}\n"
    );
    std::fs::write(dir.join(format!("{id}.jsonl")), text).unwrap();
}

fn screen_text(p: &Arc<Mutex<vt100::Parser>>) -> Vec<String> {
    let guard = p.lock().unwrap();
    guard.screen().contents().lines().map(str::to_string).collect()
}

fn wait_until(p: &Arc<Mutex<vt100::Parser>>, secs: u64, what: &str, ok: impl Fn(&[String]) -> bool) -> Vec<String> {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        let rows = screen_text(p);
        if ok(&rows) {
            return rows;
        }
        assert!(Instant::now() < end, "timed out waiting for {what}; screen:\n{}", rows.join("\n"));
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// index of the row whose relative number is 0 (the cursor row)
fn zero_row(rows: &[String]) -> Option<usize> {
    rows.iter().position(|r| r.split('│').next().unwrap_or("").trim_start_matches(['▌', ' ']).starts_with("0 "))
}

#[test]
fn starts_navigates_and_quits_cleanly() {
    // not under /tmp or /var/folders: ct skips sessions that live there
    let base = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let home = base.path().join("home");
    let alpha = base.path().join("alphadir");
    let beta = base.path().join("betadir");
    for d in [&home, &alpha, &beta] {
        std::fs::create_dir_all(d).unwrap();
    }
    write_session(&home, "-a", "11111111-1111-4111-8111-111111111111", &alpha, "first chat title");
    write_session(&home, "-b", "22222222-2222-4222-8222-222222222222", &beta, "second chat title");

    let pair = native_pty_system()
        .openpty(PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 })
        .unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_ct"));
    cmd.env("HOME", &home);
    cmd.env("TERM", "xterm-256color");
    cmd.cwd(&home);
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let pid = child.process_id().expect("pid");

    let parser = Arc::new(Mutex::new(vt100::Parser::new(NonZeroU16::new(40).unwrap(), NonZeroU16::new(120).unwrap(), 0)));
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let p2 = parser.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            p2.lock().unwrap().process(&buf[..n]);
        }
    });

    let rows = wait_until(&parser, 5, "folders and titles", |rows| {
        let all = rows.join("\n").to_lowercase();
        ["alphadir", "betadir", "first chat title", "second chat title"].iter().all(|s| all.contains(s))
    });
    // the cursor starts on a folder header, so no chat shows 0 yet
    assert!(zero_row(&rows).is_none(), "unexpected 0 row; screen:\n{}", rows.join("\n"));

    writer.write_all(b"j").unwrap();
    writer.flush().unwrap();
    let rows = wait_until(&parser, 5, "cursor on first chat", |rows| zero_row(rows).is_some());
    let first = zero_row(&rows).unwrap();

    writer.write_all(b"j").unwrap();
    writer.flush().unwrap();
    wait_until(&parser, 5, "cursor to move", |rows| zero_row(rows).is_some_and(|r| r != first));

    // prefix (ctrl+space) then q
    writer.write_all(&[0x00]).unwrap();
    writer.write_all(b"q").unwrap();
    writer.flush().unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(Instant::now() < end, "ct did not exit within 3s");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "exit status {status:?}");

    let out = std::process::Command::new("pgrep").args(["-P", &pid.to_string()]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty(), "leftover children: {}", String::from_utf8_lossy(&out.stdout));
}

fn marker_running(marker: &str) -> bool {
    let out = std::process::Command::new("pgrep").args(["-f", marker]).output().unwrap();
    !String::from_utf8_lossy(&out.stdout).trim().is_empty()
}

#[test]
fn quitting_ends_the_chat_process_and_its_grandchildren() {
    run_fake_chat(false);
}

#[test]
fn sigterm_restores_the_terminal_and_ends_the_chat_process() {
    run_fake_chat(true);
}

fn run_fake_chat(sigterm: bool) {
    let base = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let home = base.path().join("home");
    let work = base.path().join("workdir");
    let bin = home.join("bin");
    for d in [&home, &work, &bin] {
        std::fs::create_dir_all(d).unwrap();
    }
    let id = "33333333-3333-4333-8333-333333333333";
    write_session(&home, "-w", id, &work, "fake chat");
    // ct closed just now, so the fresh session file doesn't look like another claude's
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let cfg = home.join(".config").join("claude-term");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(cfg.join("state.toml"), format!("last = \"{id}\"\nclosed_at = {now}\n")).unwrap();
    // a unique sleep time is the marker for `pgrep -f`
    let marker = format!("sleep 6{}{}.25", std::process::id(), sigterm as u8);
    let fake = bin.join("claude");
    std::fs::write(&fake, format!("#!/bin/sh\necho fake-claude-ready\n{marker} &\nexec {marker}\n").replace("\n{marker} &", "\n{marker} &")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

    let pair = native_pty_system().openpty(PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 }).unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_ct"));
    cmd.env("HOME", &home);
    cmd.env("TERM", "xterm-256color");
    cmd.env("SHELL", "/bin/sh");
    cmd.env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
    cmd.cwd(&home);
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let pid = child.process_id().expect("pid");
    let parser = Arc::new(Mutex::new(vt100::Parser::new(NonZeroU16::new(40).unwrap(), NonZeroU16::new(120).unwrap(), 0)));
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let p2 = parser.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            p2.lock().unwrap().process(&buf[..n]);
        }
    });

    wait_until(&parser, 8, "the fake claude to start in its pane", |rows| rows.join("\n").contains("fake-claude-ready"));
    let end = Instant::now() + Duration::from_secs(3);
    while !marker_running(&marker) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(30));
    }
    assert!(marker_running(&marker), "fake claude's sleeps never started");

    assert!(parser.lock().unwrap().screen().alternate_screen(), "ct should be on the alternate screen while running");
    if sigterm {
        // SAFETY: plain signal to the ct we just started.
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    } else {
        writer.write_all(&[0x00]).unwrap();
        writer.write_all(b"q").unwrap();
        writer.flush().unwrap();
    }
    let end = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(Instant::now() < end, "ct did not exit within 3s");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "exit status {status:?}");
    // give the reader thread a moment to see the final bytes
    std::thread::sleep(Duration::from_millis(100));
    assert!(!parser.lock().unwrap().screen().alternate_screen(), "ct left the alternate screen on exit");
    let end = Instant::now() + Duration::from_secs(2);
    while marker_running(&marker) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(30));
    }
    let left = marker_running(&marker);
    if left {
        let _ = std::process::Command::new("pkill").args(["-f", &marker]).status();
    }
    assert!(!left, "the chat's sleeps outlived ct");
}
