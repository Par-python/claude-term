use crate::app::{App, Focus, Modal};
use crate::pane::Pane;
use crate::procs::Mark;
use crate::sidebar::{Row, truncate_to_width};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use std::sync::atomic::Ordering;

const SIDEBAR_WIDTH: u16 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub sidebar: Option<Rect>,
    pub chat: Option<Rect>,
    pub shell: Option<Rect>,
    pub status: Rect,
}

/// Sidebar 32 cols on the left, chat 70% over shell 30% on the right, one status row at the bottom.
/// Zoom gives the whole right side to the focused pane.
pub fn layout(area: Rect, sidebar_visible: bool, zoomed: bool, focus: Focus) -> Layout {
    let status = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, area.height.min(1));
    let body = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
    let sw = if sidebar_visible { SIDEBAR_WIDTH.min(area.width / 2) } else { 0 };
    let sidebar = sidebar_visible.then(|| Rect::new(body.x, body.y, sw, body.height));
    let right = Rect::new(body.x + sw, body.y, body.width - sw, body.height);
    let (chat, shell) = match (zoomed, focus) {
        (true, Focus::Chat) => (Some(right), None),
        (true, Focus::Shell) => (None, Some(right)),
        _ => {
            let ch = (u32::from(right.height) * 70 / 100) as u16;
            (
                Some(Rect::new(right.x, right.y, right.width, ch)),
                Some(Rect::new(right.x, right.y + ch, right.width, right.height - ch)),
            )
        }
    };
    Layout { sidebar, chat, shell, status }
}

/// The area inside a pane's border.
pub fn inner(r: Rect) -> Rect {
    Block::bordered().inner(r)
}

pub fn draw(f: &mut Frame, app: &App) {
    let l = layout(f.area(), app.sidebar_visible, app.zoomed, app.focus);
    if let Some(r) = l.sidebar {
        draw_sidebar(f, r, app);
    }
    if let Some(r) = l.chat {
        let title = match app.open_chat.as_deref() {
            Some(id) => match app.find_chat(id) {
                Some(c) => format!(" claude: {} ", c.meta.title),
                None => " claude (copy) ".to_string(),
            },
            None => " claude ".to_string(),
        };
        let pane = app.open_chat.as_deref().and_then(|id| app.procs.chat_pane(id));
        draw_pane(f, r, &title, pane, app.focus == Focus::Chat, "pick a chat and press enter");
    }
    if let Some(r) = l.shell {
        let title = match &app.shell_cwd {
            Some(c) => format!(" shell: {} ", c.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            None => " shell ".to_string(),
        };
        let pane = app.shell_cwd.as_deref().and_then(|c| app.procs.shell_pane(c));
        draw_pane(f, r, &title, pane, app.focus == Focus::Shell, "");
    }
    draw_status(f, l.status, app);
}

fn border_style(focused: bool) -> Style {
    Style::default().fg(if focused { Color::Cyan } else { Color::DarkGray })
}

fn draw_sidebar(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" chats ").border_style(border_style(app.focus == Focus::Sidebar));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if app.sidebar.folders.is_empty() {
        let p = Paragraph::new("no chats yet. press N to start one")
            .style(Style::default().add_modifier(Modifier::DIM))
            .wrap(Wrap { trim: true });
        f.render_widget(p, inner);
        return;
    }
    let rows = app.sidebar.rows();
    let height = inner.height as usize;
    let cursor = app.sidebar.cursor.min(rows.len().saturating_sub(1));
    let offset = crate::sidebar::window_offset(app.sidebar.scroll, cursor, height, rows.len());
    let width = inner.width as usize;
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(i, row)| {
            let mut line = match row {
                Row::Folder(fi) => {
                    let fo = &app.sidebar.folders[*fi];
                    let arrow = if fo.collapsed { '▸' } else { '▾' };
                    let name = truncate_to_width(&fo.name, width.saturating_sub(2));
                    Line::from(Span::styled(format!("{name} {arrow}"), Style::default().add_modifier(Modifier::DIM)))
                }
                Row::Chat(fi, ci) => {
                    let c = &app.sidebar.folders[*fi].chats[*ci];
                    let rel = app.sidebar.relative_number_in(&rows, i).unwrap_or(0);
                    let (mark, color) = match app.procs.mark(&c.meta.id) {
                        Mark::Unread => ('•', Color::Yellow),
                        Mark::Working => ('●', Color::Green),
                        Mark::None => (' ', Color::Reset),
                    };
                    let title = truncate_to_width(&c.meta.title, width.saturating_sub(6));
                    let base = if c.available { Style::default() } else { Style::default().add_modifier(Modifier::DIM) };
                    Line::from(vec![
                        Span::styled(format!("{rel:>3} "), base.fg(Color::DarkGray)),
                        Span::styled(mark.to_string(), base.fg(color)),
                        Span::styled(format!(" {title}"), base),
                    ])
                }
            };
            if i == cursor {
                let hl = if app.focus == Focus::Sidebar {
                    Style::default().bg(Color::Blue).fg(Color::White)
                } else {
                    Style::default().add_modifier(Modifier::REVERSED | Modifier::DIM)
                };
                line = line.patch_style(hl);
                // fill the rest of the row so the highlight spans the width
                let used = line.width();
                if used < width {
                    line.spans.push(Span::styled(" ".repeat(width - used), hl));
                }
            }
            line
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
    let block = Block::bordered().title(title.to_string()).border_style(border_style(focused));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(pane) = pane else {
        let p = Paragraph::new(empty_hint.to_string()).style(Style::default().add_modifier(Modifier::DIM));
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
        f.render_widget(Paragraph::new(msg).style(Style::default().fg(Color::Black).bg(Color::Yellow)), row);
    }
}

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let line = match &app.modal {
        Modal::Message(m) => Line::styled(m.clone(), Style::default().fg(Color::Yellow)),
        Modal::Filter(s) => Line::from(format!("/{s}")),
        Modal::NewFolder(s) => Line::from(format!("new folder path: {s}")),
        Modal::ForkPrompt { .. } => {
            Line::styled("chat open elsewhere: f = open a copy, esc = cancel", Style::default().fg(Color::Yellow))
        }
        Modal::None if app.focused_pane().is_some_and(|p| p.scroll_offset() > 0) => {
            let n = app.focused_pane().map_or(0, |p| p.scroll_offset());
            let hint = if app.scroll_mode() { "  (j/k ctrl-u/d g/G, q to exit)" } else { "" };
            Line::styled(format!("scroll: {n} lines up{hint}"), Style::default().fg(Color::Yellow))
        }
        Modal::None if app.scroll_mode() => Line::styled("scroll mode  (j/k ctrl-u/d g/G, q to exit)", Style::default().fg(Color::Yellow)),
        Modal::None if !app.sidebar.filter().is_empty() => {
            Line::styled(format!("/{}", app.sidebar.filter()), Style::default().add_modifier(Modifier::DIM))
        }
        Modal::None => Line::raw(""),
    };
    f.render_widget(Paragraph::new(line), area);
}
