use ct::history::SessionMeta;
use ct::sidebar::{Row, Sidebar, SidebarAction, truncate_to_width};
use ct::tree::{Chat, Folder};
use std::path::PathBuf;
use std::time::SystemTime;
use unicode_width::UnicodeWidthStr;

fn chat(id: &str, title: &str) -> Chat {
    Chat {
        meta: SessionMeta {
            id: id.into(),
            cwd: PathBuf::from("/x"),
            title: title.into(),
            modified: SystemTime::UNIX_EPOCH,
            empty: false,
        },
        available: true,
    }
}

fn folder(name: &str, titles: &[&str]) -> Folder {
    Folder {
        path: PathBuf::from(format!("/x/{name}")),
        name: name.into(),
        chats: titles.iter().enumerate().map(|(i, t)| chat(&format!("{name}{i}"), t)).collect(),
        collapsed: false,
    }
}

// rows: 0 hdr A, 1-3 A0..A2, 4 hdr B, 5-7 B0..B2
fn fixture() -> Sidebar {
    Sidebar::new(vec![folder("A", &["one", "two", "three"]), folder("B", &["four", "five", "six"])])
}

fn keys(s: &mut Sidebar, ks: &str) -> SidebarAction {
    let mut last = SidebarAction::None;
    for c in ks.chars() {
        last = s.handle_key(c);
    }
    last
}

#[test]
fn rows_lists_headers_and_chats() {
    let s = fixture();
    assert_eq!(s.rows().len(), 8);
    assert_eq!(s.rows()[0], Row::Folder(0));
    assert_eq!(s.rows()[5], Row::Chat(1, 0));
}

#[test]
fn relative_numbers_skip_headers() {
    let mut s = fixture();
    s.cursor = 3; // A chat 2
    assert_eq!(s.relative_number(1), Some(2));
    assert_eq!(s.relative_number(3), Some(0));
    assert_eq!(s.relative_number(5), Some(1));
    assert_eq!(s.relative_number(0), None);
    assert_eq!(s.relative_number(4), None);
}

#[test]
fn relative_numbers_from_header() {
    let mut s = fixture();
    s.cursor = 4; // header B
    assert_eq!(s.relative_number(3), Some(1));
    assert_eq!(s.relative_number(2), Some(2));
    assert_eq!(s.relative_number(5), Some(1));
    assert_eq!(s.relative_number(7), Some(3));
}

#[test]
fn move_from_header_lands_on_adjacent_chat() {
    let mut s = fixture();
    s.cursor = 4;
    s.move_chats(1);
    assert_eq!(s.cursor, 5);
    s.cursor = 4;
    s.move_chats(-1);
    assert_eq!(s.cursor, 3);
    s.cursor = 4;
    s.move_chats(2);
    assert_eq!(s.cursor, 6);
}

#[test]
fn count_then_j_moves_n_chats() {
    let mut s = fixture();
    s.cursor = 1;
    keys(&mut s, "3j");
    assert_eq!(s.cursor, 5); // crossed header B
    keys(&mut s, "2k");
    assert_eq!(s.cursor, 2);
}

#[test]
fn count_resets_after_non_digit() {
    let mut s = fixture();
    s.cursor = 1;
    keys(&mut s, "2jj");
    assert_eq!(s.cursor, 5);
}

#[test]
fn move_clamps_at_ends() {
    let mut s = fixture();
    s.cursor = 1;
    s.move_chats(-5);
    assert_eq!(s.cursor, 1);
    s.move_chats(50);
    assert_eq!(s.cursor, 7);
}

#[test]
fn brace_jumps_between_headers() {
    let mut s = fixture();
    s.cursor = 2;
    keys(&mut s, "}");
    assert_eq!(s.cursor, 4);
    keys(&mut s, "}");
    assert_eq!(s.cursor, 4);
    keys(&mut s, "{");
    assert_eq!(s.cursor, 0);
    s.cursor = 6;
    s.prev_folder();
    assert_eq!(s.cursor, 4);
}

#[test]
fn gg_and_capital_g() {
    let mut s = fixture();
    s.cursor = 3;
    keys(&mut s, "G");
    assert_eq!(s.cursor, 7);
    keys(&mut s, "g");
    assert_eq!(s.cursor, 7);
    keys(&mut s, "g");
    assert_eq!(s.cursor, 0);
}

#[test]
fn collapsed_folder_hides_chats_and_numbers_skip_them() {
    let mut s = fixture();
    s.cursor = 0;
    s.toggle_collapse();
    assert_eq!(s.rows().len(), 5);
    assert_eq!(s.cursor, 0);
    s.cursor = 1; // header B
    assert_eq!(s.relative_number(2), Some(1));
    s.cursor = 4; // B chat 2
    assert_eq!(s.relative_number(2), Some(2));
}

#[test]
fn enter_on_header_toggles_and_on_chat_opens() {
    let mut s = fixture();
    assert_eq!(s.handle_key('\n'), SidebarAction::None);
    assert!(s.folders[0].collapsed);
    s.handle_key('\n');
    assert!(!s.folders[0].collapsed);
    s.cursor = 1;
    assert_eq!(s.handle_key('\n'), SidebarAction::Open);
    assert_eq!(s.handle_key('n'), SidebarAction::NewChat);
    assert_eq!(s.handle_key('N'), SidebarAction::NewFolder);
    assert_eq!(s.handle_key('/'), SidebarAction::StartFilter);
}

#[test]
fn filter_matches_titles_case_insensitive_and_hides_empty_folders() {
    let mut s = Sidebar::new(vec![
        folder("A", &["Fix Login", "other"]),
        folder("B", &["nothing"]),
    ]);
    s.set_filter("LOGIN");
    assert_eq!(s.rows(), vec![Row::Folder(0), Row::Chat(0, 0)]);
}

#[test]
fn selected_accessors() {
    let mut s = fixture();
    assert!(s.selected_chat().is_none());
    assert_eq!(s.selected_folder().unwrap().name, "A");
    s.cursor = 6;
    assert_eq!(s.selected_chat().unwrap().meta.id, "B1");
    assert_eq!(s.selected_folder().unwrap().name, "B");
}

#[test]
fn replace_folders_keeps_selection() {
    let mut s = fixture();
    s.cursor = 6; // B1
    s.replace_folders(vec![folder("C", &["new"]), folder("B", &["x", "y", "z"])]);
    assert_eq!(s.selected_chat().unwrap().meta.id, "B1");
    s.replace_folders(vec![folder("C", &["new"])]);
    assert!(s.cursor < s.rows().len());
}

#[test]
fn select_by_id_works() {
    let mut s = fixture();
    assert!(s.select_by_id("B2"));
    assert_eq!(s.cursor, 7);
    assert!(!s.select_by_id("nope"));
}

#[test]
fn truncate_unicode() {
    let t = truncate_to_width("修复登录🐛bug", 6);
    assert!(UnicodeWidthStr::width(t.as_str()) <= 6);
    assert!(t.ends_with('…'));
    assert_eq!(truncate_to_width("short", 10), "short");
    assert_eq!(truncate_to_width("anything", 0), "");
    assert_eq!(truncate_to_width("🐛🐛🐛", 1), "…");
}

#[test]
fn truncate_respects_emoji_presentation_sequences() {
    for w in 0..8 {
        for input in ["☺\u{FE0F}☺\u{FE0F}☺\u{FE0F}x", "👨\u{200D}👩\u{200D}👧 family", "e\u{301}e\u{301}e\u{301}"] {
            let t = truncate_to_width(input, w);
            assert!(UnicodeWidthStr::width(t.as_str()) <= w, "{input:?} w={w} -> {t:?}");
        }
    }
}

#[test]
fn replace_folders_keeps_collapse_state() {
    let mut s = fixture();
    s.folders[1].collapsed = true;
    s.replace_folders(vec![folder("A", &["one"]), folder("B", &["four"])]);
    assert!(s.folders[1].collapsed);
    assert!(!s.folders[0].collapsed);
}

#[test]
fn set_filter_keeps_cursor_on_same_chat() {
    let mut s = fixture();
    s.select_by_id("B1"); // "five"
    s.set_filter("five");
    assert_eq!(s.filter(), "five");
    assert_eq!(s.selected_chat().unwrap().meta.id, "B1");
    s.set_filter("one");
    assert_eq!(s.cursor, 0);
    s.set_filter("");
    assert_eq!(s.rows().len(), 8);
}

#[test]
fn empty_sidebar_does_not_panic() {
    let mut s = Sidebar::new(vec![]);
    assert!(s.selected_folder().is_none());
    assert!(s.selected_chat().is_none());
    s.move_chats(1);
    s.handle_key('j');
    assert_eq!(s.handle_key('\n'), SidebarAction::None);
    s.toggle_collapse();
}

fn long() -> Sidebar {
    let titles: Vec<String> = (0..40).map(|i| format!("chat {i}")).collect();
    let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
    Sidebar::new(vec![folder("L", &refs)])
}

#[test]
fn scroll_holds_still_when_cursor_moves_back_up_inside_the_window() {
    let mut s = long();
    // walk down past a 10-row window
    for _ in 0..20 {
        s.move_chats(1);
        s.ensure_visible(10);
    }
    let before = s.scroll;
    assert!(before > 0);
    s.move_chats(-1);
    s.ensure_visible(10);
    assert_eq!(s.scroll, before, "one step up must not move the list");
    // the cursor stays inside the window with scrolloff 2
    assert!(s.cursor >= s.scroll + 2 && s.cursor < s.scroll + 10 - 2);
}

#[test]
fn scroll_follows_a_jump_to_a_far_row() {
    let mut s = long();
    s.ensure_visible(10);
    assert_eq!(s.scroll, 0);
    s.bottom();
    s.ensure_visible(10);
    assert!(s.cursor >= s.scroll && s.cursor < s.scroll + 10);
    assert_eq!(s.scroll, s.rows().len() - 10);
    s.top();
    s.ensure_visible(10);
    assert_eq!(s.scroll, 0);
}

#[test]
fn scroll_keeps_scrolloff_two_when_moving_down() {
    let mut s = long();
    for _ in 0..12 {
        s.move_chats(1);
        s.ensure_visible(10);
    }
    // cursor never sits closer than 2 rows from the bottom edge
    assert!(s.cursor + 2 < s.scroll + 10, "cursor {} scroll {}", s.cursor, s.scroll);
}
