use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ct::config::{parse_config, Config};
use ct::keys::*;

fn k(code: KeyCode, m: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, m)
}

fn cfg() -> Config {
    Config::default()
}

#[test]
fn alt_digit_jumps_down() {
    let ev = k(KeyCode::Char('3'), KeyModifiers::ALT);
    assert_eq!(global_action(&ev, &cfg()), Some(Global::Jump(3)));
}

#[test]
fn alt_shift_digit_jumps_up() {
    let m = KeyModifiers::ALT | KeyModifiers::SHIFT;
    assert_eq!(global_action(&k(KeyCode::Char('#'), m), &cfg()), Some(Global::Jump(-3)));
    assert_eq!(global_action(&k(KeyCode::Char('3'), m), &cfg()), Some(Global::Jump(-3)));
}

#[test]
fn mac_option_symbols() {
    for (i, c) in "¡™£¢∞§¶•ª".chars().enumerate() {
        let ev = k(KeyCode::Char(c), KeyModifiers::NONE);
        assert_eq!(global_action(&ev, &cfg()), Some(Global::Jump(i as i32 + 1)), "{c}");
    }
    for (i, c) in "⁄€‹›ﬁﬂ‡°·".chars().enumerate() {
        let ev = k(KeyCode::Char(c), KeyModifiers::NONE);
        assert_eq!(global_action(&ev, &cfg()), Some(Global::Jump(-(i as i32 + 1))), "{c}");
    }
}

#[test]
fn ctrl_space_is_prefix() {
    let a = k(KeyCode::Char(' '), KeyModifiers::CONTROL);
    let b = k(KeyCode::Null, KeyModifiers::NONE);
    assert_eq!(global_action(&a, &cfg()), Some(Global::Prefix));
    assert_eq!(global_action(&b, &cfg()), Some(Global::Prefix));
}

#[test]
fn plain_keys_are_not_global() {
    assert_eq!(global_action(&k(KeyCode::Char('a'), KeyModifiers::NONE), &cfg()), None);
    assert_eq!(global_action(&k(KeyCode::Char('3'), KeyModifiers::NONE), &cfg()), None);
}

#[test]
fn release_is_ignored() {
    let mut ev = k(KeyCode::Char('3'), KeyModifiers::ALT);
    ev.kind = KeyEventKind::Release;
    ev.state = KeyEventState::NONE;
    assert_eq!(global_action(&ev, &cfg()), None);
}

#[test]
fn ctrl_letter_prefix_from_config() {
    let c = parse_config("prefix = \"ctrl+b\"");
    let ev = k(KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(global_action(&ev, &c), Some(Global::Prefix));
    assert_eq!(global_action(&k(KeyCode::Char(' '), KeyModifiers::CONTROL), &c), None);
}

#[test]
fn unparseable_prefix_falls_back() {
    let c = parse_config("prefix = \"weird\"\njump_modifier = \"hyper\"");
    assert_eq!(global_action(&k(KeyCode::Char(' '), KeyModifiers::CONTROL), &c), Some(Global::Prefix));
    assert_eq!(global_action(&k(KeyCode::Char('2'), KeyModifiers::ALT), &c), Some(Global::Jump(2)));
}

#[test]
fn prefix_commands() {
    let p = |c| prefix_cmd(&k(KeyCode::Char(c), KeyModifiers::NONE));
    assert_eq!(p('h'), Some(PrefixCmd::FocusLeft));
    assert_eq!(p('j'), Some(PrefixCmd::FocusDown));
    assert_eq!(p('k'), Some(PrefixCmd::FocusUp));
    assert_eq!(p('l'), Some(PrefixCmd::FocusRight));
    assert_eq!(p('z'), Some(PrefixCmd::Zoom));
    assert_eq!(p('b'), Some(PrefixCmd::ToggleSidebar));
    assert_eq!(p('q'), Some(PrefixCmd::Quit));
    assert_eq!(p('x'), None);
}

#[test]
fn key_to_bytes() {
    let b = |c, m| ct::keys::key_to_bytes(&k(c, m));
    let n = KeyModifiers::NONE;
    assert_eq!(b(KeyCode::Char('a'), n), b"a");
    assert_eq!(b(KeyCode::Char('é'), n), "é".as_bytes());
    assert_eq!(b(KeyCode::Enter, n), b"\r");
    assert_eq!(b(KeyCode::Backspace, n), vec![0x7f]);
    assert_eq!(b(KeyCode::Char('c'), KeyModifiers::CONTROL), vec![0x03]);
    assert_eq!(b(KeyCode::Up, n), b"\x1b[A");
    assert_eq!(b(KeyCode::Down, n), b"\x1b[B");
    assert_eq!(b(KeyCode::Right, n), b"\x1b[C");
    assert_eq!(b(KeyCode::Left, n), b"\x1b[D");
    assert_eq!(b(KeyCode::Char('x'), KeyModifiers::ALT), b"\x1bx");
    assert_eq!(b(KeyCode::Tab, n), b"\t");
    assert_eq!(b(KeyCode::BackTab, KeyModifiers::SHIFT), b"\x1b[Z");
    assert_eq!(b(KeyCode::Esc, n), vec![0x1b]);
    assert_eq!(b(KeyCode::Home, n), b"\x1b[H");
    assert_eq!(b(KeyCode::End, n), b"\x1b[F");
    assert_eq!(b(KeyCode::PageUp, n), b"\x1b[5~");
    assert_eq!(b(KeyCode::PageDown, n), b"\x1b[6~");
    assert_eq!(b(KeyCode::Delete, n), b"\x1b[3~");
    assert_eq!(b(KeyCode::F(1), n), b"\x1bOP");
    assert_eq!(b(KeyCode::F(4), n), b"\x1bOS");
    assert!(b(KeyCode::F(9), n).is_empty());
}

#[test]
fn bad_config_uses_defaults() {
    let c = parse_config("max_background = \"x\"");
    assert_eq!(c.max_background, 5);
    assert_eq!(c.prefix, "ctrl+space");
    assert_eq!(c.jump_modifier, "alt");
    assert_eq!(parse_config("max_background = 2").max_background, 2);
}

#[test]
fn modified_special_keys() {
    let b = |c, m| ct::keys::key_to_bytes(&k(c, m));
    assert_eq!(b(KeyCode::Left, KeyModifiers::CONTROL), b"\x1b[1;5D");
    assert_eq!(b(KeyCode::Up, KeyModifiers::SHIFT), b"\x1b[1;2A");
    assert_eq!(b(KeyCode::Up, KeyModifiers::ALT), b"\x1b[1;3A");
    assert_eq!(b(KeyCode::Delete, KeyModifiers::CONTROL), b"\x1b[3;5~");
    assert_eq!(b(KeyCode::PageDown, KeyModifiers::ALT | KeyModifiers::SHIFT), b"\x1b[6;4~");
    assert_eq!(b(KeyCode::Home, KeyModifiers::CONTROL), b"\x1b[1;5H");
    assert_eq!(b(KeyCode::End, KeyModifiers::SHIFT), b"\x1b[1;2F");
    assert_eq!(b(KeyCode::F(2), KeyModifiers::CONTROL), b"\x1b[1;5Q");
}

#[test]
fn enter_with_modifiers() {
    let b = |m| ct::keys::key_to_bytes(&k(KeyCode::Enter, m));
    assert_eq!(b(KeyModifiers::SHIFT), b"\x1b\r");
    assert_eq!(b(KeyModifiers::ALT), b"\x1b\r");
}

#[test]
fn mac_symbols_can_be_disabled() {
    let c = parse_config("mac_option_symbols = false");
    assert!(!c.mac_option_symbols);
    assert_eq!(c.max_background, 5);
    assert_eq!(c.prefix, "ctrl+space");
    assert_eq!(global_action(&k(KeyCode::Char('£'), KeyModifiers::NONE), &c), None);
    assert!(Config::default().mac_option_symbols);
}

#[test]
fn ctrl_punctuation() {
    let c = |ch| ct::keys::key_to_bytes(&k(KeyCode::Char(ch), KeyModifiers::CONTROL));
    assert_eq!(c('4'), vec![0x1c]);
    assert_eq!(c('7'), vec![0x1f]);
    assert_eq!(c(' '), vec![0]);
    assert_eq!(c('2'), vec![0]);
    assert_eq!(c('@'), vec![0]);
    assert_eq!(c('\\'), vec![0x1c]);
    assert_eq!(c(']'), vec![0x1d]);
    assert_eq!(c('_'), vec![0x1f]);
    assert_eq!(c('['), vec![0x1b]);
    assert_eq!(c('/'), vec![0x1f]);
}

#[test]
fn release_encodes_nothing() {
    let mut ev = k(KeyCode::Char('a'), KeyModifiers::NONE);
    ev.kind = KeyEventKind::Release;
    assert!(ct::keys::key_to_bytes(&ev).is_empty());
}

#[test]
fn prefixes_that_look_like_tab_enter_or_newline_fall_back_to_ctrl_space() {
    for l in ['h', 'i', 'j', 'm'] {
        let c = parse_config(&format!("prefix = \"ctrl+{l}\""));
        // not the prefix...
        assert_eq!(global_action(&k(KeyCode::Char(l), KeyModifiers::CONTROL), &c), None, "ctrl+{l}");
        // ...ctrl+space is
        assert_eq!(global_action(&k(KeyCode::Char(' '), KeyModifiers::CONTROL), &c), Some(Global::Prefix));
    }
    let c = parse_config("prefix = \"ctrl+[\"");
    assert_eq!(global_action(&k(KeyCode::Char(' '), KeyModifiers::CONTROL), &c), Some(Global::Prefix));
}

#[test]
fn mouse_config_defaults_on() {
    assert!(parse_config("").mouse);
    assert!(!parse_config("mouse = false").mouse);
}

#[test]
fn prefix_bracket_is_scroll_mode() {
    assert_eq!(prefix_cmd(&k(KeyCode::Char('['), KeyModifiers::NONE)), Some(PrefixCmd::Scroll));
}
