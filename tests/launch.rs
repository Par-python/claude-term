use ct::launch::*;
use ct::live::*;
use std::collections::HashSet;
use std::path::Path;

#[test]
fn specs_match_spec_flags() {
    let cwd = Path::new("/work/a");
    let r = resume_spec("abc", cwd);
    assert_eq!(r.program, "claude");
    assert_eq!(r.args, vec!["--resume", "abc"]);
    assert_eq!(r.cwd, cwd);
    let (fid, f) = fork_spec("abc", cwd);
    assert_eq!(f.args, vec!["--resume".to_string(), "abc".into(), "--fork-session".into(), "--session-id".into(), fid.clone()]);
    assert_eq!(uuid::Uuid::parse_str(&fid).unwrap().get_version_num(), 4);
    assert_ne!(fid, "abc");
    let (id, n) = new_chat_spec(cwd);
    assert_eq!(n.args, vec!["--session-id".to_string(), id.clone()]);
    let u = uuid::Uuid::parse_str(&id).unwrap();
    assert_eq!(u.get_version_num(), 4);
    let s = shell_spec(cwd);
    assert!(s.args.is_empty());
    assert_eq!(s.cwd, cwd);
    assert!(!s.program.is_empty());
}

#[test]
fn ps_parsing() {
    let ps = "28490 /Applications/Claude.app/x/claude --output-format stream-json --resume=11111111-2222-4333-8444-555555555555 --allowedTools=Bash\n\
123 claude --resume abc-1\n\
124 claude -r abc-2\n\
125 vim --resume nope-1\n";
    let ids = session_ids_in_ps(ps);
    let want: HashSet<String> = ["11111111-2222-4333-8444-555555555555", "abc-1", "abc-2"]
        .iter().map(|s| s.to_string()).collect();
    assert_eq!(ids, want);
}

#[test]
fn ps_parsing_three_columns_and_session_id() {
    let ps = "10 1 /Users/x/Application Support/Claude/claude.app/Contents/MacOS/claude --session-id=s-1 --foo\n\
11 1 claude --session-id s-2\n";
    let ids = session_ids_in_ps(ps);
    assert!(ids.contains("s-1") && ids.contains("s-2"));
}

#[test]
fn live_check_ignores_own_and_children() {
    let ps = "10 1 claude --resume a1\n11 10 claude --resume a1\n12 1 claude --resume a2\n";
    let own: HashSet<u32> = [10].into_iter().collect();
    assert!(!is_live_in("a1", ps, &own));
    assert!(is_live_in("a2", ps, &own));
    assert!(is_live_in("a1", ps, &HashSet::new()));
}

#[test]
fn live_check_errs_toward_true_when_unparsed() {
    let ps = "10 1 claude --weird zzz-id\n20 1 vim zzz-id\n";
    assert!(is_live_in("zzz-id", ps, &HashSet::new()));
    assert!(!is_live_in("other", ps, &HashSet::new()));
    let ps2 = "20 1 vim zzz-id\n";
    assert!(!is_live_in("zzz-id", ps2, &HashSet::new()));
}

#[test]
fn claude_started_from_own_shell_counts_as_live() {
    // 50 stands in for a shell pane pid; it is not in the chat pid set
    let ps = "50 1 zsh\n51 50 claude --resume sh-1\n";
    let chat_pids: HashSet<u32> = [99].into_iter().collect();
    assert!(is_live_in("sh-1", ps, &chat_pids));
}

#[test]
fn start_spec_resumes_only_with_history() {
    let td = tempfile::tempdir().unwrap();
    let root = td.path();
    std::fs::create_dir_all(root.join("proj")).unwrap();
    let cwd = Path::new("/work/a");
    assert_eq!(start_spec("abc", cwd, root).args, vec!["--session-id", "abc"]);
    std::fs::write(root.join("proj").join("abc.jsonl"), "{}\n").unwrap();
    assert_eq!(start_spec("abc", cwd, root).args, vec!["--resume", "abc"]);
}

#[test]
fn restart_spec_switches_to_resume_once_history_exists() {
    let td = tempfile::tempdir().unwrap();
    let root = td.path();
    std::fs::create_dir_all(root.join("proj")).unwrap();
    let cwd = Path::new("/work/a");
    let (id, fork) = fork_spec("orig", cwd);
    // nothing written yet: run the fork again
    assert_eq!(restart_spec(&fork, &id, cwd, root).args, fork.args);
    std::fs::write(root.join("proj").join(format!("{id}.jsonl")), "{}\n").unwrap();
    assert_eq!(restart_spec(&fork, &id, cwd, root).args, vec!["--resume".to_string(), id.clone()]);
    let (nid, new) = new_chat_spec(cwd);
    std::fs::write(root.join("proj").join(format!("{nid}.jsonl")), "{}\n").unwrap();
    assert_eq!(restart_spec(&new, &nid, cwd, root).args, vec!["--resume".to_string(), nid]);
}

#[test]
fn start_spec_with_remote() {
    let cwd = Path::new("/work/a");
    assert_eq!(start_spec_with("abc", cwd, true, false).args, vec!["--resume", "abc"]);
    assert_eq!(start_spec_with("abc", cwd, true, true).args, vec!["--resume", "abc", "--remote-control"]);
    assert_eq!(start_spec_with("abc", cwd, false, true).args, vec!["--session-id", "abc", "--remote-control"]);
    let root = Path::new("/no/such/root");
    assert_eq!(start_spec("abc", cwd, root).args, vec!["--session-id", "abc"]);
}
