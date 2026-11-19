use crate::event::AppEvent;
use crate::history::ScanCache;
use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel, RecvTimeoutError};
use std::time::{Duration, Instant};

const QUIET: Duration = Duration::from_millis(300);
// a chat that writes constantly would otherwise postpone the rescan forever
const MAX_WAIT: Duration = Duration::from_secs(2);

/// Keeps the watcher alive; dropping it stops the watch and the debounce thread.
pub struct HistoryWatcher {
    _watcher: notify::RecommendedWatcher,
}

/// Watches `root` recursively and sends one `HistoryChanged` per burst of changes.
pub fn spawn_watcher(root: &Path, tx: Sender<AppEvent>) -> notify::Result<HistoryWatcher> {
    let (raw_tx, raw_rx) = channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = raw_tx.send(());
        }
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    std::thread::spawn(move || {
        // ends when the watcher (and its raw_tx) is dropped
        while raw_rx.recv().is_ok() {
            let start = Instant::now();
            loop {
                let left = MAX_WAIT.saturating_sub(start.elapsed());
                match raw_rx.recv_timeout(QUIET.min(left)) {
                    Ok(()) if !left.is_zero() => continue,
                    Ok(()) | Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            if tx.send(AppEvent::HistoryChanged).is_err() {
                return;
            }
        }
    });
    Ok(HistoryWatcher { _watcher: watcher })
}

/// Starts the scan worker. Send `()` to request a scan; the result comes back as
/// `AppEvent::Scanned`. Requests that pile up during a scan become exactly one more scan.
pub fn spawn_scanner(root: PathBuf, mut cache: ScanCache, tx: Sender<AppEvent>) -> Sender<()> {
    let (req_tx, req_rx) = channel::<()>();
    std::thread::spawn(move || {
        while req_rx.recv().is_ok() {
            // everything queued so far is covered by the scan we are about to run
            while req_rx.try_recv().is_ok() {}
            let metas = cache.scan(&root);
            if tx.send(AppEvent::Scanned(metas)).is_err() {
                return;
            }
        }
    });
    req_tx
}
