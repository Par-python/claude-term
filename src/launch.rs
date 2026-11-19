use crate::pane::PaneSpec;
use std::path::Path;

fn claude(args: Vec<String>, cwd: &Path) -> PaneSpec {
    PaneSpec { program: "claude".into(), args, cwd: cwd.to_path_buf() }
}

pub fn resume_spec(id: &str, cwd: &Path) -> PaneSpec {
    claude(vec!["--resume".into(), id.into()], cwd)
}

pub fn new_chat_spec(cwd: &Path) -> (String, PaneSpec) {
    let id = uuid::Uuid::new_v4().to_string();
    let spec = claude(vec!["--session-id".into(), id.clone()], cwd);
    (id, spec)
}

/// Returns the new session id along with the spec, like `new_chat_spec`.
pub fn fork_spec(id: &str, cwd: &Path) -> (String, PaneSpec) {
    let new_id = uuid::Uuid::new_v4().to_string();
    let spec = claude(
        vec!["--resume".into(), id.into(), "--fork-session".into(), "--session-id".into(), new_id.clone()],
        cwd,
    );
    (new_id, spec)
}

pub fn shell_spec(cwd: &Path) -> PaneSpec {
    let program = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/zsh".into());
    PaneSpec { program, args: vec![], cwd: cwd.to_path_buf() }
}

/// True if a `<id>.jsonl` exists in any project dir under `root`.
pub fn has_history(root: &Path, id: &str) -> bool {
    let name = format!("{id}.jsonl");
    std::fs::read_dir(root).is_ok_and(|d| d.flatten().any(|e| e.path().join(&name).is_file()))
}

/// How to start a chat we already know the id of: resume it if claude has history for it,
/// else claim the id with `--session-id` (claude rejects `--resume` for an id it never saw,
/// and `--session-id` for one it has).
pub fn start_spec(id: &str, cwd: &Path, root: &Path) -> PaneSpec {
    if has_history(root, id) {
        resume_spec(id, cwd)
    } else {
        claude(vec!["--session-id".into(), id.into()], cwd)
    }
}

/// Spec for restarting a pane that ran `old`: resume once history exists, otherwise run `old` again.
pub fn restart_spec(old: &PaneSpec, id: &str, cwd: &Path, root: &Path) -> PaneSpec {
    if has_history(root, id) { resume_spec(id, cwd) } else { old.clone() }
}
