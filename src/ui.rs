use crate::app::{App, Focus, Modal};
use crate::pane::Pane;
use crate::procs::Mark;
use crate::sidebar::{Row, truncate_to_width};
use crate::theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use std::sync::atomic::Ordering;

const SIDEBAR_WIDTH: u16 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub sidebar: Option<Rect>,
    pub chat: Option<Rect>,
    pub shell: Option<Rect>,
    pub status: Rect,
}

/// Sidebar 32 cols on the left, then a one column divider, then chat 70% over shell 30% with a one row
/// divider between them, and one status row at the bottom. Every pane rect includes its title row.
/// Zoom gives the whole right side to the focused pane.
pub fn layout(area: Rect, sidebar_visible: bool, zoomed: bool, focus: Focus) -> Layout {
    let status = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, area.height.min(1));
    let body = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
    let sw = if sidebar_visible { SIDEBAR_WIDTH.min(area.width / 2) } else { 0 };
    let sidebar = sidebar_visible.then(|| Rect::new(body.x, body.y, sw, body.height));
    let taken = if sidebar_visible { (sw + 1).min(body.width) } else { 0 };
    let right = Rect::new(body.x + taken, body.y, body.width - taken, body.height);
    let (chat, shell) = match (zoomed, focus) {
        (true, Focus::Chat) => (Some(right), None),
        (true, Focus::Shell) => (None, Some(right)),
        _ => {
            let ch = (u32::from(right.height) * 70 / 100) as u16;
            let sy = (right.y + ch + 1).min(right.bottom());
            (
                Some(Rect::new(right.x, right.y, right.width, ch)),
                Some(Rect::new(right.x, sy, right.width, right.bottom() - sy)),
            )
        }
    };
    Layout { sidebar, chat, shell, status }
}

/// The content area of a pane or the sidebar: the rect minus its title row.
pub fn inner(r: Rect) -> Rect {
    Rect::new(r.x, r.y + u16::from(r.height > 0), r.width, r.height.saturating_sub(1))
}

/// Screen lines used by rows `top..=last`: one each, plus a blank above every header but the first line.
fn lines_used(rows: &[Row], top: usize, last: usize) -> usize {
    let n = last + 1 - top;
    n + rows[top + 1..=last].iter().filter(|r| matches!(r, Row::Folder(_))).count()
}

/// The top row of the sidebar window, worked out in screen lines (blank lines count) so the cursor
/// keeps 2 lines of context. App stores this in `sidebar.scroll`; drawing and clicks read the same value.
pub fn scroll_for(rows: &[Row], scroll: usize, cursor: usize, height: usize) -> usize {
    if rows.is_empty() || height == 0 {
        return 0;
    }
    let last = rows.len() - 1;
    let cursor = cursor.min(last);
    let so = 2.min(height.saturating_sub(1) / 2);
    let mut top = scroll.min(last);
    if cursor < top + so {
        top = cursor.saturating_sub(so);
    }
    let below = (cursor + so).min(last);
    while top < cursor && lines_used(rows, top, below) > height {
        top += 1;
    }
    // don't leave empty space at the bottom when everything fits
    while top > 0 && lines_used(rows, top - 1, last) <= height {
        top -= 1;
    }
    top
}

/// What each screen line of the sidebar shows: a row index, or `None` for the blank line above a header.
/// `scroll` is the stored top row (see `scroll_for`). Drawing and mouse clicks both use this.
pub fn sidebar_lines(rows: &[Row], scroll: usize, cursor: usize, height: usize) -> Vec<Option<usize>> {
    let top = scroll_for(rows, scroll, cursor, height);
    let mut lines: Vec<Option<usize>> = Vec::new();
    for (i, row) in rows.iter().enumerate().skip(top) {
        if lines.len() >= height {
            break;
        }
        if i > top && matches!(row, Row::Folder(_)) {
            lines.push(None);
            if lines.len() >= height {
                break;
            }
        }
        lines.push(Some(i));
    }
    lines
}

pub fn draw(f: &mut Frame, app: &App) {
    let l = layout(f.area(), app.sidebar_visible, app.zoomed, app.focus);
    if let Some(r) = l.sidebar {
        draw_sidebar(f, r, app);
        draw_vline(f, r.right(), r.y, r.height);
    }
    if let Some(r) = l.chat {
        let title = match app.open_chat.as_deref() {
            Some(id) => match app.find_chat(id) {
                Some(c) => {
                    let folder = c.meta.cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    format!("{} · {folder}", c.meta.title)
                }
                None => "claude (copy)".to_string(),
            },
            None => "claude".to_string(),
        };
        let pane = app.open_chat.as_deref().and_then(|id| app.procs.chat_pane(id));
        draw_pane(f, r, &title, pane, app.focus == Focus::Chat, "pick a chat and press enter");
    }
    if let Some(r) = l.shell {
        let title = match &app.shell_cwd {
            Some(c) => format!("shell · {}", c.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            None => "shell".to_string(),
        };
        let pane = app.shell_cwd.as_deref().and_then(|c| app.procs.shell_pane(c));
        draw_pane(f, r, &title, pane, app.focus == Focus::Shell, "");
    }
    if let (Some(c), Some(_)) = (l.chat, l.shell) {
        draw_hline(f, c.x, c.bottom(), c.width, l.status.y);
    }
    draw_status(f, l.status, app);
    if app.help_open() {
        draw_help(f, f.area());
    }
}

fn draw_vline(f: &mut Frame, x: u16, y: u16, h: u16) {
    let area = f.area();
    if x >= area.right() {
        return;
    }
    for yy in y..(y + h).min(area.bottom()) {
        f.buffer_mut()[(x, yy)].set_symbol("│").set_style(Style::default().fg(theme::DIVIDER));
    }
}

fn draw_hline(f: &mut Frame, x: u16, y: u16, w: u16, limit: u16) {
    if y >= limit {
        return;
    }
    for xx in x..(x + w).min(f.area().right()) {
        f.buffer_mut()[(xx, y)].set_symbol("─").set_style(Style::default().fg(theme::DIVIDER));
    }
}

/// The one-row title at the top of a pane: `▸ ` and a bright title when focused, a dim title when not.
fn draw_title(f: &mut Frame, area: Rect, title: &str, focused: bool) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let marker = if focused { Span::styled(theme::FOCUS_MARK, theme::warn()) } else { Span::raw("  ") };
    let text = truncate_to_width(title, (area.width as usize).saturating_sub(2));
    let line = Line::from(vec![marker, Span::styled(text, theme::title(focused))]);
    f.render_widget(Paragraph::new(line), Rect::new(area.x, area.y, area.width, 1));
}

fn draw_sidebar(f: &mut Frame, area: Rect, app: &App) {
    draw_title(f, area, "chats", app.focus == Focus::Sidebar);
    let inner = inner(area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    if app.sidebar.folders.is_empty() {
        let p = Paragraph::new("no chats yet. press N to start one").style(theme::header()).wrap(Wrap { trim: true });
        f.render_widget(p, inner);
        return;
    }
    let rows = app.sidebar.rows();
    let cursor = app.sidebar.cursor.min(rows.len().saturating_sub(1));
    let width = inner.width as usize;
    let lines: Vec<Line> = sidebar_lines(&rows, app.sidebar.scroll, cursor, inner.height as usize)
        .into_iter()
        .map(|slot| {
            let Some(i) = slot else { return Line::raw("") };
            let sel = i == cursor;
            let bg = |st: Style| if sel { st.bg(theme::SELECTION_BG) } else { st };
            let mut spans = match &rows[i] {
                Row::Folder(fi) => {
                    let fo = &app.sidebar.folders[*fi];
                    // the name always starts in column 1; the edge marker takes column 0 when selected
                    let lead = 1;
                    let arrow = if fo.collapsed { 2 } else { 0 };
                    let name = truncate_to_width(&fo.name.to_uppercase(), width.saturating_sub(lead + arrow));
                    let mut v = Vec::new();
                    if sel {
                        v.push(Span::styled(theme::SEL_EDGE, bg(theme::warn())));
                    } else {
                        v.push(Span::raw(" "));
                    }
                    v.push(Span::styled(name, bg(theme::header())));
                    if fo.collapsed {
                        v.push(Span::styled(" ▸", bg(Style::default().fg(theme::TEXT_FAINT))));
                    }
                    v
                }
                Row::Chat(fi, ci) => {
                    let c = &app.sidebar.folders[*fi].chats[*ci];
                    let rel = app.sidebar.relative_number_in(&rows, i).unwrap_or(0);
                    let (mark, mark_style) = match app.procs.mark(&c.meta.id) {
                        Mark::Unread => (theme::UNREAD, theme::warn()),
                        Mark::Working => (theme::WORKING, theme::warn()),
                        Mark::None => (' ', Style::default()),
                    };
                    let title = truncate_to_width(&c.meta.title, width.saturating_sub(7));
                    let title_style = if sel {
                        theme::selected()
                    } else if c.available {
                        Style::default().fg(theme::TEXT)
                    } else {
                        Style::default().fg(theme::TEXT_FAINT)
                    };
                    let num = format!("{rel:>3}");
                    let mut v = Vec::new();
                    if sel {
                        v.push(Span::styled(theme::SEL_EDGE, bg(theme::warn())));
                        v.push(Span::styled(num.chars().skip(1).collect::<String>(), bg(theme::warn())));
                    } else {
                        v.push(Span::styled(num, theme::number()));
                    }
                    let used: usize = 5 + unicode_width::UnicodeWidthStr::width(title.as_str());
                    v.push(Span::styled("  ", bg(Style::default())));
                    v.push(Span::styled(title, title_style));
                    let pad = width.saturating_sub(used + 1);
                    v.push(Span::styled(" ".repeat(pad), bg(Style::default())));
                    v.push(Span::styled(mark.to_string(), bg(mark_style)));
                    v
                }
            };
            if sel {
                let used: usize = spans.iter().map(Span::width).sum();
                if used < width {
                    spans.push(Span::styled(" ".repeat(width - used), theme::selected()));
                }
            }
            Line::from(spans)
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn to_color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn draw_pane(f: &mut Frame, area: Rect, title: &str, pane: Option<&Pane>, focused: bool, empty_hint: &str) {
    draw_title(f, area, title, focused);
    let inner = inner(area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let Some(pane) = pane else {
        let p = Paragraph::new(empty_hint.to_string()).style(theme::header());
        f.render_widget(p, inner);
        return;
    };
    let exited = pane.exited.load(Ordering::SeqCst);
    {
        // tui-term's widget wants vt100 0.16, we parse with 0.18, so draw the cells here
        let guard = pane.screen_lock();
        let screen = guard.screen();
        let buf = f.buffer_mut();
        for row in 0..inner.height {
            for col in 0..inner.width {
                let Some(cell) = screen.cell(row, col) else { continue };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut m = Modifier::empty();
                for (on, flag) in [
                    (cell.bold(), Modifier::BOLD),
                    (cell.italic(), Modifier::ITALIC),
                    (cell.underline(), Modifier::UNDERLINED),
                    (cell.inverse(), Modifier::REVERSED),
                    (cell.dim(), Modifier::DIM),
                ] {
                    if on {
                        m |= flag;
                    }
                }
                let style = Style::reset().fg(to_color(cell.fgcolor())).bg(to_color(cell.bgcolor())).add_modifier(m);
                let sym = if cell.has_contents() { cell.contents() } else { " " };
                buf[(inner.x + col, inner.y + row)].set_symbol(sym).set_style(style);
            }
        }
        if focused && !exited && !screen.hide_cursor() && screen.scrollback() == 0 {
            let (r, c) = screen.cursor_position();
            if r < inner.height && c < inner.width {
                f.set_cursor_position((inner.x + c, inner.y + r));
            }
        }
    }
    if exited && inner.height > 0 {
        let msg = "[process exited, press enter to restart]";
        let row = Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);
        f.render_widget(Paragraph::new(msg).style(theme::header()), row);
    }
}

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let line = match &app.modal {
        Modal::Message(m) if m.starts_with("no folders yet") => Line::styled(m.clone(), Style::default().fg(theme::TEXT)),
        Modal::Message(m) => Line::styled(m.clone(), theme::warn()),
        Modal::Filter(s) => Line::styled(format!("/{s}"), Style::default().fg(theme::TEXT)),
        Modal::NewFolder(s) => Line::styled(format!("new folder path: {s}"), Style::default().fg(theme::TEXT)),
        Modal::ForkPrompt { .. } => Line::styled("chat open elsewhere: f = open a copy, esc = cancel", theme::warn()),
        Modal::None if app.focused_pane().is_some_and(|p| p.scroll_offset() > 0) => {
            let n = app.focused_pane().map_or(0, |p| p.scroll_offset());
            let hint = if app.scroll_mode() { "  (j/k ctrl-u/d g/G, q to exit)" } else { "" };
            Line::styled(format!("scroll: {n} lines up{hint}"), Style::default().fg(theme::TEXT))
        }
        Modal::None if app.scroll_mode() => Line::styled("scroll mode  (j/k ctrl-u/d g/G, q to exit)", Style::default().fg(theme::TEXT)),
        Modal::None if !app.sidebar.filter().is_empty() => Line::styled(format!("/{}", app.sidebar.filter()), theme::hint()),
        Modal::None => Line::styled(format!("{} ? keys", crate::keys::prefix_label(&app.cfg)), theme::hint()),
    };
    let left = line.width();
    f.render_widget(Paragraph::new(line), area);
    let n = app.procs.running_besides(app.open_chat.as_deref());
    if n > 0 {
        let text = format!("{n} running");
        if left + text.len() < area.width as usize {
            let line = Line::styled(text, theme::hint()).right_aligned();
            f.render_widget(Paragraph::new(line), area);
        }
    }
}

/// One popup column: sections stacked with a blank line between, keys aligned. Returns the lines and their width.
fn help_column(secs: &[&crate::help::HelpSection]) -> (Vec<Line<'static>>, usize) {
    let key_w = secs.iter().flat_map(|s| s.rows.iter()).map(|(k, _)| k.chars().count()).max().unwrap_or(0);
    let mut lines = Vec::new();
    let mut width = 0;
    for (i, sec) in secs.iter().enumerate() {
        if i > 0 {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(sec.title.to_uppercase(), Style::default().fg(theme::TEXT_DIM)));
        for (k, d) in sec.rows {
            width = width.max(key_w + 2 + d.chars().count());
            lines.push(Line::from(vec![
                Span::styled(format!("{k:<key_w$}  "), Style::default().fg(theme::ACCENT)),
                Span::styled(*d, Style::default().fg(theme::TEXT)),
            ]));
        }
    }
    (lines, width)
}

const HELP_GUTTER: usize = 3;
const HELP_MORE: &str = "… make the window taller to see all keys";

/// The keys popup: the only box in ct. One column if it fits, else two, else clipped with a hint.
fn draw_help(f: &mut Frame, screen: Rect) {
    let s = crate::help::sections();
    let max_w = screen.width.saturating_sub(2) as usize;
    let max_h = screen.height.saturating_sub(2) as usize;
    // inside the border: one column of padding each side, no padding rows
    let single = help_column(&s.iter().collect::<Vec<_>>());
    let mut cols = vec![single];
    if cols[0].0.len() > max_h {
        let two = vec![help_column(&[&s[0], &s[2]]), help_column(&[&s[1], &s[3]])];
        if two.iter().map(|c| c.1).sum::<usize>() + HELP_GUTTER + 4 <= max_w {
            cols = two;
        }
    }
    let content_h = cols.iter().map(|c| c.0.len()).max().unwrap_or(0);
    let content_w = cols.iter().map(|c| c.1).sum::<usize>() + HELP_GUTTER * (cols.len() - 1);
    let clipped = content_h + 2 > max_h;
    let w = ((content_w + 4).min(max_w)) as u16;
    let h = ((content_h + 2).min(max_h)) as u16;
    if w < 3 || h < 3 {
        return;
    }
    let area = Rect::new(screen.x + (screen.width - w) / 2, screen.y + (screen.height - h) / 2, w, h);
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::DIVIDER))
        .title(Span::styled(" keys ", Style::default().fg(theme::TEXT_DIM)));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let body = Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), inner.height);
    let mut x = body.x;
    for (mut lines, cw) in cols {
        if clipped {
            lines.truncate((body.height as usize).saturating_sub(1));
        }
        let cw = (cw as u16).min(body.right().saturating_sub(x));
        f.render_widget(Paragraph::new(lines), Rect::new(x, body.y, cw, body.height));
        x += cw + HELP_GUTTER as u16;
    }
    if clipped && body.height > 0 {
        let r = Rect::new(body.x, body.bottom() - 1, body.width, 1);
        f.render_widget(Paragraph::new(Line::styled(HELP_MORE, Style::default().fg(theme::TEXT_FAINT))), r);
    }
}
