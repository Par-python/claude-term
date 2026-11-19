use std::collections::HashSet;
use std::process::Command;

struct Line<'a> {
    pid: Option<u32>,
    ppid: Option<u32>,
    args: &'a str,
}

/// Accepts `pid ppid args` and `pid args`.
fn parse_line(line: &str) -> Option<Line<'_>> {
    let (pid_s, rest) = split_tok(line)?;
    let pid = pid_s.parse::<u32>().ok();
    if let Some((second, rest2)) = split_tok(rest)
        && let Ok(ppid) = second.parse::<u32>() {
            return Some(Line { pid, ppid: Some(ppid), args: rest2 });
        }
    Some(Line { pid, ppid: None, args: rest })
}

fn split_tok(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    if s.is_empty() {
        return None;
    }
    let end = s.find(char::is_whitespace).unwrap_or(s.len());
    Some((&s[..end], &s[end..]))
}

fn is_claude(args: &str) -> bool {
    args.split_whitespace().any(|t| t == "claude" || t.ends_with("/claude"))
}

fn ids_in_args(args: &str) -> Vec<String> {
    let toks: Vec<&str> = args.split_whitespace().collect();
    let mut out = vec![];
    for (i, t) in toks.iter().enumerate() {
        if let Some(v) = t.strip_prefix("--resume=").or_else(|| t.strip_prefix("--session-id=")) {
            out.push(v.to_string());
        } else if matches!(*t, "--resume" | "-r" | "--session-id")
            && let Some(v) = toks.get(i + 1) {
                out.push(v.to_string());
            }
    }
    out
}

pub fn session_ids_in_ps(ps_output: &str) -> HashSet<String> {
    ps_output
        .lines()
        .filter_map(parse_line)
        .filter(|l| is_claude(l.args))
        .flat_map(|l| ids_in_args(l.args))
        .collect()
}

/// Checks already-captured ps output. Errs toward true: a claude line that
/// mentions the id anywhere counts, even if the flags can't be parsed.
pub fn is_live_in(id: &str, ps_output: &str, own_pids: &HashSet<u32>) -> bool {
    if id.is_empty() {
        return false;
    }
    ps_output.lines().filter_map(parse_line).any(|l| {
        if l.pid.is_some_and(|p| own_pids.contains(&p)) || l.ppid.is_some_and(|p| own_pids.contains(&p)) {
            return false;
        }
        is_claude(l.args) && (ids_in_args(l.args).iter().any(|i| i == id) || l.args.contains(id))
    })
}

pub fn is_live_elsewhere(id: &str, own_pids: &HashSet<u32>) -> bool {
    match Command::new("ps").args(["-axo", "pid=,ppid=,args="]).output() {
        Ok(o) if o.status.success() => is_live_in(id, &String::from_utf8_lossy(&o.stdout), own_pids),
        // can't tell, so warn
        _ => true,
    }
}
