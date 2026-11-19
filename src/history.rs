use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const MAX_TITLE_CHARS: usize = 60;
const SKIP_PREFIXES: [&str; 4] = ["/private/tmp", "/tmp", "/var/folders", "/private/var/folders"];

#[derive(Debug, Clone, PartialEq)]
pub struct SessionMeta {
    pub id: String,
    pub cwd: PathBuf,
    pub title: String,
    pub modified: SystemTime,
}

pub fn projects_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".claude")
        .join("projects")
}

fn user_text(v: &Value) -> Option<String> {
    let content = v.get("message")?.get("content")?;
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(blocks) => blocks.iter().find_map(|b| {
            if b.get("type")?.as_str()? != "text" {
                return None;
            }
            let t = b.get("text")?.as_str()?;
            if t.trim_start().starts_with('<') {
                None
            } else {
                Some(t.to_string())
            }
        }),
        _ => None,
    }
}

fn truncate_title(s: &str) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= MAX_TITLE_CHARS {
        return one_line;
    }
    let mut out: String = one_line.chars().take(MAX_TITLE_CHARS - 1).collect();
    out.push('…');
    out
}

/// What has been learned from the start of a session file so far.
#[derive(Debug, Clone, Default)]
struct Parsed {
    /// bytes up to and including the last newline that has been consumed
    offset: u64,
    cwd: Option<PathBuf>,
    custom: Option<String>,
    first_user: Option<String>,
}

impl Parsed {
    /// Folds one line in. Lines that aren't json are skipped.
    fn feed(&mut self, raw: &[u8]) {
        let line = String::from_utf8_lossy(raw);
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            return;
        };
        if self.cwd.is_none()
            && let Some(c) = v.get("cwd").and_then(Value::as_str)
            && !c.is_empty()
        {
            self.cwd = Some(PathBuf::from(c));
        }
        match v.get("type").and_then(Value::as_str) {
            Some("custom-title") => {
                if let Some(t) = v.get("customTitle").and_then(Value::as_str) {
                    let t = truncate_title(t);
                    if !t.is_empty() {
                        self.custom = Some(t);
                    }
                }
            }
            Some("user") if self.first_user.is_none() => {
                if let Some(t) = user_text(&v) {
                    let t = t.trim();
                    if !t.is_empty() && !t.starts_with('<') {
                        self.first_user = Some(t.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    /// Reads `path` from `offset` and feeds every complete line. A last line with no newline
    /// yet is fed too (feeding is idempotent) but not consumed, so it is read again once finished.
    /// Returns the number of bytes read.
    fn read_more(&mut self, file: File) -> u64 {
        use std::io::Seek;
        let mut reader = BufReader::new(file);
        if reader.seek(std::io::SeekFrom::Start(self.offset)).is_err() {
            return 0;
        }
        let mut read = 0u64;
        let mut buf = Vec::new();
        loop {
            buf.clear();
            // read_until + lossy decode so invalid utf-8 doesn't stop the scan
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => read += n as u64,
            }
            self.feed(&buf);
            if buf.ends_with(b"\n") {
                self.offset += buf.len() as u64;
            }
        }
        read
    }

    fn meta(&self, id: &str, modified: SystemTime) -> Option<SessionMeta> {
        let cwd = self.cwd.clone()?;
        let title = match &self.custom {
            Some(t) => t.clone(),
            None => truncate_title(self.first_user.as_deref().unwrap_or("(untitled)")),
        };
        Some(SessionMeta { id: id.to_string(), cwd, title, modified })
    }
}

/// Parse one session file. Returns None if no `cwd` is found.
pub fn parse_session(path: &Path) -> Option<SessionMeta> {
    let file = File::open(path).ok()?;
    let modified = file.metadata().ok()?.modified().ok()?;
    let id = path.file_stem()?.to_string_lossy().into_owned();
    let mut st = Parsed::default();
    st.read_more(file);
    st.meta(&id, modified)
}

fn is_temp(cwd: &Path) -> bool {
    SKIP_PREFIXES.iter().any(|p| cwd.starts_with(p))
}

/// Scan `root/*/*.jsonl`. Skips temp-dir sessions, non-jsonl files and subdirectories.
pub fn scan_projects(root: &Path) -> Vec<SessionMeta> {
    ScanCache::default().scan(root)
}

type Stamp = (SystemTime, u64);

struct Entry {
    stamp: Stamp,
    parsed: Parsed,
    meta: Option<SessionMeta>,
}

/// Remembers what each session file looked like, so a rescan only re-parses
/// files whose mtime or length changed, and a file that only grew is read from where
/// the last scan stopped.
#[derive(Default)]
pub struct ScanCache {
    entries: HashMap<PathBuf, Entry>,
    /// how many files have been parsed (fully or incrementally) so far (a test hook)
    pub parse_count: usize,
    /// how many bytes of session files have been read so far (a test hook)
    pub bytes_read: u64,
}

impl ScanCache {
    pub fn scan(&mut self, root: &Path) -> Vec<SessionMeta> {
        let mut seen: HashMap<PathBuf, Entry> = HashMap::new();
        if let Ok(dirs) = std::fs::read_dir(root) {
            for dir in dirs.flatten() {
                if !dir.path().is_dir() {
                    continue;
                }
                let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
                for f in files.flatten() {
                    let p = f.path();
                    if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                        continue;
                    }
                    let Ok(md) = std::fs::metadata(&p) else { continue };
                    let Ok(mtime) = md.modified() else { continue };
                    if !md.is_file() {
                        continue;
                    }
                    let stamp = (mtime, md.len());
                    let entry = match self.entries.remove(&p) {
                        Some(old) if old.stamp == stamp => old,
                        old => {
                            self.parse_count += 1;
                            // only growth can be read incrementally; a shrink or a rewrite starts over
                            let mut parsed = match old {
                                Some(o) if stamp.1 > o.stamp.1 && o.parsed.offset <= o.stamp.1 => o.parsed,
                                _ => Parsed::default(),
                            };
                            match File::open(&p) {
                                Ok(file) => self.bytes_read += parsed.read_more(file),
                                Err(_) => continue,
                            }
                            let id = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                            let meta = parsed.meta(&id, mtime);
                            Entry { stamp, parsed, meta }
                        }
                    };
                    seen.insert(p, entry);
                }
            }
        }
        // anything not seen this time was deleted
        self.entries = seen;
        let mut out: Vec<SessionMeta> = self
            .entries
            .values()
            .filter_map(|e| e.meta.clone())
            .filter(|m| !is_temp(&m.cwd))
            .collect();
        out.sort_by_key(|b| std::cmp::Reverse(b.modified));
        out
    }
}
