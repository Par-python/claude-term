//! Data behind the environment panel: context use, output files, git branch, connectors.
use crate::event::AppEvent;
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// process group of the `claude mcp list` run in flight, 0 if none; lets quitting kill it
static MCP_PGID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Kills the connector check's process group if one is running. Call on exit.
pub fn kill_running_mcp() {
    let g = MCP_PGID.swap(0, std::sync::atomic::Ordering::SeqCst);
    if g > 0 {
        unsafe { libc::killpg(g, libc::SIGKILL) };
    }
}

const WINDOW: u64 = 200_000;
const WINDOW_1M: u64 = 1_000_000;
const MCP_TIMEOUT: Duration = Duration::from_secs(30);
const OUTPUT_TOOLS: [&str; 4] = ["Write", "Edit", "MultiEdit", "NotebookEdit"];

/// Follows one session file, reading only appended bytes and keeping a half-written last line aside.
pub struct ChatTracker {
    path: PathBuf,
    offset: u64,
    partial: Vec<u8>,
    model: String,
    last_tokens: Option<u64>,
    /// most recent first
    outputs: Vec<PathBuf>,
    /// path -> sequence number of its latest write; `outputs` is rebuilt from it
    seen: std::collections::HashMap<PathBuf, u64>,
    seq: u64,
}

impl ChatTracker {
    pub fn new(path: PathBuf) -> Self {
        ChatTracker { path, offset: 0, partial: Vec::new(), model: String::new(), last_tokens: None, outputs: Vec::new(), seen: Default::default(), seq: 0 }
    }

    /// Reads appended bytes. True if the context or outputs changed.
    pub fn poll(&mut self) -> bool {
        let Ok(mut f) = std::fs::File::open(&self.path) else {
            return false;
        };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len < self.offset {
            // truncated or replaced: start over
            *self = ChatTracker::new(std::mem::take(&mut self.path));
            return self.poll();
        }
        if f.seek(SeekFrom::Start(self.offset)).is_err() {
            return false;
        }
        let mut buf = Vec::new();
        if f.read_to_end(&mut buf).is_err() {
            return false;
        }
        self.offset += buf.len() as u64;
        self.partial.extend_from_slice(&buf);
        let mut changed = false;
        let data = std::mem::take(&mut self.partial);
        let mut start = 0;
        while let Some(i) = data[start..].iter().position(|&b| b == b'\n') {
            changed |= self.feed(&data[start..start + i]);
            start += i + 1;
        }
        self.partial = data[start..].to_vec();
        if changed {
            let mut v: Vec<(&PathBuf, &u64)> = self.seen.iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1));
            self.outputs = v.into_iter().map(|(p, _)| p.clone()).collect();
        }
        changed
    }

    fn feed(&mut self, raw: &[u8]) -> bool {
        let Ok(v) = serde_json::from_slice::<Value>(raw.trim_ascii()) else {
            return false;
        };
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            return false;
        }
        let Some(msg) = v.get("message") else { return false };
        let mut changed = false;
        let sidechain = v.get("isSidechain").and_then(Value::as_bool).unwrap_or(false);
        let model = msg.get("model").and_then(Value::as_str);
        if let Some(u) = msg.get("usage")
            && !sidechain
            && model != Some("<synthetic>")
        {
            let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            let total = n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens");
            if total > 0 {
                if let Some(m) = model {
                    self.model = m.to_string();
                }
                if self.last_tokens != Some(total) {
                    self.last_tokens = Some(total);
                    changed = true;
                }
            }
        }
        if let Some(blocks) = msg.get("content").and_then(Value::as_array) {
            for b in blocks {
                if b.get("type").and_then(Value::as_str) != Some("tool_use") {
                    continue;
                }
                let name = b.get("name").and_then(Value::as_str).unwrap_or("");
                if !OUTPUT_TOOLS.contains(&name) {
                    continue;
                }
                if let Some(p) = b.get("input").and_then(|i| i.get("file_path")).and_then(Value::as_str) {
                    self.seq += 1;
                    self.seen.insert(PathBuf::from(p), self.seq);
                    changed = true;
                }
            }
        }
        changed
    }

    pub fn context_pct(&self) -> Option<u8> {
        let t = self.last_tokens?;
        let window = if self.model.ends_with("[1m]") { WINDOW_1M } else { WINDOW };
        Some((t * 100 / window).min(100) as u8)
    }

    /// Files written or edited, most recent first, without repeats.
    pub fn outputs(&self) -> &[PathBuf] {
        &self.outputs
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpStatus {
    Connected,
    NeedsAuth,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connector {
    pub name: String,
    pub status: McpStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connectors {
    Checking,
    Unavailable,
    Ready(Vec<Connector>),
}

/// Parses `claude mcp list`. Each line splits on its last ` - `; the name is what precedes the first `: `.
pub fn parse_mcp_list(out: &str) -> Vec<Connector> {
    out.lines()
        .filter_map(|line| {
            let (left, state) = line.rsplit_once(" - ")?;
            let (name, _) = left.split_once(": ")?;
            if !state.trim_start().starts_with(['✔', '!', '✗']) {
                return None;
            }
            let status = if state.contains("Connected") {
                McpStatus::Connected
            } else if state.contains("Needs authentication") {
                McpStatus::NeedsAuth
            } else {
                McpStatus::Failed
            };
            Some(Connector { name: name.trim().to_string(), status })
        })
        .collect()
}

/// `<branch>`, `detached`, or `no git`.
pub fn git_branch(cwd: &Path) -> String {
    let out = Command::new("git").arg("-C").arg(cwd).args(["rev-parse", "--abbrev-ref", "HEAD"]).stdin(Stdio::null()).stderr(Stdio::null()).output();
    match out {
        Ok(o) if o.status.success() => {
            let b = String::from_utf8_lossy(&o.stdout).trim().to_string();
            match b.as_str() {
                "" => "no git".into(),
                "HEAD" => "detached".into(),
                _ => b,
            }
        }
        _ => "no git".into(),
    }
}

pub fn hostname_short() -> String {
    let out = Command::new("hostname").arg("-s").stdin(Stdio::null()).stderr(Stdio::null()).output().ok().filter(|o| o.status.success());
    let name = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty());
    name.or_else(|| std::env::var("HOSTNAME").ok().map(|h| h.split('.').next().unwrap_or("").to_string()).filter(|s| !s.is_empty())).unwrap_or_else(|| "localhost".into())
}

/// Returns the command's stdout, or None if it failed or timed out.
pub type McpRunner = Box<dyn Fn() -> Option<String> + Send>;

/// Runs `run` now and then every `every`, one run at a time, sending each result.
pub fn spawn_mcp_worker(run: McpRunner, tx: Sender<AppEvent>, every: Duration) {
    std::thread::spawn(move || {
        loop {
            let c = match run() {
                Some(out) => Connectors::Ready(parse_mcp_list(&out)),
                None => Connectors::Unavailable,
            };
            if tx.send(AppEvent::Connectors(c)).is_err() {
                break;
            }
            std::thread::sleep(every);
        }
    });
}

/// `claude mcp list` with a 30s limit. A child that overruns is killed and reaped.
pub fn real_mcp_runner() -> McpRunner {
    Box::new(|| {
        let mut cmd = Command::new("claude");
        cmd.args(["mcp", "list"]);
        run_with_timeout(cmd, MCP_TIMEOUT)
    })
}

/// Runs `cmd` in its own process group and returns stdout if it exits 0 within `timeout`.
/// On overrun the whole group is killed and the child reaped; the reader thread is left to
/// finish by itself, so a grandchild holding the pipe cannot block the caller.
pub fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Option<String> {
    use std::os::unix::process::CommandExt;
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).process_group(0).spawn().ok()?;
    MCP_PGID.store(child.id() as i32, std::sync::atomic::Ordering::SeqCst);
    let mut stdout = child.stdout.take()?;
    let (otx, orx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = stdout.read_to_end(&mut b);
        let _ = otx.send(String::from_utf8_lossy(&b).into_owned());
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                kill_running_mcp();
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    MCP_PGID.store(0, std::sync::atomic::Ordering::SeqCst);
    if !status.success() {
        return None;
    }
    orx.recv_timeout(Duration::from_secs(1)).ok()
}
