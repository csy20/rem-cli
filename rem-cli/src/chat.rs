//! Chat session management, prompt building, and context assembly.
//! Provides [`ChatSession`] which tracks conversation state, resolves `@` file
//! references, builds system prompts with project context, and manages modes.

use crate::feedback::FeedbackTracker;
use crate::indexer::{
    build_retrieved_context, load_codebase_index, retrieve_relevant_chunks, CodebaseIndex,
};
use crate::intent::TaskIntent;
use crate::memory::ProjectMemory;
use crate::search::SearchResult;
use crate::session_io::{build_project_context, detect_project_type};
use crate::{FileEntry, RE_AT_REF};
use anyhow::{Context, Result};
use rustyline::config::Configurer;
use rustyline::DefaultEditor;
use std::fs;
use std::io::{self};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Cached project file listing to avoid repeated directory walks.
struct ProjectListingCache {
    listing: String,
    dir: PathBuf,
}

impl ProjectListingCache {
    fn get_or_build(&mut self, dir: &Path, max_bytes: usize) -> &str {
        if self.dir != dir {
            self.listing = build_project_context(dir, max_bytes);
            self.dir = dir.to_path_buf();
        }
        &self.listing
    }
}

/// Holds all mutable state for an interactive chat session.
pub(crate) struct ChatSession {
    pub(crate) rl: DefaultEditor,
    pub(crate) last_code: String,
    pub(crate) last_files: Vec<FileEntry>,
    pub(crate) last_files_written: Vec<crate::BackupEntry>,
    pub(crate) undo_stack: Vec<Vec<crate::BackupEntry>>,
    pub(crate) last_search: Vec<SearchResult>,
    pub(crate) last_intent: TaskIntent,
    pub(crate) last_user_input: String,
    pub(crate) project_dir: Option<PathBuf>,
    pub(crate) workspace_dir: Option<PathBuf>,
    pub(crate) history: Vec<(String, String)>,
    pub(crate) feedback: FeedbackTracker,
    pub(crate) mode: RunMode,
    pub(crate) last_tokens: u32,
    pub(crate) last_elapsed: std::time::Duration,
    pub(crate) project_memory: ProjectMemory,
    pub(crate) messages_since_save: usize,
    pub(crate) project_type: Option<String>,
    listing_cache: Option<ProjectListingCache>,
    /// Cached codebase index to avoid reloading from disk on every turn.
    cached_index: Option<(PathBuf, CodebaseIndex)>,
}

impl ChatSession {
    fn history_path() -> PathBuf {
        dirs::home_dir()
            .map(|h| h.join(".config/rem-cli/history.txt"))
            .unwrap_or_else(|| PathBuf::from(".rem_history.txt"))
    }

    /// Creates a new chat session with the given model and optional workspace.
    pub(crate) fn new(model: &str, workspace: Option<PathBuf>) -> Result<Self> {
        let mut rl = DefaultEditor::new().context("failed to start line editor")?;
        let history_path = Self::history_path();
        let _ = rl.load_history(&history_path);
        rl.set_max_history_size(1000).ok();
        let project_dir = workspace.clone();
        let project_memory =
            ProjectMemory::load(project_dir.as_deref().unwrap_or_else(|| Path::new(".")));
        Ok(Self {
            rl,
            last_code: String::new(),
            last_files: Vec::new(),
            last_files_written: Vec::new(),
            undo_stack: Vec::new(),
            last_search: Vec::new(),
            last_intent: TaskIntent::FastAnswer,
            last_user_input: String::new(),
            project_dir: workspace.clone(),
            workspace_dir: workspace,
            history: Vec::new(),
            feedback: FeedbackTracker::new(model),
            mode: RunMode::Chat,
            last_tokens: 0,
            last_elapsed: std::time::Duration::from_secs(0),
            project_memory,
            messages_since_save: 0,
            project_type: None,
            listing_cache: None,
            cached_index: None,
        })
    }

    /// Saves the readline history to disk.
    pub(crate) fn save_history(&mut self) {
        let path = Self::history_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = self.rl.save_history(&path);
    }

    /// Reads a line from the terminal with the given prompt.
    pub(crate) fn readline(&mut self, prompt: &str) -> io::Result<String> {
        self.rl.readline(prompt).map_err(io::Error::other)
    }

    /// Adds a line to the readline history.
    pub(crate) fn add_history(&mut self, line: &str) {
        let _ = self.rl.add_history_entry(line);
    }

    /// Builds a context string from the last web search results.
    pub(crate) fn build_search_context(&self) -> String {
        if self.last_search.is_empty() {
            return String::new();
        }
        let mut ctx = String::from("Web search results:\n");
        for (i, r) in self.last_search.iter().enumerate().take(3) {
            ctx.push_str(&format!("{}. {} — {}\n", i + 1, r.title, r.snippet));
        }
        ctx
    }

    /// Query-aware project context. When a codebase_index.json exists for the project,
    /// performs keyword retrieval of relevant chunks (by content/name match against the
    /// user task) and returns a targeted "Relevant code chunks" block. This is the key
    /// mechanism for scaling rem to larger codebases without prompt explosion.
    /// Falls back to the classic exhaustive (but capped) file listing when no index.
    /// Builds project context for the given query.
    /// Uses codebase index for retrieval when available; falls back to file listing.
    /// The index is cached in memory to avoid reloading from disk on every turn.
    pub(crate) fn build_relevant_project_context(&mut self, query: &str) -> String {
        let dir = match self.project_dir.clone() {
            Some(ref d) => d.clone(),
            None => return String::new(),
        };

        // Load/reload index if dir changed or not yet cached
        let should_reload = self
            .cached_index
            .as_ref()
            .map(|(cached_dir, _)| cached_dir != &dir)
            .unwrap_or(true);
        if should_reload {
            self.cached_index = load_codebase_index(&dir).map(|i| (dir.clone(), i));
        }

        // Use cached index for BM25 retrieval
        if let Some((_, idx)) = &self.cached_index {
            let hits = retrieve_relevant_chunks(idx, query, 8, 4500);
            if !hits.is_empty() {
                return build_retrieved_context(&hits, 4800);
            }
        }

        // No index or no hits: use cached file listing
        if self.listing_cache.is_none() {
            self.listing_cache = Some(ProjectListingCache {
                listing: String::new(),
                dir: PathBuf::new(),
            });
        }
        if let Some(ref mut cache) = self.listing_cache {
            return cache.get_or_build(&dir, 6000).to_string();
        }
        String::new()
    }

    /// Builds a truncated history string from recent conversation turns.
    pub(crate) fn build_chat_history(&self) -> String {
        if self.history.is_empty() {
            return String::new();
        }
        let mut out = String::from("[Previous conversation — keep context in mind]:\n\n");
        for (user, assistant) in self.history.iter().rev().take(6).rev() {
            let truncated_assistant = crate::truncate_to_lines(assistant, 15);
            out.push_str(&format!("User: {}\nREM: {}\n\n", user, truncated_assistant));
        }
        out
    }

    /// Returns the cached project type, computing it on first call.
    pub(crate) fn get_project_type(&mut self) -> &str {
        if self.project_type.is_none() {
            let t = self
                .project_dir
                .as_deref()
                .map(detect_project_type)
                .unwrap_or("")
                .to_string();
            self.project_type = Some(t);
        }
        self.project_type.as_deref().unwrap_or("")
    }

    /// Returns the project memory as context for the LLM prompt.
    pub(crate) fn build_memory_context(&self) -> String {
        self.project_memory.as_context()
    }

    /// Returns the session data as a JSON value for persistence.
    pub(crate) fn to_session_json(&self) -> serde_json::Value {
        let last_files_json: Vec<serde_json::Value> = self
            .last_files
            .iter()
            .map(|f| serde_json::json!({"path": f.path, "content": f.content}))
            .collect();
        serde_json::json!({
            "history": self.history.iter().map(|(u, a)| serde_json::json!({"user": u, "assistant": a})).collect::<Vec<_>>(),
            "mode": self.mode.label(),
            "workspace": self.project_dir.as_ref().map(|d| d.display().to_string()),
            "saved_at": crate::format_timestamp(),
            "last_code": self.last_code,
            "last_files": last_files_json,
            "last_files_written": self.last_files_written.iter().map(|e| e.path.display().to_string()).collect::<Vec<_>>(),
        })
    }

    /// Returns the project directory, falling back to cwd.
    pub(crate) fn session_dir(&self) -> std::path::PathBuf {
        self.project_dir
            .as_ref()
            .cloned()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
    }

    /// Auto-saves the session to `.rem/session.json` every N messages.
    pub(crate) fn auto_save_session(&self) {
        let dir = self.session_dir();
        let session_file = dir.join(".rem/session.json");
        if let Err(e) = std::fs::create_dir_all(dir.join(".rem")) {
            tracing::warn!("failed to create session dir: {}", e);
        }
        if let Err(e) = std::fs::write(
            &session_file,
            serde_json::to_string_pretty(&self.to_session_json()).unwrap_or_default(),
        ) {
            tracing::warn!("failed to auto-save session: {}", e);
        }
    }

    /// Resolves `@<path>` references in user input to file contents.
    /// Returns (modified_input, extra_context).
    pub(crate) fn resolve_at_references(&self, input: &str) -> (String, String) {
        let mut extra_context = String::new();
        let mut cleaned_input = input.to_string();

        for cap in RE_AT_REF.captures_iter(input) {
            let ref_path = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            if ref_path.starts_with("http") {
                continue;
            }
            let path = if ref_path.starts_with('/') || ref_path.starts_with("~/") {
                if ref_path.starts_with("~/") {
                    if let Some(home) = dirs::home_dir() {
                        home.join(ref_path.trim_start_matches("~/"))
                    } else {
                        PathBuf::from(ref_path)
                    }
                } else {
                    PathBuf::from(ref_path)
                }
            } else {
                let base = self
                    .project_dir
                    .as_deref()
                    .unwrap_or_else(|| Path::new("."));
                match crate::resolve_safe_path(base, ref_path) {
                    Some(p) => p,
                    None => continue,
                }
            };

            if path.is_file() {
                if let Ok(content) = fs::read_to_string(&path) {
                    let truncated = crate::truncate_bytes(&content, 8000);
                    extra_context.push_str(&format!(
                        "\n[File: {}]\n{}\n[/File: {}]\n",
                        path.display(),
                        truncated,
                        path.display()
                    ));
                }
            } else if path.is_dir() {
                let mut listing = String::new();
                for e in WalkDir::new(&path)
                    .max_depth(2)
                    .sort_by_file_name()
                    .into_iter()
                    .flatten()
                {
                    if let Ok(rel) = e.path().strip_prefix(&path) {
                        let rel_str = rel.display().to_string();
                        if rel_str.is_empty() || rel_str.starts_with('.') {
                            continue;
                        }
                        if rel.components().any(|c| {
                            c.as_os_str()
                                .to_str()
                                .is_some_and(crate::find::should_skip_dir)
                        }) {
                            continue;
                        }
                        let marker = if e.file_type().is_dir() { "/" } else { "" };
                        listing.push_str(&format!("  {}{}\n", rel_str, marker));
                    }
                }
                if !listing.is_empty() {
                    let total = listing.lines().count();
                    extra_context.push_str(&format!(
                        "\n[Directory: {} ({} entries)]\n{}[/Directory: {}]\n",
                        path.display(),
                        total,
                        listing,
                        path.display()
                    ));
                }
            }

            cleaned_input = cleaned_input.replace(&format!("@{}", ref_path), ref_path);
        }

        (cleaned_input, extra_context)
    }
}

/// Chat interaction mode: Chat, Code, or Plan.
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum RunMode {
    Chat,
    Code,
    Plan,
}

impl RunMode {
    /// Cycles through Chat → Code → Plan → Chat.
    pub(crate) fn toggle(&self) -> RunMode {
        match self {
            RunMode::Chat => RunMode::Code,
            RunMode::Code => RunMode::Plan,
            RunMode::Plan => RunMode::Chat,
        }
    }

    /// Returns the display label for this mode.
    pub(crate) fn label(&self) -> &str {
        match self {
            RunMode::Chat => "CHAT",
            RunMode::Code => "CODE",
            RunMode::Plan => "PLAN",
        }
    }
}
