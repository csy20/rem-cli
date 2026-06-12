use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::TaskIntent;

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

pub struct FeedbackTracker {
    store: FeedbackStore,
    path: PathBuf,
    dirty: bool,
}

impl FeedbackTracker {
    pub fn new(model: &str) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let path = home.join(".config/rem-cli/feedback.json");
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

    pub fn record_correction(
        &mut self,
        input: &str,
        classified_as: &TaskIntent,
        correct: &TaskIntent,
    ) {
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
                self.store
                    .entries
                    .drain(0..(self.store.entries.len() - 500));
            }
        }

        self.store.total_corrections += 1;
        self.dirty = true;
    }

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
