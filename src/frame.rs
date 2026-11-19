//! Collecting events into one frame, so streaming output draws once per burst.

use crate::app::App;
use crate::event::AppEvent;
use crate::pane::Notify;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// After pane output, wait this long for more before drawing.
pub const SETTLE: Duration = Duration::from_millis(6);
/// But never delay a frame longer than this from its first event.
pub const FRAME_MAX: Duration = Duration::from_millis(16);
/// How often to look again while a synchronized-output block is open.
const HOLD_POLL: Duration = Duration::from_millis(5);

/// Handles at least one event (waiting for it), plus whatever else is due before the next draw.
/// Returns false when the app should quit or the channel is gone.
pub fn collect(app: &mut App, rx: &Receiver<AppEvent>) -> bool {
    let Ok(first) = rx.recv() else { return false };
    let start = Instant::now();
    let saw_output = std::cell::Cell::new(false);
    let take = |app: &mut App, ev: AppEvent| {
        if matches!(ev, AppEvent::Pty(Notify::Output(_))) {
            saw_output.set(true);
        }
        app.handle(ev)
    };
    if !take(app, first) {
        return false;
    }
    loop {
        // everything already waiting
        while let Ok(ev) = rx.try_recv() {
            if !take(app, ev) {
                return false;
            }
        }
        // streaming output usually has more right behind it
        if !saw_output.get() {
            break;
        }
        let left = FRAME_MAX.saturating_sub(start.elapsed());
        if left.is_zero() {
            break;
        }
        match rx.recv_timeout(SETTLE.min(left)) {
            Ok(ev) => {
                if !take(app, ev) {
                    return false;
                }
            }
            Err(RecvTimeoutError::Timeout) => break,
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
    // a program that asked for a synchronized update wants the whole frame at once
    while app.hold_draw() {
        match rx.recv_timeout(HOLD_POLL) {
            Ok(ev) => {
                if !app.handle(ev) {
                    return false;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
    true
}
