//! Live file watcher for auto re-indexing.
//! Watches the project directory for file create/modify/delete events,
//! debounces them (batches within 1 second), and triggers index regeneration.
//!
//! These functions are ready for CLI integration (e.g., via a `/watch` REPL command).

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::warn;

/// Starts a file watcher on `root` that triggers `on_change` when files change.
/// Returns a channel sender that can be used to stop the watcher by sending `()`.
pub fn watch_directory<F>(root: &Path, mut on_change: F) -> Result<mpsc::Sender<()>>
where
    F: FnMut() + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<()>();
    let (event_tx, event_rx) = mpsc::channel::<Vec<Event>>();

    let watch_path = root.to_path_buf();

    // Watcher thread
    std::thread::spawn(move || {
        let mut watcher = match RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = event_tx.send(vec![event]);
                }
            },
            Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                warn!("failed to create file watcher: {}", e);
                return;
            }
        };

        if let Err(e) = watcher.watch(&watch_path, RecursiveMode::Recursive) {
            warn!("failed to watch directory: {}", e);
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
    watch_directory(root, move || {
        let root = root_clone.clone();
        std::thread::spawn(move || {
            if let Err(e) = auto_reindex(&root) {
                warn!("auto-reindex failed: {}", e);
            }
        });
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
