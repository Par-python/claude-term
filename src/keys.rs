use crate::config::Config;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Global {
    Prefix,
    Jump(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixCmd {
    FocusLeft,
    FocusDown,
    FocusUp,
    FocusRight,
    Zoom,
    ToggleSidebar,
    /// scroll mode on the focused pane
    Scroll,
    Quit,
    /// the keys popup
    Help,
}

/// What the prefix config string means. Unknown values fall back to ctrl+space.
enum PrefixKey {
    Space,
    Letter(char),
}

fn parse_prefix(s: &str) -> PrefixKey {
    let s = s.trim().to_lowercase();
    if let Some(rest) = s.strip_prefix("ctrl+") {
        let mut it = rest.chars();
        if let (Some(c), None) = (it.next(), it.next())
            && c.is_ascii_lowercase()
            // these arrive as tab, enter, backspace-ish and newline, so they can't be told apart from plain keys
            && !matches!(c, 'h' | 'i' | 'j' | 'm') {
                return PrefixKey::Letter(c);
            }
    }
    PrefixKey::Space
}

/// The prefix as shown to the user: what `parse_prefix` actually settled on.
pub fn prefix_label(cfg: &Config) -> String {
    match parse_prefix(&cfg.prefix) {
        PrefixKey::Space => "ctrl+space".to_string(),
        PrefixKey::Letter(c) => format!("ctrl+{c}"),
    }
}

const OPT_DOWN: [char; 9] = ['¡', '™', '£', '¢', '∞', '§', '¶', '•', 'ª'];
const OPT_UP: [char; 9] = ['⁄', '€', '‹', '›', 'ﬁ', 'ﬂ', '‡', '°', '·'];
const SHIFT_DIGITS: [char; 9] = ['!', '@', '#', '$', '%', '^', '&', '*', '('];

fn pos(table: &[char; 9], c: char) -> Option<i32> {
    table.iter().position(|&x| x == c).map(|i| i as i32 + 1)
}

pub fn global_action(ev: &KeyEvent, cfg: &Config) -> Option<Global> {
    if ev.kind == KeyEventKind::Release {
        return None;
    }
    let m = ev.modifiers;
    match parse_prefix(&cfg.prefix) {
        PrefixKey::Space => {
            if ev.code == KeyCode::Null
                || (ev.code == KeyCode::Char(' ') && m.contains(KeyModifiers::CONTROL))
            {
                return Some(Global::Prefix);
            }
        }
        PrefixKey::Letter(l) => {
            if m.contains(KeyModifiers::CONTROL)
                && matches!(ev.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&l))
            {
                return Some(Global::Prefix);
            }
        }
    }
    // jump_modifier: only "alt" is supported, so any value behaves as alt.
    let KeyCode::Char(c) = ev.code else { return None };
    if m.contains(KeyModifiers::ALT) {
        if let Some(d) = c.to_digit(10).filter(|d| (1..=9).contains(d)) {
            let n = d as i32;
            return Some(Global::Jump(if m.contains(KeyModifiers::SHIFT) { -n } else { n }));
        }
        if let Some(i) = pos(&SHIFT_DIGITS, c) {
            return Some(Global::Jump(-i));
        }
    }
    // mac terminals without "option as meta" send symbols with no modifiers
    if !cfg.mac_option_symbols {
        return None;
    }
    if let Some(n) = pos(&OPT_DOWN, c) {
        return Some(Global::Jump(n));
    }
    pos(&OPT_UP, c).map(|n| Global::Jump(-n))
}

pub fn prefix_cmd(ev: &KeyEvent) -> Option<PrefixCmd> {
    match ev.code {
        KeyCode::Char('h') => Some(PrefixCmd::FocusLeft),
        KeyCode::Char('j') => Some(PrefixCmd::FocusDown),
        KeyCode::Char('k') => Some(PrefixCmd::FocusUp),
        KeyCode::Char('l') => Some(PrefixCmd::FocusRight),
        KeyCode::Char('z') => Some(PrefixCmd::Zoom),
        KeyCode::Char('b') => Some(PrefixCmd::ToggleSidebar),
        KeyCode::Char('[') => Some(PrefixCmd::Scroll),
        KeyCode::Char('q') => Some(PrefixCmd::Quit),
        KeyCode::Char('?') => Some(PrefixCmd::Help),
        _ => None,
    }
}

fn ctrl_byte(c: char) -> Option<u8> {
    match c {
        'a'..='z' | 'A'..='Z' => Some((c.to_ascii_lowercase() as u8) & 0x1f),
        ' ' | '2' | '@' => Some(0),
        '[' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '6' => Some(0x1e),
        '_' | '/' | '7' => Some(0x1f),
        _ => None,
    }
}

/// Encode a key event as the bytes a terminal would send to a pty.
/// Returns an empty vec for keys we don't encode (and for key releases).
pub fn key_to_bytes(ev: &KeyEvent) -> Vec<u8> {
    if ev.kind == KeyEventKind::Release {
        return Vec::new();
    }
    let m = ev.modifiers;
    let alt = m.contains(KeyModifiers::ALT);
    let ctrl = m.contains(KeyModifiers::CONTROL);
    // xterm modifier parameter
    let p = 1 + m.contains(KeyModifiers::SHIFT) as u8 + 2 * alt as u8 + 4 * ctrl as u8;

    // keys whose modifiers go into a CSI parameter instead of an ESC prefix
    let csi = |plain: &str, num: u8, fin: char| -> Vec<u8> {
        if p > 1 {
            if plain.ends_with('~') {
                format!("\x1b[{num};{p}~").into_bytes()
            } else {
                format!("\x1b[1;{p}{fin}").into_bytes()
            }
        } else {
            plain.as_bytes().to_vec()
        }
    };
    let base: Vec<u8> = match ev.code {
        KeyCode::Up => return csi("\x1b[A", 1, 'A'),
        KeyCode::Down => return csi("\x1b[B", 1, 'B'),
        KeyCode::Right => return csi("\x1b[C", 1, 'C'),
        KeyCode::Left => return csi("\x1b[D", 1, 'D'),
        KeyCode::Home => return csi("\x1b[H", 1, 'H'),
        KeyCode::End => return csi("\x1b[F", 1, 'F'),
        KeyCode::PageUp => return csi("\x1b[5~", 5, '~'),
        KeyCode::PageDown => return csi("\x1b[6~", 6, '~'),
        KeyCode::Delete => return csi("\x1b[3~", 3, '~'),
        KeyCode::F(n @ 1..=4) => {
            return if p > 1 {
                format!("\x1b[1;{p}{}", (b'O' + n) as char).into_bytes()
            } else {
                vec![0x1b, b'O', b'O' + n]
            };
        }
        KeyCode::Enter => {
            // shift+enter and alt+enter are a newline in claude code
            return if m.contains(KeyModifiers::SHIFT) || alt { b"\x1b\r".to_vec() } else { b"\r".to_vec() };
        }
        KeyCode::Char(c) => match ctrl.then(|| ctrl_byte(c)).flatten() {
            Some(b) => vec![b],
            None => c.to_string().into_bytes(),
        },
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        _ => return Vec::new(),
    };
    if alt {
        let mut out = vec![0x1b];
        out.extend(base);
        out
    } else {
        base
    }
}

#[cfg(test)]
mod prefix_label_tests {
    use super::*;

    fn label(p: &str) -> String {
        prefix_label(&crate::config::parse_config(&format!("prefix = \"{p}\"")))
    }

    #[test]
    fn label_follows_the_parsed_prefix() {
        assert_eq!(label("ctrl+h"), "ctrl+space");
        assert_eq!(label("nonsense"), "ctrl+space");
        assert_eq!(label("ctrl+space"), "ctrl+space");
        assert_eq!(label("ctrl+b"), "ctrl+b");
        assert_eq!(label("CTRL+B"), "ctrl+b");
    }
}
