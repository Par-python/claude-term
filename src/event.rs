use crate::pane::Notify;

pub enum AppEvent {
    Input(crossterm::event::Event),
    Pty(Notify),
    /// the watcher saw a change; ask the scanner for a fresh scan
    HistoryChanged,
    /// a finished scan from the worker thread
    Scanned(Vec<crate::history::SessionMeta>),
    /// a timer beat, so time-based marks get re-evaluated
    Tick,
    /// connector health from the mcp worker
    Connectors(crate::env::Connectors),
    /// the git branch of a chat's folder, tagged with the chat it was read for
    GitBranch { chat: String, branch: String },
    /// SIGTERM, SIGINT or SIGHUP arrived: leave like prefix+q does
    Quit,
}

/// Whether the app wants this terminal event. With mouse capture on the terminal also
/// reports every motion and release, which would only cause redraws.
pub fn wanted(ev: &crossterm::event::Event) -> bool {
    use crossterm::event::{Event, MouseEventKind};
    match ev {
        Event::Mouse(m) => matches!(m.kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown | MouseEventKind::Down(_)),
        _ => true,
    }
}
