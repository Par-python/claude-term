//! The look of ct's own chrome: explicit RGB only, no backgrounds except the selected row and popups.

use ratatui::style::{Color, Style};

pub const ACCENT: Color = Color::Rgb(0xD9, 0x77, 0x57);
pub const TEXT_BRIGHT: Color = Color::Rgb(0xE8, 0xE6, 0xE3);
pub const TEXT: Color = Color::Rgb(0xC8, 0xC5, 0xC0);
pub const TEXT_DIM: Color = Color::Rgb(0x7A, 0x76, 0x72);
pub const TEXT_FAINT: Color = Color::Rgb(0x4E, 0x4B, 0x48);
pub const DIVIDER: Color = Color::Rgb(0x2E, 0x2C, 0x2A);
pub const SELECTION_BG: Color = Color::Rgb(0x2A, 0x28, 0x26);

pub const SEL_EDGE: &str = "▌";
pub const FOCUS_MARK: &str = "▸ ";
pub const WORKING: char = '●';
pub const UNREAD: char = '•';

/// A pane or sidebar title: bright when focused, dim otherwise.
pub fn title(focused: bool) -> Style {
    Style::default().fg(if focused { TEXT_BRIGHT } else { TEXT_DIM })
}

pub fn header() -> Style {
    Style::default().fg(TEXT_DIM)
}

pub fn number() -> Style {
    Style::default().fg(TEXT_FAINT)
}

pub fn selected() -> Style {
    Style::default().bg(SELECTION_BG).fg(TEXT_BRIGHT)
}

pub fn hint() -> Style {
    Style::default().fg(TEXT_DIM)
}

pub fn warn() -> Style {
    Style::default().fg(ACCENT)
}
