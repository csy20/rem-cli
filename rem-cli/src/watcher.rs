//! Live file watcher for auto re-indexing.
//! Watches the project directory for file create/modify/delete events,
//! debounces them (batches within 1 second), and triggers index regeneration.
//!
//! These functions are ready for CLI integration (e.g., via a `/watch` REPL command).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::warn;

fn create_watcher_with_retry(
    fail_tx: &mpsc::Sender<()>,
    event_tx: &mpsc::Sender<Vec<Event>>,
) -> Option<RecommendedWatcher> {
    for attempt in 1..=3 {
        let tx_clone = event_tx.clone();
        match RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| match res {
                Ok(event) => {
                    let _ = tx_clone.send(vec![event]);
                }
                Err(e) => warn!("notify watch error: {}", e),
            },
            Config::default(),
        ) {
            Ok(w) => return Some(w),
            Err(e) => {
                warn!("failed to create file watcher (attempt {}/3): {}", attempt, e);
                if attempt < 3 {
                    std::thread::sleep(Duration::from_millis(500 * attempt));
                }
            }
        }
    }
    let _ = fail_tx.send(());
    None
}

/// Starts a file watcher on `root` that triggers `on_change` when files change.
/// Returns a channel sender that can be used to stop the watcher by sending `()`.
pub fn watch_directory<F>(root: &Path, mut on_change: F) -> Result<mpsc::Sender<()>>
where
    F: FnMut() + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<()>();
    let (event_tx, event_rx) = mpsc::channel::<Vec<Event>>();

    let watch_path = root.to_path_buf();

    let tx_clone = tx.clone();
    // Watcher thread
    std::thread::spawn(move || {
        let mut watcher = match create_watcher_with_retry(&tx_clone, &event_tx) {
            Some(w) => w,
            None => return,
        };

        // Retry watch() with backoff
        let mut last_err = String::new();
        for attempt in 1..=3 {
            match watcher.watch(&watch_path, RecursiveMode::Recursive) {
                Ok(()) => break,
                Err(e) => {
                    last_err = e.to_string();
                    warn!("failed to watch directory (attempt {}/3): {}", attempt, last_err);
                    std::thread::sleep(Duration::from_millis(200 * attempt));
                }
            }
        }
        if !last_err.is_empty() {
            warn!("failed to watch directory after 3 attempts: {}", last_err);
            return;
        }

        // Debounce loop: collect events within 1 second window
        let debounce = Duration::from_secs(1);
        let mut pending = false;

        loop {
            // Check for stop signal (message or sender dropped)
            match rx.try_recv() {
                Ok(()) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                Err(_) => {}
            }

            if event_rx.recv_timeout(debounce).is_ok() {
                pending = true;
                // Drain any additional events within the debounce window
                while event_rx.recv_timeout(debounce / 4).is_ok() {
                    // collect
                }
            }

            if pending {
                pending = false;
                on_change();
            }

            // Check stop signal again after debounce
            match rx.try_recv() {
                Ok(()) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                Err(_) => {}
            }
        }

        // Drop watcher explicitly
        drop(watcher);
    });

    Ok(tx)
}

/// Watches for file changes and auto-triggers index regeneration.
/// Call with the project root path. Returns a sender that stops the watcher.
pub fn watch_and_reindex(root: &Path) -> Result<mpsc::Sender<()>> {
    let root_clone = root.to_path_buf();
    let reindex_busy = Arc::new(AtomicBool::new(false));
    watch_directory(root, {
        let reindex_busy = Arc::clone(&reindex_busy);
        move || {
            if reindex_busy.swap(true, Ordering::SeqCst) {
                return;
            }
            let root = root_clone.clone();
            let reindex_busy = Arc::clone(&reindex_busy);
            std::thread::spawn(move || {
                // Guard resets reindex_busy on drop — covers both normal exit and panic
                struct ReindexGuard<'a> {
                    flag: &'a AtomicBool,
                }
                impl Drop for ReindexGuard<'_> {
                    fn drop(&mut self) {
                        self.flag.store(false, Ordering::SeqCst);
                    }
                }
                let _guard = ReindexGuard { flag: &reindex_busy };

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if let Err(e) = auto_reindex(&root) {
                        warn!("auto-reindex failed: {}", e);
                    }
                }));
                if let Err(panic) = result {
                    warn!("auto-reindex thread panicked: {:?}", panic);
                }
            });
        }
    })
}

fn auto_reindex(root: &Path) -> Result<()> {
    let (chunks, file_mtimes) = crate::indexer::generate_codebase_index(root)?;
    if !chunks.is_empty() {
        crate::indexer::write_codebase_index(root, chunks, file_mtimes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::EventKind;

    fn should_reindex(event: &Event) -> bool {
        matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        )
    }

    #[test]
    fn should_reindex_accepts_create_modify_remove() {
        use notify::event::{AccessKind, CreateKind, ModifyKind, RemoveKind};

        let make = |kind| Event {
            kind,
            paths: vec![],
            attrs: notify::event::EventAttributes::default(),
        };

        for kind in [
            EventKind::Create(CreateKind::Any),
            EventKind::Modify(ModifyKind::Any),
            EventKind::Remove(RemoveKind::Any),
        ] {
            assert!(should_reindex(&make(kind)));
        }

        for kind in [EventKind::Access(AccessKind::Any), EventKind::Any, EventKind::Other] {
            assert!(!should_reindex(&make(kind)));
        }
    }
}
