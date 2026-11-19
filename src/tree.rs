use crate::history::SessionMeta;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Chat {
    pub meta: SessionMeta,
    /// the chat's cwd still exists on disk
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Folder {
    pub path: PathBuf,
    pub name: String,
    pub chats: Vec<Chat>,
    pub collapsed: bool,
}

/// `<repo>/.claude/worktrees/<name>[/...]` maps to `<repo>`; anything else maps to itself.
pub fn group_key(cwd: &Path) -> PathBuf {
    let comps: Vec<Component> = cwd.components().collect();
    for i in 0..comps.len().saturating_sub(2) {
        if comps[i].as_os_str() == ".claude" && comps[i + 1].as_os_str() == "worktrees" && i > 0 {
            return comps[..i].iter().collect();
        }
    }
    cwd.to_path_buf()
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Group sessions by folder. Folders and chats are sorted newest first; all start expanded.
pub fn group(sessions: Vec<SessionMeta>) -> Vec<Folder> {
    let mut map: HashMap<PathBuf, Vec<Chat>> = HashMap::new();
    for meta in sessions {
        let available = meta.cwd.exists();
        map.entry(group_key(&meta.cwd)).or_default().push(Chat { meta, available });
    }
    let mut folders: Vec<Folder> = map
        .into_iter()
        .map(|(path, mut chats)| {
            chats.sort_by_key(|b| std::cmp::Reverse(b.meta.modified));
            Folder { name: folder_name(&path), path, chats, collapsed: false }
        })
        .collect();
    folders.sort_by(|a, b| {
        b.chats[0].meta.modified.cmp(&a.chats[0].meta.modified).then_with(|| a.path.cmp(&b.path))
    });
    folders
}
