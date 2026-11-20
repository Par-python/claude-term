use ct::env::*;
use ct::event::AppEvent;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;

fn asst(model: &str, input: u64) -> String {
    format!(
        "{{\"type\":\"assistant\",\"message\":{{\"model\":\"{model}\",\"usage\":{{\"input_tokens\":{input},\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0,\"output_tokens\":1}},\"content\":[]}}}}\n"
    )
}

fn tool(name: &str, path: &str) -> String {
    format!(
        "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"name\":\"{name}\",\"input\":{{\"file_path\":\"{path}\"}}}}]}}}}\n"
    )
}

#[test]
fn tracker_reads_context_from_last_usage() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.jsonl");
    std::fs::write(&p, format!("{}{}", asst("claude-opus-5-5", 10), asst("claude-opus-5-5", 50_000))).unwrap();
    let mut t = ChatTracker::new(p);
    assert_eq!(t.context_pct(), None);
    assert!(t.poll());
    assert_eq!(t.context_pct(), Some(25));
}

#[test]
fn tracker_1m_window() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.jsonl");
    std::fs::write(&p, asst("claude-opus-5-5[1m]", 500_000)).unwrap();
    let mut t = ChatTracker::new(p);
    t.poll();
    assert_eq!(t.context_pct(), Some(50));
}

#[test]
fn tracker_outputs_dedup_recent_first() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.jsonl");
    std::fs::write(&p, format!("{}{}{}", tool("Write", "/x/a"), tool("Edit", "/x/b"), tool("Edit", "/x/a"))).unwrap();
    let mut t = ChatTracker::new(p);
    t.poll();
    assert_eq!(t.outputs(), &[PathBuf::from("/x/a"), PathBuf::from("/x/b")]);
}

#[test]
fn tracker_incremental_partial_line() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.jsonl");
    let line = tool("Write", "/x/a");
    let (h1, h2) = line.split_at(line.len() / 2);
    std::fs::write(&p, h1).unwrap();
    let mut t = ChatTracker::new(p.clone());
    assert!(!t.poll());
    assert!(t.outputs().is_empty());
    let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
    f.write_all(h2.as_bytes()).unwrap();
    assert!(t.poll());
    assert_eq!(t.outputs().len(), 1);
    assert!(!t.poll());
    assert_eq!(t.outputs().len(), 1);
}

#[test]
fn parse_mcp_list_statuses() {
    let out = "Checking MCP server health…\n\nclaude.ai Google Drive: https://x.example/mcp - ✔ Connected\nclaude.ai Railway: https://mcp.railway.com - ! Needs authentication\nlocal: npx foo - bar - ✗ Failed to connect\n";
    let c = parse_mcp_list(out);
    assert_eq!(c.len(), 3);
    assert_eq!(c[0].name, "claude.ai Google Drive");
    assert_eq!(c[0].status, McpStatus::Connected);
    assert_eq!(c[1].name, "claude.ai Railway");
    assert_eq!(c[1].status, McpStatus::NeedsAuth);
    assert_eq!(c[2].name, "local");
    assert_eq!(c[2].status, McpStatus::Failed);
}

#[test]
fn git_branch_cases() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(git_branch(dir.path()), "no git");
    let run = |args: &[&str]| {
        let s = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(s.status.success());
    };
    run(&["init", "-b", "main"]);
    run(&["commit", "--allow-empty", "-m", "x"]);
    assert_eq!(git_branch(dir.path()), "main");
    run(&["checkout", "--detach"]);
    assert_eq!(git_branch(dir.path()), "detached");
}

#[test]
fn mcp_worker_failure_is_unavailable() {
    let (tx, rx) = channel();
    spawn_mcp_worker(Box::new(|| None), tx, Duration::from_secs(300));
    match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
        AppEvent::Connectors(Connectors::Unavailable) => {}
        _ => panic!("expected Unavailable"),
    }
}

#[test]
fn mcp_worker_does_not_pile_up() {
    let (tx, rx) = channel();
    let now = Arc::new(AtomicUsize::new(0));
    let max = Arc::new(AtomicUsize::new(0));
    let (n, m) = (now.clone(), max.clone());
    spawn_mcp_worker(
        Box::new(move || {
            let c = n.fetch_add(1, Ordering::SeqCst) + 1;
            m.fetch_max(c, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(150));
            n.fetch_sub(1, Ordering::SeqCst);
            None
        }),
        tx,
        Duration::from_millis(20),
    );
    for _ in 0..3 {
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    assert_eq!(max.load(Ordering::SeqCst), 1);
}

#[test]
fn tracker_ignores_synthetic_and_zero_usage() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.jsonl");
    let synth = "{\"type\":\"assistant\",\"message\":{\"model\":\"<synthetic>\",\"usage\":{\"input_tokens\":0,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0},\"content\":[]}}\n";
    std::fs::write(&p, format!("{}{}{}", asst("claude-opus-5-5[1m]", 500_000), synth, asst("claude-opus-5-5", 0))).unwrap();
    let mut t = ChatTracker::new(p);
    t.poll();
    assert_eq!(t.context_pct(), Some(50));
}

#[test]
fn tracker_many_lines_is_fast_and_correct() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.jsonl");
    let mut s = String::new();
    for i in 0..20_000 {
        s.push_str(&tool("Write", &format!("/x/{i}")));
    }
    s.push_str(&asst("claude-opus-5-5", 100_000));
    std::fs::write(&p, s).unwrap();
    let mut t = ChatTracker::new(p);
    let start = std::time::Instant::now();
    t.poll();
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(t.outputs().len(), 20_000);
    assert_eq!(t.outputs()[0], PathBuf::from("/x/19999"));
    assert_eq!(t.context_pct(), Some(50));
}

#[test]
fn tracker_sidechain_usage_skipped_outputs_kept() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.jsonl");
    let side = "{\"type\":\"assistant\",\"isSidechain\":true,\"message\":{\"model\":\"claude-opus-5-5\",\"usage\":{\"input_tokens\":150000},\"content\":[{\"type\":\"tool_use\",\"name\":\"Write\",\"input\":{\"file_path\":\"/x/s\"}}]}}\n";
    std::fs::write(&p, format!("{}{}", asst("claude-opus-5-5", 50_000), side)).unwrap();
    let mut t = ChatTracker::new(p);
    t.poll();
    assert_eq!(t.context_pct(), Some(25));
    assert_eq!(t.outputs(), &[PathBuf::from("/x/s")]);
}

#[test]
fn parse_mcp_list_skips_unknown_status() {
    let out = "a: cmd - ✔ Connected\nnote: something - weird\nWarning: x - y\n";
    let c = parse_mcp_list(out);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].name, "a");
}

#[test]
fn run_with_timeout_kills_group_and_returns() {
    let mut cmd = std::process::Command::new("sh");
    cmd.args(["-c", "sleep 60 & sleep 60"]);
    let start = std::time::Instant::now();
    assert_eq!(run_with_timeout(cmd, Duration::from_secs(1)), None);
    assert!(start.elapsed() < Duration::from_secs(2));
    let mut ok = std::process::Command::new("sh");
    ok.args(["-c", "echo hi"]);
    assert_eq!(run_with_timeout(ok, Duration::from_secs(5)).as_deref(), Some("hi\n"));
}

#[test]
fn run_with_timeout_tolerates_invalid_utf8() {
    let mut cmd = std::process::Command::new("sh");
    cmd.args(["-c", "printf 'a: x - \\342\\234\\224 Connected\\n\\377\\n'"]);
    let out = run_with_timeout(cmd, Duration::from_secs(5)).expect("output despite bad bytes");
    assert_eq!(parse_mcp_list(&out).len(), 1);
}
