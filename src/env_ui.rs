//! Drawing the environment panel.
use crate::app::{App, Focus};
use crate::env::{Connectors, McpStatus};
use crate::sidebar::truncate_to_width;
use crate::theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::path::Path;
use unicode_width::UnicodeWidthStr;

fn faint() -> Style {
    Style::default().fg(theme::TEXT_FAINT)
}

fn text() -> Style {
    Style::default().fg(theme::TEXT)
}

/// `~/Documents` style parent: the home dir becomes `~`.
fn tilde(p: &Path) -> String {
    match dirs::home_dir().and_then(|h| p.strip_prefix(&h).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => p.display().to_string(),
    }
}

/// Label on the left, value pushed to the right edge.
fn split_row(label: &str, label_style: Style, value: &str, value_style: Style, width: usize) -> Line<'static> {
    // the value always shows; the label gives way
    let value = truncate_to_width(value, width);
    let label = truncate_to_width(label, width.saturating_sub(value.width() + 1));
    let pad = width.saturating_sub(label.width() + value.width());
    Line::from(vec![Span::styled(label, label_style), Span::raw(" ".repeat(pad)), Span::styled(value, value_style)])
}

fn rule(width: usize) -> Line<'static> {
    Line::styled("─".repeat(width), Style::default().fg(theme::DIVIDER))
}

fn bar(pct: u8) -> String {
    let full = ((usize::from(pct) + 10) / 20).min(5);
    format!("{}{}", theme::BAR_FULL.to_string().repeat(full), theme::BAR_EMPTY.to_string().repeat(5 - full))
}

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    crate::ui::draw_title(f, area, "environment", app.focus == Focus::Env);
    let inner = crate::ui::inner(area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    // column 0 is for the selection edge
    let w = (inner.width as usize).saturating_sub(1);
    let env = &app.env;
    let mut lines: Vec<Line> = Vec::new();
    let chat = env.chat.as_deref().and_then(|id| app.find_chat(id));
    let Some(chat) = chat else {
        lines.push(Line::styled("no chat open", faint()));
        f.render_widget(Paragraph::new(indent(lines, None)), inner);
        return;
    };
    // the line each selectable row landed on
    let mut row_lines: Vec<usize> = Vec::new();
    let clip = |s: String| truncate_to_width(&s, w);
    lines.push(Line::styled(clip(format!("this computer · {}", env.host)), text()));
    let cwd = &chat.meta.cwd;
    let name = cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let parent = cwd.parent().map(tilde).unwrap_or_default();
    lines.push(Line::styled(clip(format!("{name} · {parent}")), text()));
    lines.push(match env.branch.as_str() {
        "" => Line::styled("…", faint()),
        "no git" => Line::styled("no git", faint()),
        b => Line::styled(clip(b.to_string()), text()),
    });
    let remote_on = app.remote.contains(&chat.meta.id);
    let (remote, remote_style) = if remote_on { ("on", theme::warn()) } else { ("off", theme::hint()) };
    row_lines.push(lines.len());
    lines.push(split_row("remote control", text(), remote, remote_style, w));
    lines.push(rule(w));
    let (pct_text, pct_style) = match env.context_pct() {
        Some(p) => (format!("{p}% {}", bar(p)), if p >= 80 { theme::warn() } else { text() }),
        None => ("–".to_string(), faint()),
    };
    lines.push(split_row("context", text(), &pct_text, pct_style, w));
    let (summary, summary_style) = match &env.connectors {
        Connectors::Checking => ("checking…".to_string(), faint()),
        Connectors::Unavailable => ("unavailable".to_string(), faint()),
        Connectors::Ready(list) => {
            let bad = list.iter().filter(|c| c.status != McpStatus::Connected).count();
            if bad > 0 { (format!("{bad} to reconnect"), theme::warn()) } else { (format!("{} connected", list.len()), text()) }
        }
    };
    row_lines.push(lines.len());
    lines.push(split_row("connectors", text(), &format!("{summary} {}", theme::CHEVRON), summary_style, w));
    if let (true, Connectors::Ready(list)) = (env.expanded, &env.connectors) {
        for c in list {
            let (s, st) = match c.status {
                McpStatus::Connected => ("connected", text()),
                McpStatus::NeedsAuth => ("needs auth", theme::warn()),
                McpStatus::Failed => ("failed", theme::warn()),
            };
            lines.push(split_row(&format!("  {}", c.name), text(), s, st, w));
        }
    }
    lines.push(rule(w));
    lines.push(Line::styled("OUTPUTS", Style::default().fg(theme::TEXT_DIM)));
    if env.outputs().is_empty() {
        lines.push(Line::styled("no files yet", faint()));
    }
    for p in env.outputs() {
        row_lines.push(lines.len());
        let base = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
        lines.push(Line::styled(clip(base), text()));
    }
    let cursor_line = row_lines.get(env.cursor.min(row_lines.len().saturating_sub(1))).copied();
    let top = scroll_top(env.scroll.get(), cursor_line.unwrap_or(0), lines.len(), inner.height as usize);
    env.scroll.set(top);
    let selected = (app.focus == Focus::Env).then_some(cursor_line).flatten();
    let shown: Vec<Line> = indent(lines, selected).into_iter().skip(top).take(inner.height as usize).collect();
    f.render_widget(Paragraph::new(shown), inner);
}

/// First line to show so `cursor` stays in view with up to 2 lines of context, like the sidebar.
fn scroll_top(scroll: usize, cursor: usize, total: usize, height: usize) -> usize {
    if height == 0 || total <= height {
        return 0;
    }
    let so = 2.min(height.saturating_sub(1) / 2);
    let mut top = scroll.min(total - height);
    if cursor < top + so {
        top = cursor.saturating_sub(so);
    }
    if cursor + so >= top + height {
        top = (cursor + so + 1).saturating_sub(height);
    }
    top.min(total - height)
}

/// Adds the edge column: `▌` and the selection background on the selected line, a space elsewhere.
fn indent(lines: Vec<Line<'static>>, selected: Option<usize>) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            if selected != Some(i) {
                let mut spans = vec![Span::raw(" ")];
                spans.extend(line.spans.into_iter().map(|s| Span::styled(s.content, line.style.patch(s.style))));
                return Line::from(spans);
            }
            let mut spans = vec![Span::styled(theme::SEL_EDGE, theme::warn().bg(theme::SELECTION_BG))];
            spans.extend(line.spans.into_iter().map(|s| Span::styled(s.content, line.style.patch(s.style).bg(theme::SELECTION_BG))));
            Line::from(spans).style(Style::default().bg(theme::SELECTION_BG))
        })
        .collect()
}
