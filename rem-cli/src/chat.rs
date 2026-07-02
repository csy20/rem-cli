//! Chat session management, prompt building, and context assembly.
//! Provides [`ChatSession`] which tracks conversation state, resolves `@` file
//! references, builds system prompts with project context, and manages modes.

use crate::feedback::FeedbackTracker;
use crate::indexer::{build_retrieved_context, load_codebase_index, retrieve_relevant_chunks, CodebaseIndex};
use crate::intent::TaskIntent;
use crate::memory::ProjectMemory;
use crate::search::SearchResult;
use crate::session_io::build_project_context;
use crate::token_count::estimate_tokens;
use crate::types::{FileEntry, RE_AT_REF};
use anyhow::{Context, Result};
use regex::Regex;
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

/// Readline and conversation history management.
pub(crate) struct HistoryManager {
    pub(crate) rl: DefaultEditor,
    pub(crate) history: Vec<(String, String)>,
    pub(crate) messages_since_save: usize,
}

impl HistoryManager {
    fn history_path() -> PathBuf {
        dirs::home_dir()
            .map(|h| h.join(".config/rem-cli/history.txt"))
            .unwrap_or_else(|| PathBuf::from(".rem_history.txt"))
    }

    pub(crate) fn new() -> Result<Self> {
        let mut rl = DefaultEditor::new().context("failed to start line editor")?;
        let history_path = Self::history_path();
        let _ = rl.load_history(&history_path);
        rl.set_max_history_size(crate::constants::MAX_HISTORY_ENTRIES).ok();
        Ok(Self {
            rl,
            history: Vec::new(),
            messages_since_save: 0,
        })
    }

    pub(crate) fn push_turn(&mut self, user: String, assistant: String) {
        self.history.push((user, assistant));
    }

    pub(crate) fn save_history(&mut self) {
        let path = Self::history_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = self.rl.save_history(&path);
    }

    pub(crate) fn readline(&mut self, prompt: &str) -> io::Result<String> {
        self.rl.readline(prompt).map_err(io::Error::other)
    }

    pub(crate) fn add_history(&mut self, line: &str) {
        let _ = self.rl.add_history_entry(line);
    }

    pub(crate) fn build_chat_history(&self) -> String {
        if self.history.is_empty() {
            return String::new();
        }
        const TOKEN_BUDGET_PER_TURN: usize = 500;
        let mut out = String::from("[Previous conversation — keep context in mind]:\n\n");
        for (user, assistant) in self.history.iter().rev().take(6).rev() {
            let truncated_assistant = if estimate_tokens(assistant) > TOKEN_BUDGET_PER_TURN {
                let estimated_len = (assistant.len() * TOKEN_BUDGET_PER_TURN) / estimate_tokens(assistant).max(1);
                let cutoff = estimated_len.min(assistant.len());
                let cutoff = (0..=cutoff).rev().find(|&i| assistant.is_char_boundary(i)).unwrap_or(0);
                let truncated = assistant[..cutoff].to_string();
                format!("{}...\n[truncated to ~{} tokens]", truncated, TOKEN_BUDGET_PER_TURN)
            } else {
                assistant.clone()
            };
            // Escape internal newlines to avoid breaking the \n\n turn separator
            let safe_user = user.replace('\n', "\\n");
            let safe_assistant = truncated_assistant.replace('\n', "\\n");
            out.push_str(&format!("User: {}\nREM: {}\n\n", safe_user, safe_assistant));
        }
        out
    }
}

/// Maximum undo stack depth to prevent unbounded memory growth.
const MAX_UNDO_DEPTH: usize = 50;

/// Tracks last generated code, files, writes, and undo stack.
pub(crate) struct CodeOutput {
    pub(crate) last_code: String,
    pub(crate) last_files: Vec<FileEntry>,
    pub(crate) last_files_written: Vec<crate::BackupEntry>,
    pub(crate) undo_stack: Vec<Vec<crate::BackupEntry>>,
}

impl CodeOutput {
    pub(crate) fn new() -> Self {
        Self {
            last_code: String::new(),
            last_files: Vec::new(),
            last_files_written: Vec::new(),
            undo_stack: Vec::new(),
        }
    }

    pub(crate) fn push_undo(&mut self, entries: Vec<crate::BackupEntry>) {
        self.undo_stack.push(entries);
        while self.undo_stack.len() > MAX_UNDO_DEPTH {
            self.undo_stack.remove(0);
        }
    }
}

/// Project directory, file listing cache, and codebase index cache.
pub(crate) struct ProjectContext {
    pub(crate) project_dir: Option<PathBuf>,
    pub(crate) workspace_dir: Option<PathBuf>,
    pub(crate) project_type: Option<String>,
    pub(crate) project_memory: ProjectMemory,
    listing_cache: Option<ProjectListingCache>,
    cached_index: Option<(PathBuf, CodebaseIndex)>,
}

impl ProjectContext {
    pub(crate) fn new(workspace: Option<PathBuf>) -> Self {
        let project_dir = workspace.clone();
        let project_memory = ProjectMemory::load(project_dir.as_deref().unwrap_or_else(|| Path::new(".")));
        Self {
            project_dir,
            workspace_dir: workspace,
            project_type: None,
            project_memory,
            listing_cache: None,
            cached_index: None,
        }
    }

    pub(crate) fn session_dir(&self) -> PathBuf {
        self.project_dir
            .as_ref()
            .cloned()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
    }

    /// Invalidates cached data when the project directory changes.
    pub(crate) fn invalidate_caches(&mut self) {
        self.listing_cache = None;
        self.cached_index = None;
        self.project_type = None;
    }

    pub(crate) fn get_project_type(&mut self) -> &str {
        if self.project_type.is_none() {
            let t = self
                .project_dir
                .as_deref()
                .map(crate::session_io::detect_project_type)
                .unwrap_or("")
                .to_string();
            self.project_type = Some(t);
        }
        self.project_type.as_deref().unwrap_or("")
    }

    pub(crate) fn build_memory_context(&self) -> String {
        self.project_memory.as_context()
    }

    pub(crate) fn build_relevant_project_context(&mut self, query: &str) -> String {
        let dir = match &self.project_dir {
            Some(d) => d.clone(),
            None => return String::new(),
        };

        let should_reload = self
            .cached_index
            .as_ref()
            .map(|(cached_dir, _)| cached_dir != &dir)
            .unwrap_or(true);
        if should_reload {
            self.cached_index = load_codebase_index(&dir).map(|mut i| {
                if i.inverted_index.is_empty() && !i.chunks.is_empty() {
                    i.inverted_index = crate::indexer::build_inverted_index(&i.chunks, &mut i.doc_freqs);
                    i.num_chunks = i.chunks.len();
                }
                (dir.clone(), i)
            });
        }

        if let Some((_, idx)) = &self.cached_index {
            let hits = retrieve_relevant_chunks(idx, query, 8, 4500);
            if !hits.is_empty() {
                return build_retrieved_context(&hits, 4800);
            }
        }

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
}

/// Holds all mutable state for an interactive chat session.
pub(crate) struct ChatSession {
    pub(crate) history_mgr: HistoryManager,
    pub(crate) code_out: CodeOutput,
    pub(crate) ctx: ProjectContext,
    pub(crate) last_search: Vec<SearchResult>,
    pub(crate) last_intent: TaskIntent,
    pub(crate) last_user_input: String,
    pub(crate) feedback: FeedbackTracker,
    pub(crate) mode: RunMode,
    pub(crate) last_tokens: u32,
    pub(crate) last_elapsed: std::time::Duration,
}

impl ChatSession {
    /// Creates a new chat session with the given model and optional workspace.
    pub(crate) fn new(model: &str, workspace: Option<PathBuf>) -> Result<Self> {
        Ok(Self {
            history_mgr: HistoryManager::new()?,
            code_out: CodeOutput::new(),
            ctx: ProjectContext::new(workspace),
            last_search: Vec::new(),
            last_intent: TaskIntent::FastAnswer,
            last_user_input: String::new(),
            feedback: FeedbackTracker::new(model),
            mode: RunMode::Chat,
            last_tokens: 0,
            last_elapsed: std::time::Duration::from_secs(0),
        })
    }

    /// Saves the readline history to disk.
    pub(crate) fn save_history(&mut self) {
        self.history_mgr.save_history();
    }

    /// Reads a line from the terminal with the given prompt.
    pub(crate) fn readline(&mut self, prompt: &str) -> io::Result<String> {
        self.history_mgr.readline(prompt)
    }

    /// Adds a line to the readline history.
    pub(crate) fn add_history(&mut self, line: &str) {
        self.history_mgr.add_history(line);
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

    /// Query-aware project context. Delegates to [`ProjectContext::build_relevant_project_context`].
    pub(crate) fn build_relevant_project_context(&mut self, query: &str) -> String {
        self.ctx.build_relevant_project_context(query)
    }

    /// Builds a truncated history string from recent conversation turns.
    pub(crate) fn build_chat_history(&self) -> String {
        self.history_mgr.build_chat_history()
    }

    /// Returns the cached project type, computing it on first call.
    pub(crate) fn get_project_type(&mut self) -> &str {
        self.ctx.get_project_type()
    }

    /// Returns the project memory as context for the LLM prompt.
    pub(crate) fn build_memory_context(&self) -> String {
        self.ctx.build_memory_context()
    }

    /// Returns the session data as a JSON value for persistence.
    pub(crate) fn to_session_json(&self) -> serde_json::Value {
        let last_files_json: Vec<serde_json::Value> = self
            .code_out
            .last_files
            .iter()
            .map(|f| serde_json::json!({"path": f.path, "content": f.content}))
            .collect();
        serde_json::json!({
            "history": self.history_mgr.history.iter().map(|(u, a)| serde_json::json!({"user": u, "assistant": a})).collect::<Vec<_>>(),
            "mode": self.mode.label(),
            "workspace": self.ctx.project_dir.as_ref().map(|d| d.display().to_string()),
            "saved_at": crate::format_timestamp(),
            "last_code": self.code_out.last_code,
            "last_files": last_files_json,
            "last_files_written": self.code_out.last_files_written.iter().map(|e| e.path.display().to_string()).collect::<Vec<_>>(),
        })
    }

    /// Returns the project directory, falling back to cwd.
    pub(crate) fn session_dir(&self) -> std::path::PathBuf {
        self.ctx.session_dir()
    }

    /// Auto-saves the session to `.rem/session.json.gz` every N messages.
    pub(crate) fn auto_save_session(&self) {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let dir = self.session_dir();
        let session_file = dir.join(".rem/session.json.gz");
        if let Err(e) = std::fs::create_dir_all(dir.join(".rem")) {
            tracing::warn!("failed to create session dir: {}", e);
        }
        let json = match serde_json::to_string_pretty(&self.to_session_json()) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("failed to serialize session: {}", e);
                return;
            }
        };
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        if let Err(e) = encoder.write_all(json.as_bytes()) {
            tracing::warn!("failed to compress session: {}", e);
            return;
        }
        let compressed = match encoder.finish() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("failed to finish session compression: {}", e);
                return;
            }
        };
        if let Err(e) = std::fs::write(&session_file, &compressed) {
            tracing::warn!("failed to auto-save session: {}", e);
        }
    }

    /// Resolves `@<path>` references in user input to file contents.
    /// Returns (modified_input, extra_context).
    pub(crate) fn resolve_at_references(&self, input: &str) -> (String, String) {
        let mut extra_context = String::new();
        let mut cleaned_input = input.to_string();

        // Collect all unique non-http refs and sort by length descending
        // to handle overlapping references correctly (e.g. @foo vs @foobar)
        let mut refs: Vec<&str> = RE_AT_REF
            .captures_iter(input)
            .filter_map(|cap| cap.get(1))
            .map(|m| m.as_str())
            .filter(|ref_path| !ref_path.starts_with("http"))
            .collect();
        refs.sort_unstable();
        refs.dedup();
        refs.sort_unstable_by_key(|a| std::cmp::Reverse(a.len()));

        for ref_path in &refs {
            let base = self.ctx.project_dir.as_deref().unwrap_or_else(|| Path::new("."));
            let path = match crate::resolve_safe_path(base, ref_path) {
                Some(p) => p,
                None => continue,
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
                        if rel
                            .components()
                            .any(|c| c.as_os_str().to_str().is_some_and(crate::find::should_skip_dir))
                        {
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

            let re = Regex::new(&format!(r"@{}", regex::escape(ref_path))).unwrap();
            cleaned_input = re.replace_all(&cleaned_input, *ref_path).to_string();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session() -> ChatSession {
        ChatSession::new("test-model", None).unwrap()
    }

    #[test]
    fn build_search_context_empty() {
        let session = make_session();
        assert!(session.build_search_context().is_empty());
    }

    #[test]
    fn build_search_context_with_results() {
        let mut session = make_session();
        session.last_search.push(SearchResult {
            title: "Rust Lang".into(),
            snippet: "The Rust programming language".into(),
            url: "https://rust-lang.org".into(),
        });
        let ctx = session.build_search_context();
        assert!(ctx.contains("Rust Lang"));
        assert!(ctx.contains("The Rust programming language"));
    }

    #[test]
    fn to_session_json_roundtrip() {
        let mut session = make_session();
        session.code_out.last_code = "fn main() {}".into();
        session.history_mgr.history.push(("hello".into(), "world".into()));
        let json = session.to_session_json();
        assert_eq!(json["mode"], "CHAT");
        assert_eq!(json["last_code"], "fn main() {}");
        assert_eq!(json["history"][0]["user"], "hello");
        assert_eq!(json["history"][0]["assistant"], "world");
    }

    #[test]
    fn get_project_type_unknown() {
        let mut session = make_session();
        assert!(session.get_project_type().is_empty());
    }

    #[test]
    fn mode_toggle_cycles_correctly() {
        assert_eq!(RunMode::Chat.toggle(), RunMode::Code);
        assert_eq!(RunMode::Code.toggle(), RunMode::Plan);
        assert_eq!(RunMode::Plan.toggle(), RunMode::Chat);
    }

    #[test]
    fn mode_label_matches_expected() {
        assert_eq!(RunMode::Chat.label(), "CHAT");
        assert_eq!(RunMode::Code.label(), "CODE");
        assert_eq!(RunMode::Plan.label(), "PLAN");
    }

    #[test]
    fn session_dir_falls_back_to_cwd() {
        let session = make_session();
        assert_eq!(session.session_dir(), std::env::current_dir().unwrap());
    }

    #[test]
    fn resolve_at_references_no_references() {
        let session = make_session();
        let (cleaned, extra) = session.resolve_at_references("hello world");
        assert_eq!(cleaned, "hello world");
        assert!(extra.is_empty());
    }

    #[test]
    fn resolve_at_references_ignores_http() {
        let session = make_session();
        let (cleaned, extra) = session.resolve_at_references("see @https://example.com/doc for details");
        assert_eq!(cleaned, "see @https://example.com/doc for details");
        assert!(extra.is_empty());
    }
}
