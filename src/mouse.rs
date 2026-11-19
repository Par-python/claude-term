//! Mouse wheel, clicks and scroll mode.

use crate::app::{App, Focus, Modal};
use crate::pane::Pane;
use crate::sidebar::{Row, window_offset};
use crate::ui::{inner, layout};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

/// Lines moved per wheel notch.
const WHEEL_LINES: i32 = 3;

fn contains(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

/// Bytes for a wheel notch as an SGR mouse report. `x` and `y` are 1-based and pane-relative.
pub fn wheel_report(up: bool, x: u16, y: u16) -> Vec<u8> {
    format!("\x1b[<{};{x};{y}M", if up { 64 } else { 65 }).into_bytes()
}

impl App {
    /// The pane that has focus, if any.
    pub(crate) fn focused_pane(&self) -> Option<&Pane> {
        match self.focus {
            Focus::Chat => self.open_chat.as_deref().and_then(|id| self.procs.chat_pane(id)),
            Focus::Shell => self.shell_cwd.as_deref().and_then(|c| self.procs.shell_pane(c)),
            Focus::Sidebar => None,
        }
    }

    pub fn scroll_mode(&self) -> bool {
        self.scroll_mode
    }

    /// Prefix + `[`.
    pub(crate) fn enter_scroll_mode(&mut self) {
        if self.focused_pane().is_some() {
            self.scroll_mode = true;
        }
    }

    pub(crate) fn exit_scroll_mode(&mut self) {
        self.scroll_mode = false;
        if let Some(p) = self.focused_pane() {
            p.scroll_to(0);
        }
    }

    /// Keys while scroll mode is on. Everything else is swallowed so it can't reach the pane.
    pub(crate) fn scroll_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let Some(p) = self.focused_pane() else {
            self.scroll_mode = false;
            return;
        };
        let half = (p.screen_lock().screen().size().0.get() / 2).max(1) as i32;
        match k.code {
            KeyCode::Char('k') | KeyCode::Up if !ctrl => p.scroll_by(1),
            KeyCode::Char('j') | KeyCode::Down if !ctrl => p.scroll_by(-1),
            KeyCode::Char('u') if ctrl => p.scroll_by(half),
            KeyCode::Char('d') if ctrl => p.scroll_by(-half),
            KeyCode::PageUp => p.scroll_by(half * 2),
            KeyCode::PageDown => p.scroll_by(-half * 2),
            KeyCode::Char('g') if !ctrl => p.scroll_to(p.scroll_max()),
            KeyCode::Char('G') => p.scroll_to(0),
            KeyCode::Char('q') | KeyCode::Esc if !ctrl => self.exit_scroll_mode(),
            _ => {}
        }
    }

    pub(crate) fn on_mouse(&mut self, m: MouseEvent) {
        if !self.cfg.mouse || !matches!(self.modal, Modal::None | Modal::Message(_)) {
            return;
        }
        let l = layout(self.area, self.sidebar_visible, self.zoomed, self.focus);
        let (x, y) = (m.column, m.row);
        let delta = match m.kind {
            MouseEventKind::ScrollUp => Some(true),
            MouseEventKind::ScrollDown => Some(false),
            MouseEventKind::Down(MouseButton::Left) => None,
            _ => return,
        };
        if matches!(self.modal, Modal::Message(_)) {
            self.modal = Modal::None;
        }
        if let Some(r) = l.sidebar.filter(|r| contains(*r, x, y)) {
            match delta {
                Some(up) => self.sidebar.move_chats(if up { -1 } else { 1 }),
                None => self.click_sidebar(r, y),
            }
            return;
        }
        let target = [(l.chat, Focus::Chat), (l.shell, Focus::Shell)]
            .into_iter()
            .find_map(|(r, f)| r.filter(|r| contains(*r, x, y)).map(|r| (r, f)));
        let Some((r, which)) = target else { return };
        let Some(up) = delta else {
            // a click focuses the pane
            self.scroll_mode = false;
            self.focus = which;
            return;
        };
        let i = inner(r);
        if !contains(i, x, y) {
            return;
        }
        let pane = match which {
            Focus::Chat => self.open_chat.as_deref().and_then(|id| self.procs.chat_pane_mut(id)),
            _ => self.shell_cwd.as_deref().and_then(|c| self.procs.shell_pane_mut(c)),
        };
        let Some(pane) = pane else { return };
        if pane.mouse_mode_on() {
            pane.write(&wheel_report(up, x - i.x + 1, y - i.y + 1));
        } else {
            pane.scroll_by(if up { WHEEL_LINES } else { -WHEEL_LINES });
        }
    }

    fn click_sidebar(&mut self, r: Rect, y: u16) {
        let i = inner(r);
        if y < i.y || y >= i.y + i.height {
            return;
        }
        let rows = self.sidebar.rows();
        let cursor = self.sidebar.cursor.min(rows.len().saturating_sub(1));
        let top = window_offset(self.sidebar.scroll, cursor, i.height as usize, rows.len());
        let idx = top + (y - i.y) as usize;
        match rows.get(idx) {
            Some(Row::Chat(..)) => {
                self.sidebar.cursor = idx;
                self.open_selected();
            }
            Some(Row::Folder(_)) => {
                self.sidebar.cursor = idx;
                self.sidebar.toggle_collapse();
            }
            None => {}
        }
    }
}
