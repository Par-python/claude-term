use crate::tree::{Chat, Folder};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Folder(usize),
    /// (folder idx, chat idx)
    Chat(usize, usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarAction {
    None,
    Open,
    NewChat,
    NewFolder,
    StartFilter,
}

pub struct Sidebar {
    pub folders: Vec<Folder>,
    /// index into `rows()`
    pub cursor: usize,
    /// first visible row; only moves when the cursor leaves the window
    pub scroll: usize,
    filter: String,
    pending_count: Option<u32>,
    pending_g: bool,
}

impl Sidebar {
    pub fn new(folders: Vec<Folder>) -> Self {
        Sidebar { folders, cursor: 0, scroll: 0, filter: String::new(), pending_count: None, pending_g: false }
    }

    /// Visible rows, honoring `collapsed` and `filter`.
    pub fn rows(&self) -> Vec<Row> {
        let needle = self.filter.to_lowercase();
        let mut rows = Vec::new();
        for (fi, f) in self.folders.iter().enumerate() {
            let matching: Vec<usize> = f
                .chats
                .iter()
                .enumerate()
                .filter(|(_, c)| needle.is_empty() || c.meta.title.to_lowercase().contains(&needle))
                .map(|(ci, _)| ci)
                .collect();
            if !needle.is_empty() && matching.is_empty() {
                continue;
            }
            rows.push(Row::Folder(fi));
            if !f.collapsed {
                rows.extend(matching.into_iter().map(|ci| Row::Chat(fi, ci)));
            }
        }
        rows
    }

    /// Moves `scroll` just enough to keep the cursor in a window of `height` rows, with 2 rows of context.
    pub fn ensure_visible(&mut self, height: usize) {
        let len = self.rows().len();
        self.scroll = window_offset(self.scroll, self.cursor, height, len);
    }

    fn clamp_cursor(&mut self, len: usize) {
        self.cursor = self.cursor.min(len.saturating_sub(1));
    }

    /// Move by `delta` chats, skipping headers. From a header, +1 is the first chat
    /// below and -1 is the last chat above. Clamps at the ends.
    pub fn move_chats(&mut self, delta: i32) {
        let rows = self.rows();
        let chat_rows: Vec<usize> =
            rows.iter().enumerate().filter(|(_, r)| matches!(r, Row::Chat(..))).map(|(i, _)| i).collect();
        if chat_rows.is_empty() {
            return;
        }
        let cur = self.cursor.min(rows.len() - 1);
        let target = match rows[cur] {
            Row::Chat(..) => {
                let ord = chat_rows.iter().position(|&i| i == cur).unwrap_or(0) as i64;
                ord + delta as i64
            }
            Row::Folder(_) => {
                let before = chat_rows.iter().filter(|&&i| i < cur).count() as i64;
                if delta > 0 { before + delta as i64 - 1 } else { before + delta as i64 }
            }
        };
        let target = target.clamp(0, chat_rows.len() as i64 - 1) as usize;
        self.cursor = chat_rows[target];
    }

    pub fn next_folder(&mut self) {
        let rows = self.rows();
        if let Some(i) = (self.cursor + 1..rows.len()).find(|&i| matches!(rows[i], Row::Folder(_))) {
            self.cursor = i;
        }
    }

    pub fn prev_folder(&mut self) {
        let rows = self.rows();
        let end = self.cursor.min(rows.len());
        if let Some(i) = (0..end).rev().find(|&i| matches!(rows[i], Row::Folder(_))) {
            self.cursor = i;
        }
    }

    pub fn top(&mut self) {
        self.cursor = 0;
    }

    pub fn bottom(&mut self) {
        self.cursor = self.rows().len().saturating_sub(1);
    }

    /// Collapse or expand the folder under the cursor (only on a header).
    pub fn toggle_collapse(&mut self) {
        if let Some(Row::Folder(fi)) = self.rows().get(self.cursor).copied() {
            self.folders[fi].collapsed = !self.folders[fi].collapsed;
        }
    }

    /// Vim-style relative number of a row (index into `rows()`). None for headers.
    /// On a header cursor, chats count by distance from it: first below is 1, last above is 1.
    pub fn relative_number(&self, row: usize) -> Option<u32> {
        self.relative_number_in(&self.rows(), row)
    }

    /// Same as `relative_number`, for a `rows()` snapshot the caller already has.
    pub fn relative_number_in(&self, rows: &[Row], row: usize) -> Option<u32> {
        if !matches!(rows.get(row)?, Row::Chat(..)) {
            return None;
        }
        let chats_before = |idx: usize| rows[..idx.min(rows.len())].iter().filter(|r| matches!(r, Row::Chat(..))).count() as i64;
        let target = chats_before(row);
        let cur = self.cursor.min(rows.len().saturating_sub(1));
        let cur_pos = chats_before(cur);
        let dist = match rows[cur] {
            Row::Chat(..) => (target - cur_pos).abs(),
            Row::Folder(_) if target >= cur_pos => target - cur_pos + 1,
            Row::Folder(_) => cur_pos - target,
        };
        Some(dist as u32)
    }

    pub fn selected_chat(&self) -> Option<&Chat> {
        match self.rows().get(self.cursor)? {
            Row::Chat(fi, ci) => self.folders.get(*fi)?.chats.get(*ci),
            Row::Folder(_) => None,
        }
    }

    /// Folder of the cursor row, if any.
    pub fn selected_folder(&self) -> Option<&Folder> {
        let rows = self.rows();
        match rows.get(self.cursor.min(rows.len().saturating_sub(1)))? {
            Row::Folder(fi) | Row::Chat(fi, _) => self.folders.get(*fi),
        }
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Set the filter, keeping the cursor on the same chat if still visible, else the first row.
    pub fn set_filter(&mut self, f: &str) {
        let sel = self.selected_chat().map(|c| c.meta.id.clone());
        self.filter = f.to_string();
        let rows = self.rows();
        let pos = sel.and_then(|id| {
            rows.iter().position(|r| matches!(r, Row::Chat(fi, ci) if self.folders[*fi].chats[*ci].meta.id == id))
        });
        self.cursor = pos.unwrap_or(0);
    }

    pub fn handle_key(&mut self, c: char) -> SidebarAction {
        if let Some(d) = c.to_digit(10)
            && (d != 0 || self.pending_count.is_some()) {
                self.pending_count = Some(self.pending_count.unwrap_or(0).saturating_mul(10).saturating_add(d).min(100_000));
                self.pending_g = false;
                return SidebarAction::None;
            }
        let count = self.pending_count.take();
        let n = count.unwrap_or(1).max(1);
        let was_g = std::mem::take(&mut self.pending_g);
        match c {
            'j' => self.move_chats(n.min(i32::MAX as u32) as i32),
            'k' => self.move_chats(-(n.min(i32::MAX as u32) as i32)),
            'g' if was_g => self.top(),
            'g' => self.pending_g = true,
            'G' => self.bottom(),
            '}' => (0..n).for_each(|_| self.next_folder()),
            '{' => (0..n).for_each(|_| self.prev_folder()),
            '\n' | '\r' => {
                return match self.rows().get(self.cursor) {
                    Some(Row::Chat(..)) => SidebarAction::Open,
                    Some(Row::Folder(_)) => {
                        self.toggle_collapse();
                        SidebarAction::None
                    }
                    None => SidebarAction::None,
                };
            }
            'n' => return SidebarAction::NewChat,
            'N' => return SidebarAction::NewFolder,
            '/' => return SidebarAction::StartFilter,
            _ => {}
        }
        SidebarAction::None
    }

    /// Swap in new data, keeping the cursor on the same chat (or folder header) if it survives.
    pub fn replace_folders(&mut self, folders: Vec<Folder>) {
        let sel_chat = self.selected_chat().map(|c| c.meta.id.clone());
        let sel_folder = self.rows().get(self.cursor).and_then(|r| match r {
            Row::Folder(fi) => Some(self.folders[*fi].path.clone()),
            _ => None,
        });
        let mut folders = folders;
        for f in &mut folders {
            if let Some(old) = self.folders.iter().find(|o| o.path == f.path) {
                f.collapsed = old.collapsed;
            }
        }
        self.folders = folders;
        let rows = self.rows();
        let found = rows.iter().position(|r| match r {
            Row::Chat(fi, ci) => sel_chat.as_deref() == Some(self.folders[*fi].chats[*ci].meta.id.as_str()),
            Row::Folder(fi) => sel_chat.is_none() && sel_folder.as_ref() == Some(&self.folders[*fi].path),
        });
        match found {
            Some(i) => self.cursor = i,
            None => self.clamp_cursor(rows.len()),
        }
    }

    /// Move the cursor onto the chat with this id, expanding its folder if needed.
    pub fn select_by_id(&mut self, id: &str) -> bool {
        let Some(fi) = self.folders.iter().position(|f| f.chats.iter().any(|c| c.meta.id == id)) else {
            return false;
        };
        let was_collapsed = self.folders[fi].collapsed;
        self.folders[fi].collapsed = false;
        let pos = self.rows().iter().position(|r| match r {
            Row::Chat(f, c) => self.folders[*f].chats[*c].meta.id == id,
            _ => false,
        });
        match pos {
            Some(i) => {
                self.cursor = i;
                true
            }
            None => {
                self.folders[fi].collapsed = was_collapsed;
                false
            }
        }
    }
}

/// Truncate to a display width, ending with `…` when cut. Never splits a char.
pub fn truncate_to_width(s: &str, width: usize) -> String {
    if UnicodeWidthStr::width(s) <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let budget = width - 1;
    let mut out = String::new();
    for ch in s.chars() {
        out.push(ch);
        if UnicodeWidthStr::width(out.as_str()) > budget {
            out.pop();
            break;
        }
    }
    out.push('…');
    out
}

/// Where the window starts, given the old start. Scrolls only when the cursor comes within
/// 2 rows of an edge (less on tiny windows), and never past the end of the list.
pub fn window_offset(scroll: usize, cursor: usize, height: usize, len: usize) -> usize {
    if height == 0 {
        return 0;
    }
    let so = 2.min(height.saturating_sub(1) / 2);
    let max = len.saturating_sub(height);
    let mut s = scroll.min(max);
    if cursor < s + so {
        s = cursor.saturating_sub(so);
    } else if cursor + so >= s + height {
        s = (cursor + so + 1).saturating_sub(height);
    }
    s.min(max)
}
