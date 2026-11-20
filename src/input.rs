use crate::app::{App, Focus, Modal};
use crate::keys::{Global, PrefixCmd, global_action, key_to_bytes, prefix_cmd};
use crate::sidebar::SidebarAction;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

fn plain_char(k: &KeyEvent) -> Option<char> {
    match k.code {
        KeyCode::Char(c) if !k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => Some(c),
        _ => None,
    }
}

impl App {
    /// Returns false to quit.
    pub(crate) fn on_key(&mut self, k: KeyEvent) -> bool {
        // the keys popup: any key closes it and is not acted on
        if self.help {
            if k.kind != KeyEventKind::Release {
                self.help = false;
            }
            return true;
        }
        // a status message goes away on the next key, and that key still counts
        if matches!(self.modal, Modal::Message(_)) {
            self.modal = Modal::None;
        }
        if self.modal != Modal::None {
            self.modal_key(k);
            return true;
        }
        if std::mem::take(&mut self.awaiting_prefix) {
            return match prefix_cmd(&k) {
                Some(cmd) => self.run_prefix(cmd),
                None => true,
            };
        }
        match global_action(&k, &self.cfg) {
            Some(Global::Prefix) => {
                self.awaiting_prefix = true;
                return true;
            }
            // option symbols (like £ for alt+3) only count in the sidebar, so typing in panes still works
            Some(Global::Jump(_)) if self.focus != Focus::Sidebar && !k.modifiers.contains(KeyModifiers::ALT) => {}
            Some(Global::Jump(n)) => {
                self.sidebar.move_chats(n);
                self.open_selected();
                return true;
            }
            None => {}
        }
        if self.scroll_mode {
            self.scroll_key(k);
            return true;
        }
        match self.focus {
            Focus::Sidebar => self.sidebar_key(k),
            Focus::Chat | Focus::Shell => {
                if self.focused_exited() && k.code == KeyCode::Enter {
                    self.restart_focused();
                } else {
                    let bytes = key_to_bytes(&k);
                    if !bytes.is_empty() {
                        self.write_focused(&bytes);
                    }
                }
            }
        }
        true
    }

    fn run_prefix(&mut self, cmd: PrefixCmd) -> bool {
        match cmd {
            PrefixCmd::Quit => return false,
            PrefixCmd::Help => self.help = true,
            PrefixCmd::Scroll => self.enter_scroll_mode(),
            PrefixCmd::FocusLeft => {
                self.scroll_mode = false;
                self.sidebar_visible = true;
                self.focus = Focus::Sidebar;
            }
            PrefixCmd::FocusRight if self.focus == Focus::Sidebar => self.focus = Focus::Chat,
            PrefixCmd::FocusDown if self.focus == Focus::Chat => {
                self.scroll_mode = false;
                self.focus = Focus::Shell;
            }
            PrefixCmd::FocusUp if self.focus == Focus::Shell => {
                self.scroll_mode = false;
                self.focus = Focus::Chat;
            }
            PrefixCmd::Zoom => self.zoomed = !self.zoomed,
            PrefixCmd::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                if !self.sidebar_visible && self.focus == Focus::Sidebar {
                    self.focus = Focus::Chat;
                }
            }
            _ => {}
        }
        true
    }

    fn sidebar_key(&mut self, k: KeyEvent) {
        let c = match k.code {
            KeyCode::Enter => '\n',
            KeyCode::Down => 'j',
            KeyCode::Up => 'k',
            KeyCode::Esc => {
                self.sidebar.set_filter("");
                return;
            }
            _ => match plain_char(&k) {
                Some(c) => c,
                None => return,
            },
        };
        match self.sidebar.handle_key(c) {
            SidebarAction::None => {}
            SidebarAction::Open => self.open_selected(),
            SidebarAction::NewChat => self.new_chat_here(),
            SidebarAction::NewFolder => self.modal = Modal::NewFolder(String::new()),
            SidebarAction::StartFilter => self.modal = Modal::Filter(self.sidebar.filter().to_string()),
        }
    }

    fn modal_key(&mut self, k: KeyEvent) {
        match std::mem::replace(&mut self.modal, Modal::None) {
            Modal::None | Modal::Message(_) => {}
            Modal::ForkPrompt { id } => match k.code {
                KeyCode::Char('f') => self.start_fork(&id),
                KeyCode::Esc => {}
                _ => self.modal = Modal::ForkPrompt { id },
            },
            Modal::Filter(mut s) => match k.code {
                KeyCode::Esc => self.sidebar.set_filter(""),
                KeyCode::Enter => {}
                KeyCode::Backspace => {
                    s.pop();
                    self.sidebar.set_filter(&s);
                    self.modal = Modal::Filter(s);
                }
                _ => {
                    if let Some(c) = plain_char(&k) {
                        s.push(c);
                        self.sidebar.set_filter(&s);
                    }
                    self.modal = Modal::Filter(s);
                }
            },
            Modal::NewFolder(mut s) => match k.code {
                KeyCode::Esc => {}
                KeyCode::Enter => self.new_folder(&s),
                KeyCode::Backspace => {
                    s.pop();
                    self.modal = Modal::NewFolder(s);
                }
                _ => {
                    if let Some(c) = plain_char(&k) {
                        s.push(c);
                    }
                    self.modal = Modal::NewFolder(s);
                }
            },
        }
    }
}
