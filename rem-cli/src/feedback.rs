//! Intent classification feedback loop.
//! Tracks user corrections to intent classification so the system can learn
//! from mistakes. Persisted to `~/.config/rem-cli/feedback.json`.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::intent::TaskIntent;

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
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let path = home.join(".config/rem-cli/feedback.json");
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

    /// Records a user correction to intent classification.
    pub fn record_correction(&mut self, input: &str, classified_as: &TaskIntent, correct: &TaskIntent) {
        let classified_str = intent_to_str(classified_as);
        let correct_str = intent_to_str(correct);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let key = format!("{}:{}:{}", input, classified_str, correct_str);

        if let Some(entry) = self
            .store
            .entries
            .iter_mut()
            .find(|e| format!("{}:{}:{}", e.input, e.classified_as, e.correct_intent) == key)
        {
            entry.count += 1;
            entry.timestamp = now;
        } else {
            self.store.entries.push(FeedbackEntry {
                input: input.to_string(),
                classified_as: classified_str,
                correct_intent: correct_str,
                count: 1,
                timestamp: now,
            });
            if self.store.entries.len() > 500 {
                self.store.entries.sort_by_key(|e| e.timestamp);
                self.store.entries.drain(0..(self.store.entries.len() - 500));
            }
        }

        self.store.total_corrections += 1;
        self.dirty = true;
    }

    /// Writes feedback to disk if there are unsaved changes.
    pub fn flush(&mut self) {
        if self.dirty {
            if let Some(parent) = self.path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(&self.store) {
                let _ = fs::write(&self.path, json);
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

fn intent_to_str(intent: &TaskIntent) -> String {
    match intent {
        TaskIntent::FastAnswer => "FastAnswer".to_string(),
        TaskIntent::Planning => "Planning".to_string(),
        TaskIntent::WebNeeded => "WebNeeded".to_string(),
        TaskIntent::CodeAction => "CodeAction".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::TaskIntent;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rem-cli-test-feedback-{}.json", name))
    }

    #[test]
    fn record_correction_adds_entry() {
        let path = test_path("adds_entry");
        let _ = std::fs::remove_file(&path);
        let mut tracker = FeedbackTracker::new_with_path("test-model", path.clone());

        tracker.record_correction("hello", &TaskIntent::FastAnswer, &TaskIntent::Planning);
        assert_eq!(tracker.store.total_corrections, 1);
        assert_eq!(tracker.store.entries.len(), 1);
        assert!(tracker.dirty);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn record_correction_increments_existing() {
        let path = test_path("increments");
        let _ = std::fs::remove_file(&path);
        let mut tracker = FeedbackTracker::new_with_path("test-model", path.clone());

        tracker.record_correction("hello", &TaskIntent::FastAnswer, &TaskIntent::Planning);
        tracker.record_correction("hello", &TaskIntent::FastAnswer, &TaskIntent::Planning);
        assert_eq!(tracker.store.total_corrections, 2);
        assert_eq!(tracker.store.entries.len(), 1);
        assert_eq!(tracker.store.entries[0].count, 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn record_correction_distinct_inputs() {
        let path = test_path("distinct_inputs");
        let _ = std::fs::remove_file(&path);
        let mut tracker = FeedbackTracker::new_with_path("test-model", path.clone());

        tracker.record_correction("hello", &TaskIntent::FastAnswer, &TaskIntent::Planning);
        tracker.record_correction("world", &TaskIntent::FastAnswer, &TaskIntent::Planning);
        assert_eq!(tracker.store.total_corrections, 2);
        assert_eq!(tracker.store.entries.len(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn record_correction_distinct_intents() {
        let path = test_path("distinct_intents");
        let _ = std::fs::remove_file(&path);
        let mut tracker = FeedbackTracker::new_with_path("test-model", path.clone());

        tracker.record_correction("hello", &TaskIntent::FastAnswer, &TaskIntent::Planning);
        tracker.record_correction("hello", &TaskIntent::FastAnswer, &TaskIntent::CodeAction);
        assert_eq!(tracker.store.total_corrections, 2);
        assert_eq!(tracker.store.entries.len(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn flush_writes_to_disk() {
        let path = test_path("flush_write");
        let _ = std::fs::remove_file(&path);
        {
            let mut tracker = FeedbackTracker::new_with_path("test-model", path.clone());
            tracker.record_correction("test", &TaskIntent::FastAnswer, &TaskIntent::Planning);
            tracker.flush();
            assert!(!tracker.dirty);
        }
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("FastAnswer"));
        assert!(content.contains("Planning"));
        let _ = std::fs::remove_file(&path);
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
    fn five_hundred_entry_cap_enforced() {
        let path = test_path("cap_500");
        let _ = std::fs::remove_file(&path);
        let mut tracker = FeedbackTracker::new_with_path("test-model", path.clone());

        for i in 0..600 {
            let input = format!("input_{}", i);
            tracker.record_correction(&input, &TaskIntent::FastAnswer, &TaskIntent::Planning);
        }
        assert!(tracker.store.entries.len() <= 500);
        assert_eq!(tracker.store.total_corrections, 600);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn drop_calls_flush() {
        let path = test_path("drop_flush");
        let _ = std::fs::remove_file(&path);
        {
            let mut tracker = FeedbackTracker::new_with_path("test-model", path.clone());
            tracker.record_correction("drop-test", &TaskIntent::FastAnswer, &TaskIntent::Planning);
        }
        assert!(path.exists());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn new_loads_existing_data() {
        let path = test_path("load_existing");
        let _ = std::fs::remove_file(&path);
        {
            let mut tracker = FeedbackTracker::new_with_path("test-model", path.clone());
            tracker.record_correction("existing", &TaskIntent::FastAnswer, &TaskIntent::Planning);
            tracker.flush();
        }
        let tracker = FeedbackTracker::new_with_path("test-model", path.clone());
        assert_eq!(tracker.store.total_corrections, 1);
        assert_eq!(tracker.store.entries.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn new_creates_default_when_no_file() {
        let path = std::env::temp_dir().join("rem-cli-test-feedback-nonexistent.json");
        let _ = std::fs::remove_file(&path);
        let tracker = FeedbackTracker::new_with_path("fresh-model", path.clone());
        assert_eq!(tracker.store.total_corrections, 0);
        assert!(tracker.store.entries.is_empty());
        assert_eq!(tracker.store.model, "fresh-model");
        let _ = std::fs::remove_file(&path);
    }
}
