use crate::feedback::FeedbackTracker;
use crate::indexer::{build_retrieved_context, load_codebase_index, retrieve_relevant_chunks};
use crate::intent::TaskIntent;
use crate::memory::ProjectMemory;
use crate::parsing::strip_code_blocks;
use crate::provider::{Provider, ProviderKind};
use crate::search::SearchResult;
use crate::ui;
use crate::{FileEntry, RE_AT_REF};
use anyhow::{Context, Result};
use rustyline::DefaultEditor;
use std::fs;
use std::io::{self};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub(crate) struct ChatSession {
    pub(crate) rl: DefaultEditor,
    pub(crate) last_code: String,
    pub(crate) last_files: Vec<FileEntry>,
    pub(crate) last_files_written: Vec<PathBuf>,
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
}

impl ChatSession {
    pub(crate) fn new(model: &str, workspace: Option<PathBuf>) -> Result<Self> {
        let rl = DefaultEditor::new().context("failed to start line editor")?;
        let project_dir = workspace.clone();
        let project_memory =
            ProjectMemory::load(project_dir.as_deref().unwrap_or_else(|| Path::new(".")));
        Ok(Self {
            rl,
            last_code: String::new(),
            last_files: Vec::new(),
            last_files_written: Vec::new(),
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
        })
    }

    pub(crate) fn readline(&mut self, prompt: &str) -> io::Result<String> {
        self.rl.readline(prompt).map_err(io::Error::other)
    }

    pub(crate) fn add_history(&mut self, line: &str) {
        let _ = self.rl.add_history_entry(line);
    }

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
    pub(crate) fn build_relevant_project_context(&self, query: &str) -> String {
        if let Some(ref dir) = self.project_dir {
            if let Some(index) = load_codebase_index(dir) {
                // Pull more candidates, let the injector cap by chars
                let hits = retrieve_relevant_chunks(&index, query, 8, 4500);
                if !hits.is_empty() {
                    return build_retrieved_context(&hits, 4800);
                }
                // Index existed but nothing matched the query — still better to give a small
                // generic listing than nothing, or fall through.
            }
            // No index or no hits: classic behavior (names + sizes, depth-limited)
            build_project_context(dir, 6000)
        } else {
            String::new()
        }
    }

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

    pub(crate) fn build_memory_context(&self) -> String {
        self.project_memory.as_context()
    }

    pub(crate) fn resolve_at_references(&self, input: &str) -> (String, String) {
        let mut extra_context = String::new();
        let mut cleaned_input = input.to_string();

        for cap in RE_AT_REF.captures_iter(input) {
            let ref_path = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            if ref_path.starts_with("http") {
                continue;
            }
            let path = if ref_path.starts_with('/') || ref_path.starts_with("~/") {
                let resolved = if ref_path.starts_with("~/") {
                    if let Some(home) = dirs::home_dir() {
                        home.join(ref_path.trim_start_matches("~/"))
                    } else {
                        PathBuf::from(ref_path)
                    }
                } else {
                    PathBuf::from(ref_path)
                };
                resolved
            } else {
                let base = self
                    .project_dir
                    .as_deref()
                    .unwrap_or_else(|| Path::new("."));
                base.join(ref_path)
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

pub(crate) fn check_system_resources() {
    let t = ui::theme::active();
    let mem_gb = detect_system_ram_gb();
    if mem_gb > 0 && mem_gb <= 16 {
        eprintln!(
            "{} {} GB RAM detected — Ollama may be slow on CPU.",
            ui::theme::paint_warning(&t, "\u{258C} system:"),
            mem_gb
        );
        eprintln!(
            "{} Try:  OLLAMA_NUM_PARALLEL=1 OLLAMA_MAX_LOADED_MODELS=1 ollama serve",
            ui::theme::paint_rail_empty(&t)
        );
        eprintln!();
    }
}

fn detect_system_ram_gb() -> u64 {
    if let Ok(content) = fs::read_to_string("/proc/meminfo") {
        for line in content.lines() {
            if line.starts_with("MemTotal:") {
                let kb: u64 = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                return kb / 1024 / 1024;
            }
        }
    }
    0
}

pub(crate) fn print_welcome(client: &Provider) {
    println!();
    ui::header::render(&client.provider_label(), "CHAT");
    println!();
}

pub(crate) fn build_project_context(dir: &Path, max_bytes: usize) -> String {
    let mut out = String::from("Project files:\n");
    let mut count = 0u32;
    let max_depth = 4;

    let mut entries: Vec<String> = Vec::new();
    for entry in WalkDir::new(dir)
        .max_depth(max_depth as usize)
        .sort_by_file_name()
    {
        let Ok(entry) = entry else { continue };
        let p = entry.path();
        let Ok(rel) = p.strip_prefix(dir) else {
            continue;
        };
        let rel_str = rel.display().to_string();
        if rel_str.is_empty() {
            continue;
        }
        if rel_str.starts_with('.') && rel_str != "." {
            continue;
        }
        if rel_str.contains("venv")
            || rel_str.contains("dist")
            || rel_str.contains(".pytest_cache")
            || rel.components().any(|c| {
                c.as_os_str()
                    .to_str()
                    .is_some_and(crate::find::should_skip_dir)
            })
        {
            continue;
        }

        if p.is_dir() {
            if rel.components().count() >= 3 {
                continue;
            }
            entries.push(format!("{}/", rel_str));
        } else {
            let size = p.metadata().map(|m| m.len()).unwrap_or(0);
            entries.push(format!("{}  ({} bytes)", rel_str, size));
        }
        count += 1;
        if out.len() > max_bytes {
            break;
        }
    }

    if count > 0 {
        out.push_str(&entries.join("\n"));
        out.push_str("\n\n");
        out
    } else {
        String::new()
    }
}

pub(crate) fn detect_project_type(dir: &Path) -> &'static str {
    if !dir.exists() {
        return "";
    }
    let entries: Vec<String> = WalkDir::new(dir)
        .max_depth(1)
        .into_iter()
        .filter_map(|e: Result<walkdir::DirEntry, walkdir::Error>| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.file_name().to_string_lossy().to_lowercase())
        .collect();

    let has_file = |name: &str| entries.iter().any(|f| f == name);

    if has_file("Cargo.toml") {
        return "rust";
    }
    if has_file("go.mod") {
        return "go";
    }
    if has_file("pyproject.toml") || has_file("setup.py") || has_file("requirements.txt") {
        return "python";
    }
    if has_file("package.json") {
        return "javascript";
    }
    if has_file("index.html") && has_file("style.css") {
        return "html_css";
    }
    if has_file("dart.yaml") || has_file("pubspec.yaml") {
        return "dart";
    }
    if has_file("Makefile") {
        return "cpp";
    }
    ""
}

pub(crate) fn language_specific_guidance(project_type: &str) -> &'static str {
    match project_type {
        "rust" => "\nLanguage context: Rust project. Use cargo build/run. Prefer &str over String where possible. Include Cargo.toml deps.",
        "go" => "\nLanguage context: Go project. Use go mod tidy. Follow standard library patterns.",
        "python" => "\nLanguage context: Python project. Use pip install for deps. Follow PEP 8. Use type hints.",
        "javascript" => "\nLanguage context: JavaScript/Node.js project. Use npm/yarn. Prefer ES modules. Include package.json deps.",
        "html_css" => "\nLanguage context: HTML/CSS project. Use semantic HTML. Responsive CSS with modern layout (flexbox/grid).",
        "dart" => "\nLanguage context: Dart/Flutter project. Use pub get for deps. Follow effective Dart guidelines.",
        "cpp" => "\nLanguage context: C/C++ project. Use make/gcc. Show compilation commands.",
        _ => "",
    }
}

pub(crate) fn build_prompt(session: &ChatSession, client: &Provider) -> String {
    let t = ui::theme::active();
    let model_short = client.model.split(':').next().unwrap_or(&client.model);
    let mode_key = ui::theme::accent_for_mode(session.mode.label());
    let provider_prefix = match client.kind {
        ProviderKind::Ollama => "",
        _ => client.kind.as_str(),
    };
    let mut p = String::new();
    p.push('\x01');
    p.push_str(t.fg(mode_key));
    p.push('\x02');
    p.push('[');
    p.push_str(session.mode.label());
    p.push(']');
    p.push_str("\x01\x1b[0m\x02");
    p.push(' ');
    p.push('\x01');
    p.push_str(t.fg("accent"));
    p.push('\x02');
    if !provider_prefix.is_empty() {
        p.push_str(provider_prefix);
        p.push('/');
    }
    p.push_str(model_short);
    p.push('>');
    p.push_str("\x01\x1b[0m\x02");
    p.push(' ');
    p
}

pub(crate) fn validate_chat_response(
    response: &str,
    intent: &TaskIntent,
    mode: &RunMode,
) -> (bool, String) {
    if *intent != TaskIntent::CodeAction && *mode != RunMode::Code {
        let has_code_fences = response.contains("```");
        let has_multi_file = response.contains("### ") && has_code_fences;
        let has_json = response.trim().starts_with('{')
            && (response.contains("\"code\"") || response.contains("\"files\""));

        if has_multi_file || has_json {
            let code_stripped = strip_code_blocks(response);
            if !code_stripped.trim().is_empty() {
                return (true, code_stripped);
            }
            return (
                true,
                "I understood your question. Let me answer directly: ".to_string(),
            );
        }
    }

    if response.trim().is_empty() {
        return (
            true,
            "(No response generated — please try again or rephrase)".to_string(),
        );
    }

    (false, String::new())
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum RunMode {
    Chat,
    Code,
    Plan,
}

impl RunMode {
    pub(crate) fn toggle(&self) -> RunMode {
        match self {
            RunMode::Chat => RunMode::Code,
            RunMode::Code => RunMode::Plan,
            RunMode::Plan => RunMode::Chat,
        }
    }

    pub(crate) fn label(&self) -> &str {
        match self {
            RunMode::Chat => "CHAT",
            RunMode::Code => "CODE",
            RunMode::Plan => "PLAN",
        }
    }
}
