use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Default)]
struct State {
    last: Option<String>,
    /// unix seconds when ct last exited
    closed_at: Option<u64>,
}

fn read(path: &Path) -> State {
    std::fs::read_to_string(path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

fn write(path: &Path, st: &State) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = toml::to_string(st) {
        let _ = std::fs::write(path, text);
    }
}

/// `~/.config/claude-term/state.toml`, built from the home dir.
pub fn state_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".config").join("claude-term").join("state.toml"))
}

pub fn load_last_from(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    toml::from_str::<State>(&text).ok()?.last.filter(|s| !s.is_empty())
}

pub fn save_last_to(path: &Path, id: &str) {
    let mut st = read(path);
    st.last = Some(id.to_string());
    write(path, &st);
}

pub fn load_closed_at_from(path: &Path) -> Option<u64> {
    read(path).closed_at
}

pub fn save_closed_at_to(path: &Path, secs: u64) {
    let mut st = read(path);
    st.closed_at = Some(secs);
    write(path, &st);
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub fn load_closed_at() -> Option<u64> {
    load_closed_at_from(&state_path()?)
}

/// Records that ct is exiting now, so a quick relaunch knows files written
/// up to this moment came from our own children.
pub fn save_closed_now() {
    if let Some(p) = state_path() {
        save_closed_at_to(&p, now_secs());
    }
}

pub fn load_last() -> Option<String> {
    load_last_from(&state_path()?)
}

pub fn save_last(id: &str) {
    if let Some(p) = state_path() {
        save_last_to(&p, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_missing() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("sub").join("state.toml");
        assert_eq!(load_last_from(&p), None);
        save_last_to(&p, "abc-123");
        assert_eq!(std::fs::read_to_string(&p).unwrap().trim(), "last = \"abc-123\"");
        assert_eq!(load_last_from(&p).as_deref(), Some("abc-123"));
        std::fs::write(&p, "garbage ((").unwrap();
        assert_eq!(load_last_from(&p), None);
    }

    #[test]
    fn closed_at_and_last_coexist() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("state.toml");
        assert_eq!(load_closed_at_from(&p), None);
        save_last_to(&p, "abc");
        save_closed_at_to(&p, 1234);
        assert_eq!(load_last_from(&p).as_deref(), Some("abc"));
        assert_eq!(load_closed_at_from(&p), Some(1234));
        save_last_to(&p, "def");
        assert_eq!(load_closed_at_from(&p), Some(1234));
    }
}
