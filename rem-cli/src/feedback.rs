//! Intent classification feedback loop.
//! Tracks user corrections to intent classification so the system can learn
//! from mistakes. Persisted to `~/.config/rem-cli/feedback.json`.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FeedbackEntry {
    input: String,
    classified_as: String,
    correct_intent: String,
    count: u32,
    timestamp: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FeedbackStore {
    entries: Vec<FeedbackEntry>,
    model: String,
    total_corrections: u32,
}

/// Tracks intent classification corrections for the feedback loop.
pub struct FeedbackTracker {
    store: FeedbackStore,
    path: PathBuf,
    dirty: bool,
}

impl FeedbackTracker {
    /// Creates a new tracker, loading existing feedback from disk.
    pub fn new(model: &str) -> Self {
        let path = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg).join("rem-cli/feedback.json")
        } else {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            home.join(".config/rem-cli/feedback.json")
        };
        Self::load_from_path(model, path)
    }

    /// Creates a tracker from a specific path (used in tests).
    #[cfg(test)]
    pub(crate) fn new_with_path(model: &str, path: PathBuf) -> Self {
        Self::load_from_path(model, path)
    }

    fn load_from_path(model: &str, path: PathBuf) -> Self {
        let store = if path.exists() {
            fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str::<FeedbackStore>(&s).ok())
                .unwrap_or_default()
        } else {
            FeedbackStore {
                model: model.to_string(),
                ..Default::default()
            }
        };

        Self {
            store,
            path,
            dirty: false,
        }
    }

    /// Writes feedback to disk if there are unsaved changes.
    pub fn flush(&mut self) {
        if self.dirty {
            if let Some(parent) = self.path.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    tracing::warn!("failed to create feedback dir: {e}");
                    return;
                }
            }
            match serde_json::to_string_pretty(&self.store) {
                Ok(json) => {
                    if let Err(e) = fs::write(&self.path, json) {
                        tracing::warn!("failed to write feedback: {e}");
                        return;
                    }
                }
                Err(e) => {
                    tracing::warn!("failed to serialize feedback: {e}");
                    return;
                }
            }
            self.dirty = false;
        }
    }
}

impl Drop for FeedbackTracker {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rem-cli-test-feedback-{}.json", name))
    }

    #[test]
    fn flush_noop_when_not_dirty() {
        let path = test_path("flush_noop");
        let _ = std::fs::remove_file(&path);
        let mut tracker = FeedbackTracker::new_with_path("test-model", path.clone());
        tracker.flush();
        assert!(!path.exists());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn new_creates_default_when_no_file() {
        let path = std::env::temp_dir().join("rem-cli-test-feedback-nonexistent.json");
        let _ = std::fs::remove_file(&path);
        let tracker = FeedbackTracker::new_with_path("fresh-model", path.clone());
        assert_eq!(tracker.store.model, "fresh-model");
        let _ = std::fs::remove_file(&path);
    }
}
