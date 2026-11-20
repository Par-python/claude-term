use ct::history::SessionMeta;
use ct::tree::{group, group_key};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

fn meta(id: &str, cwd: &str, secs: u64) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        cwd: PathBuf::from(cwd),
        title: id.into(),
        modified: SystemTime::UNIX_EPOCH + Duration::from_secs(secs),
        empty: false,
    }
}

#[test]
fn worktree_groups_under_repo() {
    assert_eq!(
        group_key(Path::new("/r/app/.claude/worktrees/feat-x")),
        PathBuf::from("/r/app")
    );
    assert_eq!(group_key(Path::new("/r/app")), PathBuf::from("/r/app"));
}

#[test]
fn folders_sorted_by_latest_chat() {
    let f = group(vec![meta("a", "/x/old", 10), meta("b", "/x/new", 50), meta("c", "/x/old", 20)]);
    let names: Vec<_> = f.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["new", "old"]);
    assert!(f.iter().all(|f| !f.collapsed));
}

#[test]
fn chats_sorted_newest_first() {
    let f = group(vec![meta("a", "/x/p", 10), meta("b", "/x/p", 30), meta("c", "/x/p", 20)]);
    let ids: Vec<_> = f[0].chats.iter().map(|c| c.meta.id.as_str()).collect();
    assert_eq!(ids, ["b", "c", "a"]);
}

#[test]
fn folder_name_is_last_component() {
    let f = group(vec![meta("a", "/Users/x/Documents/notes.v2", 1)]);
    assert_eq!(f[0].name, "notes.v2");
}

#[test]
fn worktree_chat_joins_repo_folder_and_keeps_own_cwd() {
    let f = group(vec![
        meta("a", "/r/app", 1),
        meta("b", "/r/app/.claude/worktrees/w", 2),
    ]);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].path, PathBuf::from("/r/app"));
    assert_eq!(f[0].chats[0].meta.cwd, PathBuf::from("/r/app/.claude/worktrees/w"));
}

#[test]
fn missing_cwd_marks_unavailable() {
    let d = tempfile::tempdir().unwrap();
    let here = d.path().to_str().unwrap();
    let gone = d.path().join("gone");
    let f = group(vec![meta("a", here, 2), meta("b", gone.to_str().unwrap(), 1)]);
    let avail = |id: &str| {
        f.iter().flat_map(|f| &f.chats).find(|c| c.meta.id == id).unwrap().available
    };
    assert!(avail("a"));
    assert!(!avail("b"));
}
