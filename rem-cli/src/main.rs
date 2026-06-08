use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::sync::LazyLock;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand};
use regex::Regex;
use reqwest::Client;
use rustyline::DefaultEditor;
use serde::{Deserialize, Serialize};
use serde_json::json;
use walkdir::WalkDir;

mod agentic;
mod commands;
mod config;
mod feedback;
mod find;
mod indexer;
mod intent;
mod memory;
mod parsing;
mod provider;
mod ui;

use agentic::{build_agentic_prompt, build_tool_context, extract_goal_signal, run_lint, run_test};
use feedback::FeedbackTracker;
use find::{find_matches, FindOptions};
use indexer::{
    build_retrieved_context, generate_codebase_index, load_codebase_index,
    retrieve_relevant_chunks, write_codebase_index,
};
use intent::{classify_intent, has_creation_intent, has_file_path, intent_instruction, TaskIntent};
use memory::ProjectMemory;
use parsing::{current_name_from_bold, extract_code_block, guess_filename, strip_code_blocks};
use provider::{api_url, OllamaJsonResponse, Provider};

static CTRL_C_COUNT: AtomicU8 = AtomicU8::new(0);

fn setup_global_ctrlc_handler() {
    let _handle = tokio::spawn(async {
        loop {
            let _ = tokio::signal::ctrl_c().await;
            let count = CTRL_C_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
            if count >= 2 {
                eprintln!("\n  \u{00d7} exiting (Ctrl+C pressed twice)");
                std::process::exit(0);
            }
        }
    });
}

fn reset_ctrlc_count() {
    CTRL_C_COUNT.store(0, Ordering::SeqCst);
}

// ── ANSI styling ───────────────────────────────────────────────────────────

pub const C_RESET: &str = "\x1b[0m";
pub const C_BOLD: &str = "\x1b[1m";
pub const C_DIM: &str = "\x1b[2m";
pub const C_CYAN: &str = "\x1b[36m";
pub const C_GREEN: &str = "\x1b[32m";
pub const C_YELLOW: &str = "\x1b[33m";
pub const C_RED: &str = "\x1b[31m";
pub const C_MAGENTA: &str = "\x1b[35m";
pub const C_BLUE: &str = "\x1b[34m";
pub const C_WHITE_B: &str = "\x1b[1;37m";

#[macro_export]
macro_rules! style {
    ($color:ident, $($arg:tt)*) => {
        format!("{}{}{}", $color, format!($($arg)*), $crate::C_RESET)
    };
}

// ── Lazy-compiled regexes ──────────────────────────────────────────────────

static RE_AT_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@([^\s]+)").expect("invalid regex literal"));
static RE_HTML_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<[^>]*>").expect("invalid regex literal"));
static RE_AMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&amp;").expect("invalid regex literal"));
static RE_LT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&lt;").expect("invalid regex literal"));
static RE_GT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&gt;").expect("invalid regex literal"));
static RE_QUOT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&quot;").expect("invalid regex literal"));
static RE_APOS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&#x27;").expect("invalid regex literal"));
static RE_SEARCH_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"class="result__a"[^>]*href="([^"]*)"[^>]*>([^<]*)</a>"#)
        .expect("invalid regex literal")
});
static RE_SEARCH_SNIPPET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"class="result__snippet"[^>]*>([^<]*(?:<[^/>][^>]*>[^<]*</[^>]+>)*[^<]*)</a>"#)
        .expect("invalid regex literal")
});
static RE_HIGHLIGHT_HTML_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(</?\w+[^>]*>)").expect("invalid regex literal"));
static RE_HIGHLIGHT_ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"("[^"]*")"#).expect("invalid regex literal"));
static RE_HIGHLIGHT_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(<!--.*?-->)").expect("invalid regex literal"));
static RE_CSS_PROP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^(\s*)([a-zA-Z-]+)(\s*:)").expect("invalid regex literal"));
static RE_CSS_VAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(:\s*)([^;}{]+)").expect("invalid regex literal"));
static RE_CSS_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(/\*.*?\*/)").expect("invalid regex literal"));
static RE_JS_KW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(const|let|var|function|return|if|else|for|while|class|import|export|from|async|await|try|catch|new|this|document|console|window)\b").expect("invalid regex literal")
});
static RE_JS_STR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"('[^']*'|"[^"]*"|`[^`]*`)"#).expect("invalid regex literal"));
static RE_JS_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(//.*)").expect("invalid regex literal"));

// ── Config & Prompts ───────────────────────────────────────────────────────

const DEFAULT_SYSTEM_PROMPT: &str = r##"You are REM, a helpful coding assistant for developers of all levels.

You can chat conversationally OR generate code/files — choose the right mode based on what the user is asking for.

CHAT mode (default):
- User is asking a question, explaining something, greeting you, or having a conversation.
- Reply with a clear, direct text or markdown answer.
- NO code generation, NO file creation, NO JSON. Just answer the question.
- If the user might want code but it's not explicit, ask first: "Would you like me to write code for that?"

CODE mode:
- User has explicitly asked you to create, build, generate, scaffold, fix, refactor, or modify code/files.
- Generate complete, runnable files with clear file paths.
- Use the [MODE: CODE] marker at the start of your response when generating code.
"##;

const CHAT_SYSTEM_PROMPT_CONVERSATIONAL: &str = r##"You are REM, a helpful coding assistant in conversation mode.

[MODE: CHAT]
RULES — follow strictly:
1. The user is chatting, asking a question, greeting you, or making conversation.
2. Reply with a clear, direct text or markdown answer. BE CONCISE.
3. NO code generation. NO file creation. NO multi-file format. NO JSON.
4. If the user might want code but didn't explicitly ask, ASK FIRST: "Would you like me to write code for that?"
5. If the user asks "how would you...", "what's the best way...", "should I use X or Y" — give a plan with trade-offs, but NO code.
6. If you need current info (versions, docs), briefly suggest: "/search <query>". Never guess.
7. Keep it short. The user is a developer.
"##;

const CHAT_SYSTEM_PROMPT_CODE: &str = r##"You are REM, a coding assistant in code generation mode.

[MODE: CODE]
RULES — follow strictly:
1. The user explicitly asked for code. Generate complete, runnable files.
2. First, give a 1-line summary of what you'll create.
3. Then output files using the multi-file format below.
4. Keep explanations minimal. Focus on working code.

=== MULTI-FILE FORMAT ===
Each file MUST have its own ### heading with the full path, then a code fence.

### path/to/file.html
```html
<file content here>
```

### path/to/file.css
```css
<file content here>
```

Always provide complete, runnable code. Do NOT use JSON format — use the multi-file format above.
"##;

const CHAT_SYSTEM_PROMPT_PLAN: &str = r##"You are REM, a coding assistant in planning mode.

[MODE: PLAN]
RULES — follow strictly:
1. The user wants a strategic plan before any code is written.
2. FIRST: analyze the request and context. What needs to be built/fixed?
3. SECOND: explore the codebase — mention relevant files and patterns you see.
4. THIRD: propose an approach with alternatives and trade-offs.
5. FOURTH: recommend a concrete next step.
6. DO NOT generate any code. DO NOT output files. NO code fences. NO JSON.
7. Respond in clear markdown sections: ## Analysis, ## Approach, ## Trade-offs, ## Recommendation.
8. End with: "Should I proceed with this plan? Type /mode to switch to CODE when ready."
"##;

const BLOCKED_COMMAND_PATTERNS: [&str; 10] = [
    "rm -rf /",
    "rm -rf",
    "rm  -rf",
    "mkfs",
    "dd if=",
    ":(){:|:&};:",
    "shutdown",
    "reboot",
    "curl ",
    "sudo ",
];

// ── CLI definition ─────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "rem",
    version,
    about = "REM — Coding assistant CLI. Run `rem` to start interactive chat. Type /mode to toggle CHAT ↔ CODE ↔ PLAN.",
    long_about = None,
)]
struct Cli {
    #[arg(long, global = true, help = "Ollama model name")]
    model: Option<String>,
    #[arg(long, global = true, help = "Ollama API URL")]
    ollama_url: Option<String>,
    #[arg(long, global = true, help = "Provider: ollama (default), openai, vllm")]
    provider: Option<String>,
    #[arg(long, global = true, help = "API key for OpenAI-compatible providers")]
    api_key: Option<String>,
    #[arg(
        long,
        short = 'v',
        global = true,
        help = "Verbose output (show raw model responses)"
    )]
    verbose: bool,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    #[command(about = "Ask REM a coding question (single-shot)")]
    Ask(AskArgs),
    #[command(about = "Explain a terminal command safely")]
    Explain(ExplainArgs),
    #[command(about = "Preview a patch for a file")]
    Patch(PatchArgs),
    #[command(about = "Scaffold a new project with templates")]
    New(NewArgs),
    #[command(
        about = "Generate or refresh the codebase index (for retrieval in large projects). Pure Rust; writes .rem/codebase_index.json so chat/goal can inject only relevant chunks instead of full file listings."
    )]
    Index(IndexArgs),
}

#[derive(Args, Debug)]
struct AskArgs {
    #[arg(help = "Your coding question")]
    prompt: String,
    #[arg(long, help = "Optional file for context")]
    file: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct ExplainArgs {
    #[arg(help = "Terminal command to explain")]
    command: String,
}

#[derive(Args, Debug)]
struct PatchArgs {
    #[arg(long, help = "Target file to patch")]
    file: PathBuf,
    #[arg(long, help = "Description of changes needed")]
    task: String,
}

#[derive(Args, Debug)]
struct NewArgs {
    #[arg(help = "Project name / directory path")]
    name: String,
    #[arg(
        long,
        default_value = "bare",
        help = "Project type: bare, portfolio, landing, blog"
    )]
    project_type: String,
}

#[derive(Args, Debug)]
struct IndexArgs {
    #[arg(help = "Project directory to index (defaults to current workspace or .)")]
    dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    model: String,
    ollama_url: String,
    timeout_s: u64,
    max_context_bytes: usize,
    /// Max context (num_ctx) passed to the LLM at inference time.
    /// Raising this (e.g. 4096-8192) is a key part of scaling to larger projects + retrieved code chunks.
    /// Must be supported by the base model (Qwen2.5-Coder 1.5B supports 32k+).
    model_ctx: usize,
    prompts_dir: Option<String>,
    #[serde(default)]
    workspace_dir: Option<String>,
    #[serde(default = "default_provider")]
    provider: String,
    #[serde(default)]
    api_url: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
}

fn default_provider() -> String {
    "ollama".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            model: "rem-coder:latest".to_string(),
            ollama_url: "http://localhost:11434".to_string(),
            timeout_s: 120,
            max_context_bytes: 16_000,
            // Start at 4096 for scaling (allows room for memory + retrieved chunks + history).
            // Previously hardcoded 2048 in provider payloads (the real limit the model sees).
            model_ctx: 4096,
            prompts_dir: None,
            workspace_dir: None,
            provider: "ollama".to_string(),
            api_url: None,
            api_key: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct PartialConfig {
    model: Option<String>,
    ollama_url: Option<String>,
    timeout_s: Option<u64>,
    max_context_bytes: Option<usize>,
    /// LLM inference context window (num_ctx). Higher values enable scaling via retrieved code + memory.
    model_ctx: Option<usize>,
    prompts_dir: Option<String>,
    workspace_dir: Option<String>,
    provider: Option<String>,
    api_url: Option<String>,
    api_key: Option<String>,
}

impl AppConfig {
    fn apply_partial(&mut self, part: PartialConfig) {
        if let Some(v) = part.model {
            self.model = v;
        }
        if let Some(v) = part.ollama_url {
            self.ollama_url = v;
        }
        if let Some(v) = part.timeout_s {
            self.timeout_s = v;
        }
        if let Some(v) = part.max_context_bytes {
            self.max_context_bytes = v;
        }
        if let Some(v) = part.model_ctx {
            self.model_ctx = v;
        }
        if let Some(v) = part.prompts_dir {
            self.prompts_dir = Some(v);
        }
        if let Some(v) = part.workspace_dir {
            self.workspace_dir = Some(v);
        }
        if let Some(v) = part.provider {
            self.provider = v;
        }
        if let Some(v) = part.api_url {
            self.api_url = Some(v);
        }
        if let Some(v) = part.api_key {
            self.api_key = Some(v);
        }
    }
}

fn save_config(cfg: &AppConfig) -> Result<()> {
    if let Some(home) = dirs::home_dir() {
        let dir = home.join(".config/rem-cli");
        fs::create_dir_all(&dir)?;
        let path = dir.join("config.toml");
        let text = toml::to_string_pretty(cfg).context("failed to serialize config")?;
        fs::write(&path, text).context("failed to write config")?;
    }
    Ok(())
}

fn first_run_setup(cfg: &mut AppConfig) -> Result<Option<PathBuf>> {
    println!();
    println!(
        "{} {}",
        style!(C_BOLD, "Welcome to REM!"),
        style!(C_DIM, "first-time setup")
    );
    println!();
    println!("{}", style!(C_CYAN, "│"));
    println!(
        "{} {}{}",
        style!(C_CYAN, "│"),
        style!(C_WHITE_B, "Where should REM create your projects?"),
        style!(C_DIM, "")
    );
    println!(
        "{} {} e.g. {} or {}",
        style!(C_CYAN, "│"),
        style!(C_DIM, ""),
        style!(C_WHITE_B, "~/projects"),
        style!(C_WHITE_B, "/home/you/code")
    );
    println!(
        "{} {} type {} for current dir, or a full path",
        style!(C_CYAN, "│"),
        style!(C_DIM, ""),
        style!(C_WHITE_B, ".")
    );
    println!("{}", style!(C_CYAN, "│"));
    print!("{}", style!(C_CYAN, "│  rem> "));
    let _ = io::stdout().flush();

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();

    let dir = if trimmed.is_empty() || trimmed == "." {
        std::env::current_dir().unwrap_or_default()
    } else if trimmed.starts_with("~/") || trimmed == "~" {
        if let Some(home) = dirs::home_dir() {
            home.join(trimmed.trim_start_matches("~/"))
        } else {
            PathBuf::from(trimmed)
        }
    } else {
        PathBuf::from(trimmed)
    };

    if !dir.exists() {
        println!("{} creating {}...", style!(C_CYAN, "│"), dir.display());
        fs::create_dir_all(&dir)?;
    }

    cfg.workspace_dir = Some(dir.to_string_lossy().to_string());
    save_config(cfg)?;

    println!(
        "{} {} workspace saved to {}",
        style!(C_GREEN, "│  ✓"),
        style!(C_DIM, ""),
        style!(C_WHITE_B, "{}", dir.display())
    );
    println!(
        "{} {} change it anytime with {}",
        style!(C_CYAN, "│"),
        style!(C_DIM, ""),
        style!(C_WHITE_B, "/dir <path>")
    );
    println!();

    Ok(Some(dir))
}

// ── Model reply schema ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize, Clone)]
struct FileEntry {
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ModelReply {
    #[serde(default)]
    explanation: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    files: Vec<FileEntry>,
    #[serde(default)]
    commands: Vec<String>,
    #[serde(default)]
    checks: Vec<String>,
    #[serde(default)]
    caution: String,
}

impl ModelReply {
    fn fallback(raw_text: &str) -> Self {
        let mut commands = Vec::new();
        for line in raw_text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('$') {
                commands.push(trimmed.trim_start_matches('$').trim().to_string());
            } else if looks_like_shell_command(trimmed) {
                commands.push(trimmed.to_string());
            }
        }
        let files = extract_code_blocks_with_names(raw_text);
        let single_code = extract_code_block(raw_text);
        Self {
            explanation: raw_text.trim().to_string(),
            code: single_code,
            files,
            commands,
            checks: vec!["Verify each step before running.".to_string()],
            caution: "Model returned non-JSON output. Review everything carefully.".to_string(),
        }
    }
}

fn extract_code_blocks_with_names(text: &str) -> Vec<FileEntry> {
    let mut files = Vec::new();
    let mut current_name = String::new();
    let mut in_fence = false;
    let mut code_lines: Vec<&str> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("```") {
            if in_fence {
                let content = code_lines.join("\n");
                if !content.trim().is_empty() {
                    let path = if current_name.is_empty() {
                        guess_filename(&code_lines)
                    } else {
                        current_name.clone()
                    };
                    files.push(FileEntry { path, content });
                }
                code_lines.clear();
                current_name.clear();
                in_fence = false;
            } else {
                in_fence = true;
            }
            continue;
        }

        if in_fence {
            code_lines.push(line);
            continue;
        }

        if let Some(name) = trimmed
            .strip_prefix("### ")
            .or_else(|| trimmed.strip_prefix("## "))
        {
            current_name = name.trim().to_string();
            continue;
        }

        if let Some(name) = current_name_from_bold(trimmed) {
            current_name = name;
            continue;
        }
    }

    if in_fence && !code_lines.is_empty() {
        let content = code_lines.join("\n");
        if !content.trim().is_empty() {
            let path = if current_name.is_empty() {
                guess_filename(&code_lines)
            } else {
                current_name.clone()
            };
            files.push(FileEntry { path, content });
        }
    }

    files
}

fn resolve_safe_path(base: &Path, rel: &str) -> Option<PathBuf> {
    let expanded = if rel.starts_with('~') {
        if let Some(home) = dirs::home_dir() {
            if rel == "~" || rel.starts_with("~/") {
                home.join(rel.trim_start_matches("~/"))
            } else {
                PathBuf::from(rel)
            }
        } else {
            PathBuf::from(rel)
        }
    } else {
        PathBuf::from(rel)
    };

    let candidate = if expanded.is_relative() {
        base.join(&expanded)
    } else {
        expanded
    };

    let resolved = match std::fs::canonicalize(&candidate) {
        Ok(r) => r,
        Err(_) => {
            let parent = candidate.parent()?;
            let canonical_parent = std::fs::canonicalize(parent).ok()?;
            canonical_parent.join(candidate.file_name()?)
        }
    };

    if resolved.starts_with(base) {
        Some(resolved)
    } else {
        eprintln!(
            "  {} path traversal blocked: {}",
            style!(C_RED, "\u{2717}"),
            style!(C_YELLOW, "{}", rel)
        );
        None
    }
}

// ── Model reply schema ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct SearchResult {
    title: String,
    snippet: String,
    url: String,
}

async fn perform_web_search(client: &Client, query: &str) -> Result<Vec<SearchResult>> {
    let resp = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .header("User-Agent", "rem-cli/0.2")
        .send()
        .await
        .context("web search request failed")?;
    let html = resp
        .text()
        .await
        .context("failed to read search response")?;
    Ok(parse_ddg_html(&html))
}

fn parse_ddg_html(html: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut remaining = html;
    while results.len() < 8 {
        if let Some(cap) = RE_SEARCH_TITLE.captures(remaining) {
            let url = cap
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            let title = cap
                .get(2)
                .map(|m| strip_html(m.as_str()))
                .unwrap_or_default();
            let snippet_pos = cap.get(0).map(|m| m.end()).unwrap_or(0);
            let after_title = &remaining[snippet_pos..];
            let snippet = RE_SEARCH_SNIPPET
                .captures(after_title)
                .and_then(|c| c.get(1))
                .map(|m| strip_html(m.as_str()).trim().to_string())
                .unwrap_or_default();
            if !title.is_empty() {
                results.push(SearchResult {
                    title,
                    snippet,
                    url,
                });
            }
            let advance = cap.get(0).map(|m| m.end()).unwrap_or(1);
            if advance >= remaining.len() {
                break;
            }
            remaining = &remaining[advance..];
        } else {
            break;
        }
    }
    results
}

fn strip_html(input: &str) -> String {
    let mut s = RE_HTML_TAG.replace_all(input, "").to_string();
    s = RE_AMP.replace_all(&s, "&").to_string();
    s = RE_LT.replace_all(&s, "<").to_string();
    s = RE_GT.replace_all(&s, ">").to_string();
    s = RE_QUOT.replace_all(&s, "\"").to_string();
    s = RE_APOS.replace_all(&s, "'").to_string();
    s.trim().to_string()
}

fn print_search_results(results: &[SearchResult]) {
    if results.is_empty() {
        println!("{}", style!(C_YELLOW, "  no results found"));
        return;
    }
    println!("{}", style!(C_DIM, "│"));
    for (i, r) in results.iter().enumerate() {
        println!(
            "{} {}",
            style!(C_CYAN, "│"),
            style!(C_WHITE_B, "{}. {}", i + 1, r.title)
        );
        println!("{}   {}", style!(C_CYAN, "│"), style!(C_DIM, "{}", r.url));
        if !r.snippet.is_empty() {
            println!("{}   {}", style!(C_CYAN, "│"), r.snippet);
        }
        println!("{}", style!(C_CYAN, "│"));
    }
}

// ── Chat session state ─────────────────────────────────────────────────────

struct ChatSession {
    rl: DefaultEditor,
    last_code: String,
    last_files: Vec<FileEntry>,
    last_files_written: Vec<PathBuf>,
    last_search: Vec<SearchResult>,
    last_intent: TaskIntent,
    last_user_input: String,
    project_dir: Option<PathBuf>,
    workspace_dir: Option<PathBuf>,
    history: Vec<(String, String)>,
    feedback: FeedbackTracker,
    mode: RunMode,
    last_tokens: u32,
    last_elapsed: std::time::Duration,
    project_memory: ProjectMemory,
}

impl ChatSession {
    fn new(model: &str, workspace: Option<PathBuf>) -> Result<Self> {
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

    fn readline(&mut self, prompt: &str) -> io::Result<String> {
        self.rl.readline(prompt).map_err(io::Error::other)
    }

    fn add_history(&mut self, line: &str) {
        let _ = self.rl.add_history_entry(line);
    }

    fn build_search_context(&self) -> String {
        if self.last_search.is_empty() {
            return String::new();
        }
        let mut ctx = String::from("Web search results:\n");
        for (i, r) in self.last_search.iter().enumerate().take(3) {
            ctx.push_str(&format!("{}. {} — {}\n", i + 1, r.title, r.snippet));
        }
        ctx
    }

    #[allow(dead_code)]
    fn build_project_context(&self) -> String {
        if let Some(ref dir) = self.project_dir {
            build_project_context(dir, 6000)
        } else {
            String::new()
        }
    }

    /// Query-aware project context. When a codebase_index.json exists for the project,
    /// performs keyword retrieval of relevant chunks (by content/name match against the
    /// user task) and returns a targeted "Relevant code chunks" block. This is the key
    /// mechanism for scaling rem to larger codebases without prompt explosion.
    /// Falls back to the classic exhaustive (but capped) file listing when no index.
    fn build_relevant_project_context(&self, query: &str) -> String {
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

    fn build_chat_history(&self) -> String {
        if self.history.is_empty() {
            return String::new();
        }
        let mut out = String::from("[Previous conversation — keep context in mind]:\n\n");
        for (user, assistant) in self.history.iter().rev().take(6).rev() {
            let truncated_assistant = truncate_to_lines(assistant, 15);
            out.push_str(&format!("User: {}\nREM: {}\n\n", user, truncated_assistant));
        }
        out
    }

    fn build_memory_context(&self) -> String {
        self.project_memory.as_context()
    }

    fn resolve_at_references(&self, input: &str) -> (String, String) {
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
                    let truncated = truncate_bytes(&content, 8000);
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
                        if rel_str.contains("node_modules")
                            || rel_str.contains("target")
                            || rel_str.contains("__pycache__")
                            || rel_str.contains(".git")
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

            cleaned_input = cleaned_input.replace(&format!("@{}", ref_path), ref_path);
        }

        (cleaned_input, extra_context)
    }
}

fn truncate_to_lines(s: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = s.lines().take(max_lines).collect();
    let mut result = lines.join("\n");
    if s.lines().count() > max_lines {
        result.push_str("\n...[truncated]");
    }
    result
}

// ── Spinner ────────────────────────────────────────────────────────────────

struct SpinnerGuard {
    running: Arc<AtomicBool>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl SpinnerGuard {
    fn new(msg: &'static str) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();
        let handle = tokio::spawn(async move {
            let chars = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut i = 0;
            while r.load(Ordering::Relaxed) {
                eprint!(
                    "\r  {} {}",
                    style!(C_CYAN, "{}", chars[i]),
                    style!(C_DIM, "{}", msg)
                );
                let _ = io::stderr().flush();
                tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                i = (i + 1) % chars.len();
            }
        });
        Self {
            running,
            handle: Some(handle),
        }
    }

    fn cancel(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

impl Drop for SpinnerGuard {
    fn drop(&mut self) {
        self.cancel();
        eprint!("\r{}\r", " ".repeat(60));
        let _ = io::stderr().flush();
    }
}

fn check_system_resources() {
    let mem_gb = detect_system_ram_gb();
    if mem_gb > 0 && mem_gb <= 16 {
        eprintln!(
            "{} {} GB RAM detected — Ollama may be slow on CPU.",
            style!(C_YELLOW, "│ system:"),
            mem_gb
        );
        eprintln!(
            "{} Try:  OLLAMA_NUM_PARALLEL=1 OLLAMA_MAX_LOADED_MODELS=1 ollama serve",
            style!(C_DIM, "│")
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

// ── Entry point ────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    setup_global_ctrlc_handler();

    let cli = Cli::parse();
    let verbose = cli.verbose;

    let mut cfg = load_config().unwrap_or_default();
    if let Some(m) = cli.model {
        cfg.model = m;
    }
    if let Some(url) = cli.ollama_url {
        cfg.ollama_url = url;
    }
    if let Some(p) = cli.provider {
        cfg.provider = p;
    }
    if let Some(k) = cli.api_key {
        cfg.api_key = Some(k);
    }

    if let Some(Commands::New(args)) = cli.command {
        return run_new(args, &cfg);
    }
    if let Some(Commands::Index(args)) = cli.command {
        return run_index(args, &cfg);
    }

    let system_prompt = load_system_prompt(cfg.prompts_dir.as_deref());
    let base_url = cfg
        .api_url
        .clone()
        .unwrap_or_else(|| cfg.ollama_url.clone());
    let mut client = match cfg.provider.as_str() {
        "openai" | "vllm" => {
            let key = cfg
                .api_key
                .clone()
                .unwrap_or_else(|| std::env::var("OPENAI_API_KEY").unwrap_or_default());
            if key.is_empty() {
                eprintln!(
                    "{}: provider '{}' requires --api-key or OPENAI_API_KEY",
                    style!(C_YELLOW, "warning"),
                    cfg.provider
                );
            }
            Provider::new_openai(
                base_url,
                cfg.model.clone(),
                cfg.timeout_s,
                system_prompt,
                key,
                cfg.model_ctx,
            )
        }
        _ => Provider::new_ollama(
            base_url,
            cfg.model.clone(),
            cfg.timeout_s,
            system_prompt,
            cfg.model_ctx,
        ),
    };
    client.healthcheck().await?;
    let models = client.list_models().await?;
    if !models.iter().any(|m| m == &cfg.model) {
        let fallback = models.first().cloned().unwrap_or_else(|| cfg.model.clone());
        eprintln!(
            "{}: model '{}' not found; using '{}'",
            style!(C_YELLOW, "warning"),
            cfg.model,
            fallback
        );
        client.set_model(fallback);
    }

    check_system_resources();

    match cli.command {
        Some(Commands::Ask(args)) => run_ask(&client, &cfg, args, verbose).await,
        Some(Commands::Explain(args)) => run_explain(&client, args).await,
        Some(Commands::Patch(args)) => run_patch(&client, &cfg, args).await,
        Some(Commands::New(_)) => unreachable!(),
        None => {
            let is_pipe = !atty::is(atty::Stream::Stdin);
            if is_pipe {
                let mut stdin_data = String::new();
                if io::stdin().read_to_string(&mut stdin_data).is_ok()
                    && !stdin_data.trim().is_empty()
                {
                    return run_pipe(&client, &cfg, stdin_data.trim(), verbose).await;
                }
            }
            run_chat(&client, &mut cfg, verbose).await
        }
        Some(Commands::Index(_)) => {
            // handled by early return before client creation
            unreachable!("Index command should have been handled earlier")
        }
    }
}

fn load_config() -> Result<AppConfig> {
    let mut cfg = AppConfig::default();
    if let Some(home) = dirs::home_dir() {
        let path = home.join(".config/rem-cli/config.toml");
        if path.exists() {
            let text = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let partial: PartialConfig = toml::from_str(&text).context("invalid global config")?;
            cfg.apply_partial(partial);
        }
    }
    let local = PathBuf::from(".remcli.toml");
    if local.exists() {
        let text = fs::read_to_string(&local)
            .with_context(|| format!("failed to read {}", local.display()))?;
        let partial: PartialConfig = toml::from_str(&text).context("invalid local config")?;
        cfg.apply_partial(partial);
    }
    Ok(cfg)
}

fn load_system_prompt(custom_prompts_dir: Option<&str>) -> String {
    let mut candidates = Vec::new();
    if let Some(dir) = custom_prompts_dir {
        candidates.push(PathBuf::from(dir).join("system_prompt.txt"));
    }
    candidates.push(PathBuf::from("prompts/system_prompt.txt"));
    for path in candidates {
        if path.exists() {
            if let Ok(text) = fs::read_to_string(path) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
    }
    DEFAULT_SYSTEM_PROMPT.to_string()
}

async fn run_pipe(client: &Provider, _cfg: &AppConfig, input: &str, verbose: bool) -> Result<()> {
    let prompt = if input.len() > 12000 {
        format!(
            "Analyze the following piped input. Be concise.\n\n{}...\n[truncated]",
            &input[..12000]
        )
    } else {
        format!(
            "Analyze the following piped input. Be concise.\n\n{}",
            input
        )
    };
    let _spinner = SpinnerGuard::new("thinking...");
    let result = client.complete_chat_stream(
        &prompt,
        "[MODE: CHAT] You are in pipe/non-interactive mode. Respond concisely. No code generation unless explicitly asked.",
        "",
    ).await;
    match result {
        Ok(text) => {
            if verbose {
                eprintln!("\n  {} raw:\n{}\n", style!(C_DIM, "verbose:"), text);
            }
            println!();
            println!("{}", text.trim());
            Ok(())
        }
        Err(e) => Err(e),
    }
}

// ── Subcommand handlers ────────────────────────────────────────────────────

async fn run_ask(client: &Provider, cfg: &AppConfig, args: AskArgs, verbose: bool) -> Result<()> {
    let mut composed = args.prompt;
    if let Some(path) = args.file {
        let ctx = build_context(&path, cfg.max_context_bytes)?;
        composed = format!("{}\n\nFile context:\n{}", composed, ctx);
    }
    print_banner(client);

    let intent = classify_intent(&composed);

    let _spinner = SpinnerGuard::new("thinking...");
    let result = match intent {
        TaskIntent::CodeAction => client.complete_json(&composed).await,
        _ => {
            let system_prompt = match intent {
                TaskIntent::FastAnswer => CHAT_SYSTEM_PROMPT_CONVERSATIONAL,
                TaskIntent::Planning => CHAT_SYSTEM_PROMPT_CONVERSATIONAL,
                TaskIntent::WebNeeded => CHAT_SYSTEM_PROMPT_CONVERSATIONAL,
                TaskIntent::CodeAction => unreachable!(),
            };
            let inline_prompt = format!("{}\n\nUser: {}\n\nREM:", system_prompt, composed);
            let url = api_url(&cfg.ollama_url, "generate");
            let payload = json!({
                "model": &client.model,
                "prompt": inline_prompt,
                "stream": false
            });
            let resp = client
                .client
                .post(&url)
                .json(&payload)
                .send()
                .await
                .context("failed to call Ollama")?;
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(anyhow!("Ollama error: {}", body));
            }
            let raw: OllamaJsonResponse = resp.json().await.context("invalid Ollama response")?;
            Ok(ModelReply {
                explanation: raw.response.trim().to_string(),
                code: String::new(),
                files: vec![],
                commands: vec![],
                checks: vec![],
                caution: String::new(),
            })
        }
    };

    let reply = result?;
    if verbose {
        eprintln!(
            "{} raw explanation: {}",
            style!(C_DIM, "verbose:"),
            reply.explanation
        );
        eprintln!("{} raw files: {:?}", style!(C_DIM, "verbose:"), reply.files);
    }
    print_reply(&reply, true);
    Ok(())
}

async fn run_explain(client: &Provider, args: ExplainArgs) -> Result<()> {
    print_banner(client);
    let prompt = format!(
        "Explain this terminal command for a beginner and suggest a safer variant if needed: {}",
        args.command
    );

    let _spinner = SpinnerGuard::new("thinking...");
    let reply = client.complete_json(&prompt).await?;
    print_reply(&reply, false);
    Ok(())
}

async fn run_patch(client: &Provider, cfg: &AppConfig, args: PatchArgs) -> Result<()> {
    print_banner(client);
    let existing = fs::read_to_string(&args.file)
        .with_context(|| format!("failed to read {}", args.file.display()))?;
    let dir_ctx = build_context(&args.file, cfg.max_context_bytes)?;
    let prompt = format!(
        "Task: {}\n\nTarget file: {}\n\nCurrent content:\n{}\n\nNearby context:\n{}\n\nReturn updated file content in code or files array.",
        args.task, args.file.display(), existing, dir_ctx
    );

    let _spinner = SpinnerGuard::new("thinking...");
    let reply = client.complete_json(&prompt).await?;
    println!(
        "{}",
        style!(C_CYAN, "Patch preview for {}", args.file.display())
    );
    print_reply(&reply, true);
    Ok(())
}

// ── Interactive chat ───────────────────────────────────────────────────────

fn print_welcome(client: &Provider) {
    let model_short = client.model.split(':').next().unwrap_or(&client.model);
    println!();
    println!("{}",
        style!(C_CYAN, "\u{2554}\u{2550}\u{2564}\u{2550}\u{2564}\u{2550}\u{2564}\u{2550}\u{2564}\u{2550}\u{2557}"));
    println!(
        "{} {} {} {} {} {} {}",
        style!(C_CYAN, "\u{2551}"),
        style!(C_BOLD, "REM v{}", env!("CARGO_PKG_VERSION")),
        style!(C_CYAN, "\u{2502}"),
        style!(C_DIM, "model: {}", model_short),
        style!(C_CYAN, "\u{2502}"),
        style!(C_GREEN, "\u{26a1} /help for commands"),
        style!(C_CYAN, "\u{2551}")
    );
    println!("{}",
        style!(C_CYAN, "\u{255a}\u{2550}\u{2567}\u{2550}\u{2567}\u{2550}\u{2567}\u{2550}\u{2567}\u{2550}\u{255d}"));
    println!(
        "{} {} {} {}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_GREEN, "┃ mode: CHAT"),
        style!(C_CYAN, "│"),
        style!(C_DIM, "/mode to switch → CODE → PLAN → CHAT")
    );
    println!();
}

fn build_project_context(dir: &Path, max_bytes: usize) -> String {
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
        if rel_str.contains("node_modules")
            || rel_str.contains("target")
            || rel_str.contains("__pycache__")
            || rel_str.contains(".git")
            || rel_str.contains("venv")
            || rel_str.contains("dist")
            || rel_str.contains(".pytest_cache")
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

fn detect_project_type(dir: &Path) -> &'static str {
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

fn language_specific_guidance(project_type: &str) -> &'static str {
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

fn build_prompt(session: &ChatSession, client: &Provider) -> String {
    let model_short = client.model.split(':').next().unwrap_or(&client.model);
    let mode_color = match session.mode {
        RunMode::Chat => C_GREEN,
        RunMode::Code => C_MAGENTA,
        RunMode::Plan => C_BLUE,
    };
    let mut p = String::new();
    p.push('\x01');
    p.push_str(mode_color);
    p.push('\x02');
    p.push('[');
    p.push_str(session.mode.label());
    p.push(']');
    p.push_str("\x01\x1b[0m\x02");
    p.push(' ');
    p.push('\x01');
    p.push_str(C_CYAN);
    p.push('\x02');
    p.push_str(model_short);
    p.push('>');
    p.push_str("\x01\x1b[0m\x02");
    p.push(' ');
    p
}

async fn run_chat(client: &Provider, cfg: &mut AppConfig, verbose: bool) -> Result<()> {
    reset_ctrlc_count();

    let workspace = if let Some(ref dir) = cfg.workspace_dir {
        let path = PathBuf::from(dir);
        if !path.exists() {
            fs::create_dir_all(&path)?;
        }
        Some(path)
    } else {
        first_run_setup(cfg)?
    };

    let mut session = ChatSession::new(&client.model, workspace.clone())?;
    print_welcome(client);
    if let Some(ref wd) = workspace {
        println!(
            "{} {} {}",
            style!(C_CYAN, "│"),
            style!(C_DIM, "workspace →"),
            style!(C_WHITE_B, "{}", wd.display())
        );
        println!("{}", style!(C_CYAN, "│"));
    }

    loop {
        let prompt = build_prompt(&session, client);
        let mut error_count = 0u8;
        let line = loop {
            let line = session.readline(&prompt);
            match line {
                Ok(s) => break s,
                Err(e) => {
                    eprintln!("  {} input error: {}", style!(C_RED, "err:"), e);
                    if e.kind() == io::ErrorKind::Interrupted
                        || e.kind() == io::ErrorKind::UnexpectedEof
                    {
                        return Ok(());
                    }
                    error_count += 1;
                    if error_count >= 3 {
                        eprintln!("  {} too many errors, exiting", style!(C_RED, "err:"));
                        return Ok(());
                    }
                    continue;
                }
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.eq_ignore_ascii_case("exit") || trimmed.eq_ignore_ascii_case("quit") {
            println!("  {}", style!(C_DIM, "bye!"));
            break;
        }

        if trimmed.eq_ignore_ascii_case("/help") || trimmed.eq_ignore_ascii_case("help") {
            print_chat_help();
            continue;
        }

        if let Some(tail) = trimmed.strip_prefix("/write ") {
            handle_write(&mut session, tail);
            continue;
        }
        if let Some(tail) = trimmed.strip_prefix("/save ") {
            handle_write(&mut session, tail);
            continue;
        }

        if let Some(tail) = trimmed.strip_prefix("/dir ") {
            handle_dir(&mut session, tail);
            continue;
        }

        if let Some(tail) = trimmed.strip_prefix("/search ") {
            handle_search(client, &mut session, tail.trim()).await;
            continue;
        }

        if let Some(tail) = trimmed.strip_prefix("/explain ") {
            handle_explain(client, &mut session, tail.trim()).await;
            continue;
        }

        if let Some(tail) = trimmed.strip_prefix("/test ") {
            handle_test(client, &mut session, tail.trim()).await;
            continue;
        }

        if let Some(tail) = trimmed.strip_prefix("/refactor ") {
            handle_refactor(client, &mut session, tail.trim()).await;
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/code") {
            print_last_files(&session);
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/undo") {
            handle_undo(&mut session);
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/files") {
            handle_list_files(&session);
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/mode") {
            session.mode = session.mode.toggle();
            let mode_label = session.mode.label();
            let mode_color = match session.mode {
                RunMode::Chat => C_GREEN,
                RunMode::Code => C_MAGENTA,
                RunMode::Plan => C_BLUE,
            };
            let hint = match session.mode {
                RunMode::Chat => "reply in plain text — ask questions, chat",
                RunMode::Code => "generate code/files — create, fix, build",
                RunMode::Plan => "explore & plan — analyze, propose approach, no code",
            };
            println!("{}", style!(C_DIM, "│"));
            println!(
                "{} {} {}",
                style!(C_CYAN, "│"),
                style!(mode_color, "switched to {} mode", mode_label),
                style!(C_DIM, "")
            );
            println!(
                "{} {} {}",
                style!(C_CYAN, "│"),
                style!(C_DIM, "\u{2502}"),
                style!(C_DIM, "{}", hint)
            );
            println!("{}", style!(C_DIM, "│"));
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/plan") {
            session.mode = RunMode::Plan;
            println!("{}", style!(C_DIM, "│"));
            println!(
                "{} {} {}",
                style!(C_CYAN, "│"),
                style!(C_BLUE, "switched to PLAN mode"),
                style!(C_DIM, "")
            );
            println!(
                "{} {} {}",
                style!(C_CYAN, "│"),
                style!(C_DIM, "\u{2502}"),
                style!(C_DIM, "explore & plan — analyze, propose approach, no code")
            );
            println!("{}", style!(C_DIM, "│"));
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/clear") {
            session.history.clear();
            session.last_search.clear();
            session.last_tokens = 0;
            println!("{}", style!(C_DIM, "│"));
            println!(
                "{} {}",
                style!(C_CYAN, "│"),
                style!(C_GREEN, "conversation cleared")
            );
            println!("{}", style!(C_DIM, "│"));
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/config") {
            handle_config(&session, client);
            continue;
        }
        if let Some(tail) = trimmed.strip_prefix("/config ") {
            handle_config_set(&mut session, client, tail.trim());
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/diff") {
            handle_diff(&session);
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/tokens") {
            handle_tokens(&session);
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/memory") {
            handle_memory(&session);
            continue;
        }
        if let Some(tail) = trimmed.strip_prefix("/memory ") {
            handle_memory_set(&mut session, tail.trim());
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/init") {
            handle_init(&mut session);
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/compact") {
            handle_compact(client, &mut session).await;
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/reset") {
            session.history.clear();
            session.last_search.clear();
            session.last_tokens = 0;
            session.last_code.clear();
            session.last_files.clear();
            session.last_files_written.clear();
            println!("{}", style!(C_DIM, "│"));
            println!(
                "{} {}",
                style!(C_CYAN, "│"),
                style!(
                    C_GREEN,
                    "full reset — history, code cache, and results cleared"
                )
            );
            println!(
                "{}   {} {}",
                style!(C_CYAN, "│"),
                style!(C_DIM, "│"),
                style!(
                    C_DIM,
                    "(memory preserved — use /memory to clear project memory)"
                )
            );
            println!("{}", style!(C_DIM, "│"));
            continue;
        }

        if let Some(tail) = trimmed.strip_prefix("/goal ") {
            handle_goal(client, &mut session, tail.trim()).await;
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/copy") || trimmed == "/copy 1" {
            handle_copy(&session, 1);
            continue;
        }
        if let Some(tail) = trimmed.strip_prefix("/copy ") {
            if let Ok(n) = tail.trim().parse::<usize>() {
                handle_copy(&session, n);
            } else {
                println!("{} usage: /copy [N] — N is a number", style!(C_YELLOW, "│"));
            }
            continue;
        }

        if let Some(tail) = trimmed.strip_prefix("/lint ") {
            handle_lint(&mut session, tail.trim());
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/lint") {
            if session.last_files.is_empty() && session.last_files_written.is_empty() {
                println!(
                    "{} no files to lint. Generate code first.",
                    style!(C_YELLOW, "│")
                );
            } else {
                let paths: Vec<String> = if !session.last_files_written.is_empty() {
                    session
                        .last_files_written
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect()
                } else {
                    session
                        .last_files
                        .iter()
                        .filter(|f| !f.path.is_empty())
                        .map(|f| f.path.clone())
                        .collect()
                };
                for p in paths {
                    handle_lint(&mut session, &p);
                }
            }
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/review") {
            handle_review(client, &mut session).await;
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/find") {
            println!("{}", style!(C_DIM, "\u{2502}"));
            println!(
                "{} {}",
                style!(C_CYAN, "\u{2502}"),
                style!(C_WHITE_B, "usage: /find <query>")
            );
            println!(
                "{} {}  search text inside the project (skips node_modules, target, .git, ...)",
                style!(C_CYAN, "\u{2502}"),
                style!(C_DIM, "\u{2022}")
            );
            println!("{}", style!(C_DIM, "\u{2502}"));
            continue;
        }

        if let Some(tail) = trimmed.strip_prefix("/find ") {
            handle_find(&session, tail.trim());
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/save") && !trimmed.starts_with("/save ") {
            handle_save_session(&session);
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/resume") {
            handle_resume_session(&mut session);
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/why") {
            let intent_name = match session.last_intent {
                TaskIntent::FastAnswer => "chat/question",
                TaskIntent::Planning => "planning",
                TaskIntent::WebNeeded => "web search needed",
                TaskIntent::CodeAction => "code/file action",
            };
            println!("{}", style!(C_DIM, "│"));
            println!(
                "{} {} {} {}",
                style!(C_CYAN, "│"),
                style!(C_WHITE_B, "last intent:"),
                style!(C_GREEN, "{}", intent_name),
                style!(C_DIM, "")
            );
            println!(
                "{} {} {} {}",
                style!(C_CYAN, "│"),
                style!(C_WHITE_B, "last input:"),
                style!(C_DIM, "\"{}\"", session.last_user_input),
                style!(C_DIM, "")
            );
            let create_hit = has_creation_intent(&session.last_user_input);
            let lower_db = session.last_user_input.to_lowercase();
            let fix_hit = lower_db.starts_with("fix ")
                || lower_db.starts_with("refactor ")
                || lower_db.starts_with("rename ")
                || lower_db.starts_with("delete ")
                || lower_db.starts_with("remove ")
                || lower_db.starts_with("optimize ")
                || lower_db.starts_with("update ");
            let is_q = lower_db.starts_with("what ")
                || lower_db.starts_with("how ")
                || lower_db.starts_with("why ")
                || lower_db.starts_with("explain ");
            println!(
                "{} {}",
                style!(C_CYAN, "│"),
                style!(C_DIM, "  has_creation_intent={}", create_hit)
            );
            println!(
                "{} {}",
                style!(C_CYAN, "│"),
                style!(C_DIM, "  fix_window={}  is_question={}", fix_hit, is_q)
            );
            println!("{}", style!(C_DIM, "│"));
            continue;
        }

        let needs_path = (session.mode == RunMode::Code || has_creation_intent(trimmed))
            && !has_file_path(trimmed);
        let final_prompt = if needs_path {
            session.add_history(trimmed);
            let path = prompt_for_path(&mut session)?;
            format!("User request: {}\n\nSave file at: {}", trimmed, path)
        } else {
            session.add_history(trimmed);
            if let Some(ref dir) = session.project_dir {
                format!(
                    "User request: {}\n\nWorking directory: {}",
                    trimmed,
                    dir.display()
                )
            } else {
                format!("User request: {}", trimmed)
            }
        };

        let intent = classify_intent(trimmed);
        session.last_intent = intent.clone();
        session.last_user_input = trimmed.to_string();
        let instruction = intent_instruction(&intent);

        if session.mode == RunMode::Code {
            print!("{} ", style!(C_CYAN, "\u{2502}"));
            println!("{}", style!(C_MAGENTA, "generating code..."));
        } else if session.mode == RunMode::Plan {
            print!("{} ", style!(C_CYAN, "\u{2502}"));
            println!("{}", style!(C_BLUE, "analyzing & planning..."));
        } else if intent == TaskIntent::CodeAction {
            print!("{} ", style!(C_CYAN, "\u{2502}"));
            println!("{}", style!(C_CYAN, "Analyzing..."));
        }

        let search_ctx = session.build_search_context();
        let history_ctx = session.build_chat_history();
        let memory_ctx = session.build_memory_context();
        let (resolved_input, at_context) = session.resolve_at_references(&final_prompt);

        // Query-aware retrieval (if .rem/codebase_index.json or models/ equivalent exists).
        // This replaces the old exhaustive "every file name + size" dump for projects that have
        // been indexed. Massive scaling improvement: model receives actual relevant source.
        let project_ctx = session.build_relevant_project_context(&resolved_input);

        let full_prompt = {
            let mut p = instruction.to_string();
            p.push('\n');
            if !memory_ctx.is_empty() {
                p.push_str(&memory_ctx);
            }
            if !project_ctx.is_empty() {
                p.push_str(&project_ctx);
            }
            if !at_context.is_empty() {
                p.push_str(&at_context);
            }
            p.push_str(&resolved_input);
            if !search_ctx.is_empty() {
                p.push_str(&search_ctx);
            }
            p
        };

        print!("{} ", style!(C_CYAN, "\u{2502}"));
        println!("{}", style!(C_DIM, "\u{2500}\u{2500} rem \u{2500}\u{2500}"));

        let start = std::time::Instant::now();
        let _chat_spinner = SpinnerGuard::new("REM is writing...");
        let system_prompt = match session.mode {
            RunMode::Chat => CHAT_SYSTEM_PROMPT_CONVERSATIONAL,
            RunMode::Code => CHAT_SYSTEM_PROMPT_CODE,
            RunMode::Plan => CHAT_SYSTEM_PROMPT_PLAN,
        };

        let lang_guidance = if let Some(ref dir) = session.project_dir {
            let ptype = detect_project_type(dir);
            if !ptype.is_empty() {
                language_specific_guidance(ptype)
            } else {
                ""
            }
        } else {
            ""
        };

        let system_prompt = if !lang_guidance.is_empty() {
            format!("{}{}", system_prompt, lang_guidance)
        } else {
            system_prompt.to_string()
        };

        if session.mode == RunMode::Chat && intent == TaskIntent::CodeAction {
            println!("{}", style!(C_DIM, "\u{2502}"));
            println!(
                "{} {}",
                style!(C_YELLOW, "\u{2502}  hint:"),
                style!(
                    C_CYAN,
                    "this looks like a code request — type /mode to switch to CODE"
                )
            );
            println!("{}", style!(C_DIM, "\u{2502}"));
        }
        if session.mode == RunMode::Plan && intent == TaskIntent::CodeAction {
            println!("{}", style!(C_DIM, "\u{2502}"));
            println!(
                "{} {}",
                style!(C_YELLOW, "\u{2502}  hint:"),
                style!(
                    C_BLUE,
                    "in PLAN mode — I'll analyze first, then you can switch to CODE"
                )
            );
            println!("{}", style!(C_DIM, "\u{2502}"));
        }
        let result = client
            .complete_chat_stream(&full_prompt, &system_prompt, &history_ctx)
            .await;
        let elapsed = start.elapsed();
        session.last_elapsed = elapsed;

        match result {
            Ok(text) => {
                if verbose {
                    eprintln!(
                        "\n  {} raw response:\n{}\n",
                        style!(C_DIM, "verbose:"),
                        text
                    );
                }

                let (was_validated, validated_text) =
                    validate_chat_response(&text, &intent, &session.mode);
                let cleaned = if was_validated && session.mode != RunMode::Code {
                    println!(
                        "{} {}",
                        style!(C_YELLOW, "│"),
                        style!(
                            C_DIM,
                            "(response contained unexpected code — showing text only)"
                        )
                    );
                    println!("{}", style!(C_CYAN, "│"));
                    validated_text
                } else {
                    text.trim().to_string()
                };

                session.last_tokens = (cleaned.len() / 4) as u32;

                let treat_as_code =
                    intent == TaskIntent::CodeAction || session.mode == RunMode::Code;

                if treat_as_code {
                    let code = extract_code_block(&cleaned);
                    let files = extract_code_blocks_with_names(&cleaned);

                    if !files.is_empty() {
                        session.last_files = files.clone();
                        session.last_code = if code.is_empty() { String::new() } else { code };
                        println!("{}", style!(C_CYAN, "│"));
                        println!(
                            "{} {} {}",
                            style!(C_CYAN, "│"),
                            style!(C_GREEN, "generated:"),
                            style!(C_WHITE_B, "{} file(s)", files.len())
                        );
                        for f in &files {
                            let icon = file_icon(&f.path);
                            if f.path.is_empty() {
                                println!(
                                    "{}   {} unnamed ({} bytes)",
                                    style!(C_CYAN, "│"),
                                    icon,
                                    f.content.len()
                                );
                            } else {
                                println!(
                                    "{}   {} {} ({} bytes)",
                                    style!(C_CYAN, "│"),
                                    icon,
                                    style!(C_WHITE_B, "{}", f.path),
                                    f.content.len()
                                );
                            }
                        }
                        println!("{}", style!(C_CYAN, "│"));

                        auto_write_files(&mut session, &files);
                    } else if !code.is_empty() {
                        session.last_code = code;
                        session.last_files.clear();
                        println!("{}", style!(C_CYAN, "│"));
                        println!(
                            "{} {}",
                            style!(C_CYAN, "│"),
                            style!(C_GREEN, "detected code block — use /write <path> to save")
                        );
                        println!("{}", style!(C_CYAN, "│"));
                    } else {
                        for line in cleaned.lines() {
                            println!("{} {}", style!(C_CYAN, "│"), line);
                        }
                        println!("{}", style!(C_CYAN, "│"));
                        println!(
                            "{} {}",
                            style!(C_CYAN, "│"),
                            style!(C_DIM, "\u{23f1} {:.1}s", elapsed.as_secs_f64())
                        );
                    }
                } else if cleaned.is_empty() {
                    println!(
                        "{} {}",
                        style!(C_YELLOW, "│"),
                        style!(C_DIM, "(empty response)")
                    );
                } else {
                    for line in cleaned.lines() {
                        println!("{} {}", style!(C_CYAN, "│"), line);
                    }
                    println!("{}", style!(C_CYAN, "│"));
                    println!(
                        "{} {}",
                        style!(C_CYAN, "│"),
                        style!(C_DIM, "\u{23f1} {:.1}s", elapsed.as_secs_f64())
                    );
                }

                if !cleaned.is_empty() {
                    session.history.push((trimmed.to_string(), cleaned));
                    if session.history.len() > 12 {
                        session.history.remove(0);
                    }
                }

                println!("{}", style!(C_DIM, "│"));
            }
            Err(e) => {
                println!(
                    "{}",
                    style!(C_DIM, "\u{23f1} {:.1}s", elapsed.as_secs_f64())
                );
                eprintln!("  {} {}", style!(C_RED, "err:"), e);
                println!("{}", style!(C_DIM, "│"));
            }
        }
    }
    session.feedback.flush();
    Ok(())
}

#[derive(Debug, PartialEq, Clone)]
enum RunMode {
    Chat,
    Code,
    Plan,
}

impl RunMode {
    fn toggle(&self) -> RunMode {
        match self {
            RunMode::Chat => RunMode::Code,
            RunMode::Code => RunMode::Plan,
            RunMode::Plan => RunMode::Chat,
        }
    }

    fn label(&self) -> &str {
        match self {
            RunMode::Chat => "CHAT",
            RunMode::Code => "CODE",
            RunMode::Plan => "PLAN",
        }
    }
}

fn validate_chat_response(response: &str, intent: &TaskIntent, mode: &RunMode) -> (bool, String) {
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

fn prompt_for_path(session: &mut ChatSession) -> io::Result<String> {
    let workspace_display = session
        .project_dir
        .as_ref()
        .map(|d| d.display().to_string())
        .unwrap_or_else(|| "current dir".to_string());
    println!("{}", style!(C_CYAN, "│"));
    println!(
        "{} {}",
        style!(C_MAGENTA, "│  ?"),
        style!(
            C_WHITE_B,
            "Where should I create this? (e.g. ./my-site/index.html or ./project/)"
        )
    );
    println!(
        "{} {} workspace: {}",
        style!(C_MAGENTA, "│"),
        style!(C_DIM, ""),
        style!(C_WHITE_B, "{}", workspace_display)
    );
    println!(
        "{} {} type '.' for workspace root, or /dir <path> to change",
        style!(C_MAGENTA, "│"),
        style!(C_DIM, "")
    );
    println!("{}", style!(C_CYAN, "│"));

    loop {
        let line = session.readline("rem> path: ");
        let line = match line {
            Ok(s) => s,
            Err(_) => return Ok(".".to_string()),
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        session.add_history(trimmed);

        if trimmed.eq_ignore_ascii_case("exit") || trimmed.eq_ignore_ascii_case("quit") {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }

        if let Some(tail) = trimmed.strip_prefix("/dir ") {
            handle_dir(session, tail);
            continue;
        }

        return Ok(trimmed.to_string());
    }
}

fn handle_write(session: &mut ChatSession, path: &str) {
    let trimmed = path.trim();
    let base_dir = session
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let abs_path = match resolve_safe_path(&base_dir, trimmed) {
        Some(p) => p,
        None => return,
    };

    if session.last_code.is_empty() {
        println!(
            "  {} No code from last response. Use `/code` to view it.",
            style!(C_YELLOW, "!")
        );
        return;
    }

    if abs_path.exists() {
        let existing_size = fs::metadata(&abs_path).map(|m| m.len()).unwrap_or(0);
        println!(
            "  {} {} exists ({} bytes) — {} [y/N]",
            style!(C_YELLOW, "\u{26a0}"),
            style!(C_WHITE_B, "{}", trimmed),
            existing_size,
            style!(C_DIM, "overwrite?")
        );
        let input = session.readline("rem> ").unwrap_or_else(|_| String::new());
        if !input.trim().eq_ignore_ascii_case("y") && !input.trim().eq_ignore_ascii_case("yes") {
            println!("  {} skipped", style!(C_DIM, "\u{2502}"));
            return;
        }
    }

    if let Some(parent) = abs_path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = fs::create_dir_all(parent) {
                eprintln!(
                    "  {} cannot create directory {}: {}",
                    style!(C_RED, "\u{2717}"),
                    parent.display(),
                    e
                );
                return;
            }
        }
    }

    let tmp = abs_path.with_extension("tmp");
    match fs::write(&tmp, &session.last_code) {
        Ok(()) => {
            if let Err(e) = fs::rename(&tmp, &abs_path) {
                eprintln!("  {} atomic write failed: {}", style!(C_RED, "\u{2717}"), e);
                let _ = fs::remove_file(&tmp);
                return;
            }
            println!(
                "  {} wrote {} ({} bytes)",
                style!(C_GREEN, "\u{2713}"),
                style!(C_WHITE_B, "{}", abs_path.display()),
                session.last_code.len()
            );
            session.last_files_written.push(abs_path);
        }
        Err(e) => {
            println!("  {} failed: {}", style!(C_RED, "\u{2717}"), e);
            let _ = fs::remove_file(&tmp);
        }
    }
}

fn auto_write_files(session: &mut ChatSession, files: &[FileEntry]) {
    if files.is_empty() || files.iter().all(|f| f.path.is_empty()) {
        println!(
            "{} {} Type /write <path> to save.",
            style!(C_YELLOW, "\u{2502}  !"),
            style!(C_DIM, "")
        );
        return;
    }

    let base_dir = session
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let mut safe_entries: Vec<(&FileEntry, PathBuf)> = Vec::new();
    for f in files {
        if f.path.is_empty() {
            continue;
        }
        match resolve_safe_path(&base_dir, &f.path) {
            Some(abs) => safe_entries.push((f, abs)),
            None => {
                eprintln!(
                    "{}   {} {} {}",
                    style!(C_RED, "\u{2502} \u{2717}"),
                    style!(C_WHITE_B, "{}", f.path),
                    style!(C_DIM, "—"),
                    style!(C_RED, "path traversal blocked")
                );
            }
        }
    }

    if safe_entries.is_empty() {
        return;
    }

    println!("{}", style!(C_CYAN, "\u{2502}"));
    println!(
        "{} {} {}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_WHITE_B, "Plan: creating {} file(s)", safe_entries.len()),
        style!(C_DIM, "")
    );
    for (f, abs_path) in &safe_entries {
        let icon = file_icon(&f.path);
        let lines = f.content.lines().count();
        let marker = if abs_path.exists() {
            style!(C_YELLOW, " [EXISTS]")
        } else {
            String::new()
        };
        println!(
            "{}   {} {} ({}, {} lines){}",
            style!(C_CYAN, "\u{2502}"),
            icon,
            style!(C_WHITE_B, "{}", f.path),
            style!(C_DIM, "{} bytes", f.content.len()),
            style!(C_DIM, "{}", lines),
            marker
        );
    }
    println!("{}", style!(C_CYAN, "\u{2502}"));
    println!(
        "{} {} {}",
        style!(C_MAGENTA, "\u{2502}  ?"),
        style!(C_WHITE_B, "Write all {} files? [Y/n]", safe_entries.len()),
        style!(C_DIM, "(press Enter to confirm)")
    );
    println!(
        "{} {}",
        style!(C_MAGENTA, "\u{2502}"),
        style!(C_DIM, "  Type /code to preview, 'n' to cancel")
    );
    println!("{}", style!(C_CYAN, "\u{2502}"));

    let input = session
        .readline("rem> ")
        .unwrap_or_else(|_| String::from("y"));
    let input = input.trim();
    if !input.is_empty() && !input.eq_ignore_ascii_case("y") && !input.eq_ignore_ascii_case("yes") {
        println!(
            "{} skipped. Use /write <path> to save individually.",
            style!(C_YELLOW, "\u{2502}  !")
        );
        println!("{}", style!(C_CYAN, "\u{2502}"));
        return;
    }

    let mut written: Vec<PathBuf> = Vec::new();
    for (f, abs_path) in &safe_entries {
        let will_overwrite = abs_path.exists();
        if will_overwrite {
            println!(
                "{}   {} {} {}",
                style!(C_YELLOW, "\u{2502} \u{26a0}"),
                style!(C_WHITE_B, "{}", f.path),
                style!(C_DIM, "exists — overwriting"),
                style!(C_DIM, "")
            );
        }

        if let Some(parent) = abs_path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = fs::create_dir_all(parent) {
                    eprintln!(
                        "{}   {} cannot create dir {}: {}",
                        style!(C_RED, "\u{2502} \u{2717}"),
                        style!(C_WHITE_B, "{}", f.path),
                        parent.display(),
                        e
                    );
                    continue;
                }
            }
        }

        let tmp = abs_path.with_extension("tmp");
        match fs::write(&tmp, &f.content) {
            Ok(()) => {
                if let Err(e) = fs::rename(&tmp, abs_path) {
                    eprintln!(
                        "{}   {} atomic write failed: {}",
                        style!(C_RED, "\u{2502} \u{2717}"),
                        style!(C_WHITE_B, "{}", f.path),
                        e
                    );
                    let _ = fs::remove_file(&tmp);
                    continue;
                }
                let overwrite_note = if will_overwrite { " (overwritten)" } else { "" };
                println!(
                    "{}   {} {} ({}{})",
                    style!(C_GREEN, "\u{2502} \u{2713}"),
                    style!(C_WHITE_B, "{}", f.path),
                    style!(C_DIM, "{} bytes", f.content.len()),
                    style!(C_DIM, "{}", overwrite_note),
                    String::new()
                );
                written.push(abs_path.clone());
            }
            Err(e) => {
                println!(
                    "{}   {} {}: {}",
                    style!(C_RED, "\u{2502} \u{2717}"),
                    style!(C_WHITE_B, "{}", f.path),
                    style!(C_DIM, ""),
                    e
                );
                let _ = fs::remove_file(&tmp);
            }
        }
    }

    if !written.is_empty() {
        session.last_files_written = written;
        println!(
            "{} {} {} files written.",
            style!(C_GREEN, "\u{2502} \u{2713}"),
            style!(C_WHITE_B, "{}", session.last_files_written.len()),
            style!(C_DIM, "")
        );
    }
}

fn handle_undo(session: &mut ChatSession) {
    if session.last_files_written.is_empty() {
        println!("  {} Nothing to undo.", style!(C_YELLOW, "!"));
        return;
    }
    println!(
        "{} {}",
        style!(C_MAGENTA, "\u{2502}  ?"),
        style!(
            C_WHITE_B,
            "Delete the last {} written file(s)? [y/N]",
            session.last_files_written.len()
        )
    );

    let input = session.readline("rem> ").unwrap_or_else(|_| String::new());
    let input = input.trim();
    if !input.eq_ignore_ascii_case("y") && !input.eq_ignore_ascii_case("yes") {
        println!("  {} cancelled", style!(C_DIM, "\u{2502}"));
        return;
    }

    let mut removed = 0;
    let mut dirs_to_clean: Vec<PathBuf> = Vec::new();
    for path in session.last_files_written.drain(..) {
        if path.exists() {
            if let Some(parent) = path.parent() {
                dirs_to_clean.push(parent.to_path_buf());
            }
            match fs::remove_file(&path) {
                Ok(()) => {
                    println!(
                        "  {} removed {}",
                        style!(C_YELLOW, "\u{2502}"),
                        style!(C_DIM, "{}", path.display())
                    );
                    removed += 1;
                }
                Err(e) => {
                    println!(
                        "  {} failed to remove {}: {}",
                        style!(C_RED, "\u{2502}"),
                        path.display(),
                        e
                    );
                }
            }
        }
    }

    dirs_to_clean.sort_by_key(|b| std::cmp::Reverse(b.as_os_str().len()));
    for dir in &dirs_to_clean {
        if dir.exists() {
            let _ = fs::remove_dir(dir);
        }
    }

    if removed > 0 {
        let input = session.last_user_input.clone();
        let intent = session.last_intent.clone();
        if intent == TaskIntent::CodeAction {
            session
                .feedback
                .record_correction(&input, &intent, &TaskIntent::FastAnswer);
        }
        println!(
            "  {} {} {} file(s) removed.",
            style!(C_GREEN, "\u{2502} \u{2713}"),
            removed,
            style!(C_DIM, "")
        );
    }
}

fn handle_list_files(session: &ChatSession) {
    let dir = session
        .project_dir
        .as_ref()
        .cloned()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    println!("{}", style!(C_DIM, "\u{2502}"));
    println!(
        "{} {}",
        style!(C_DIM, "\u{2502}"),
        style!(C_WHITE_B, "\u{1f4c2} project ({})", dir.display())
    );
    println!("{}", style!(C_DIM, "\u{2502}"));

    let mut entries: Vec<(String, bool, u64)> = Vec::new();
    for entry in WalkDir::new(&dir)
        .max_depth(4)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if p == dir {
            continue;
        }
        if let Ok(rel) = p.strip_prefix(&dir) {
            let size = if p.is_file() {
                fs::metadata(p).map(|m| m.len()).unwrap_or(0)
            } else {
                0
            };
            entries.push((rel.display().to_string(), p.is_dir(), size));
        }
    }
    entries.sort();

    if entries.is_empty() {
        println!(
            "{}   {}",
            style!(C_DIM, "\u{2502}"),
            style!(C_YELLOW, "(empty)")
        );
    } else {
        for (path, is_dir, size) in &entries {
            let depth = path.chars().filter(|&c| c == '/').count();
            let indent = "  ".repeat(depth);
            let name = if let Some(pos) = path.rfind('/') {
                &path[pos + 1..]
            } else {
                path
            };
            if *is_dir {
                println!(
                    "{} {} {} {} {}",
                    style!(C_DIM, "\u{2502}"),
                    indent,
                    style!(C_DIM, "\u{251c}\u{2500}\u{2500}"),
                    style!(C_BLUE, "\u{1f4c1} {}/", name),
                    style!(C_RESET, "")
                );
            } else {
                let icon = file_icon(name);
                let hs = human_size(*size);
                println!(
                    "{} {} {} {} {} {}",
                    style!(C_DIM, "\u{2502}"),
                    indent,
                    style!(C_DIM, "\u{251c}\u{2500}\u{2500}"),
                    icon,
                    style!(C_WHITE_B, "{}", name),
                    style!(C_DIM, "({})", hs)
                );
            }
        }
    }
    println!("{}", style!(C_DIM, "\u{2502}"));
}

fn highlight_code(content: &str, lang_hint: &str) -> String {
    let lang = lang_hint.to_lowercase();
    if lang.contains("html") {
        highlight_html(content)
    } else if lang.contains("css") {
        highlight_css(content)
    } else if lang.contains("js")
        || lang.contains("javascript")
        || lang.contains("ts")
        || lang.contains("typescript")
    {
        highlight_js(content)
    } else {
        highlight_generic(content)
    }
}

fn highlight_html(code: &str) -> String {
    let mut out = code.to_string();
    out = RE_HIGHLIGHT_COMMENT
        .replace_all(&out, |caps: &regex::Captures| style!(C_DIM, "{}", &caps[1]))
        .to_string();
    out = RE_HIGHLIGHT_HTML_TAG
        .replace_all(&out, |caps: &regex::Captures| {
            let tag = &caps[1];
            let inner = RE_HIGHLIGHT_ATTR
                .replace_all(tag, |ac: &regex::Captures| style!(C_GREEN, "{}", &ac[1]))
                .to_string();
            style!(C_CYAN, "{}", inner)
        })
        .to_string();
    out
}

fn highlight_css(code: &str) -> String {
    let mut out = code.to_string();
    out = RE_CSS_COMMENT
        .replace_all(&out, |caps: &regex::Captures| style!(C_DIM, "{}", &caps[1]))
        .to_string();
    out = RE_CSS_PROP
        .replace_all(&out, |caps: &regex::Captures| {
            format!(
                "{}{}{}{}",
                &caps[1],
                style!(C_YELLOW, "{}", &caps[2]),
                &caps[3],
                style!(C_RESET, "{}", "")
            )
        })
        .to_string();
    out = RE_CSS_VAL
        .replace_all(&out, |caps: &regex::Captures| {
            format!(
                "{}{}{}{}",
                &caps[1],
                style!(C_GREEN, "{}", &caps[2].trim()),
                style!(C_RESET, "{}", ""),
                ""
            )
        })
        .to_string();
    out
}

fn highlight_js(code: &str) -> String {
    let mut out = code.to_string();
    out = RE_JS_COMMENT
        .replace_all(&out, |caps: &regex::Captures| style!(C_DIM, "{}", &caps[1]))
        .to_string();
    out = RE_JS_STR
        .replace_all(&out, |caps: &regex::Captures| {
            style!(C_GREEN, "{}", &caps[1])
        })
        .to_string();
    out = RE_JS_KW
        .replace_all(&out, |caps: &regex::Captures| {
            style!(C_MAGENTA, "{}", &caps[1])
        })
        .to_string();
    out
}

fn highlight_generic(code: &str) -> String {
    code.to_string()
}

fn detect_language_from_content(code: &str) -> &str {
    let first_line = code.trim().lines().next().unwrap_or("");
    if first_line.starts_with("<!") || first_line.starts_with("<") {
        "html"
    } else if first_line.contains("{")
        && first_line.contains("}")
        && !first_line.contains("function")
        && !first_line.contains("=>")
    {
        "css"
    } else if first_line.starts_with("const ")
        || first_line.starts_with("let ")
        || first_line.starts_with("function ")
        || first_line.starts_with("import ")
    {
        "js"
    } else {
        ""
    }
}

fn print_last_files(session: &ChatSession) {
    if !session.last_files.is_empty() {
        for f in &session.last_files {
            let label = if f.path.is_empty() {
                "(unnamed)".to_string()
            } else {
                f.path.clone()
            };
            let lang = detect_language_from_content(&f.content);
            let lang_display = if lang.is_empty() {
                String::new()
            } else {
                format!(" [{}]", lang)
            };
            println!(
                "{}",
                style!(
                    C_WHITE_B,
                    "\u{2500}\u{2500} {}{} \u{2500}\u{2500}",
                    label,
                    style!(C_DIM, "{}", lang_display)
                )
            );
            let highlighted = highlight_code(&f.content, lang);
            for code_line in highlighted.lines() {
                println!("{}", code_line);
            }
            println!("{}", style!(C_DIM, "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"));
        }
    } else if !session.last_code.is_empty() {
        let lang = detect_language_from_content(&session.last_code);
        let lang_display = if lang.is_empty() {
            String::new()
        } else {
            format!(" [{}]", lang)
        };
        println!(
            "{}",
            style!(
                C_WHITE_B,
                "\u{2500}\u{2500} last code{} \u{2500}\u{2500}",
                style!(C_DIM, "{}", lang_display)
            )
        );
        let highlighted = highlight_code(&session.last_code, lang);
        println!("{}", highlighted);
        println!("{}", style!(C_DIM, "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"));
    } else {
        println!("  {} No code from last response.", style!(C_YELLOW, "!"));
    }
}

fn handle_dir(session: &mut ChatSession, path: &str) {
    let dir = PathBuf::from(path.trim());
    let resolved = if path.trim() == "." {
        std::env::current_dir().unwrap_or_default()
    } else {
        dir
    };
    if resolved.exists() || path.trim() == "." {
        session.project_dir = Some(resolved.clone());
        session.workspace_dir = Some(resolved.clone());
        persist_workspace(&resolved);
        println!(
            "  {} workspace set to {}",
            style!(C_GREEN, "✓"),
            style!(
                C_WHITE_B,
                "{}",
                session.project_dir.as_ref().unwrap().display()
            )
        );
    } else {
        println!(
            "  {} directory does not exist — creating it",
            style!(C_YELLOW, "!")
        );
        if let Err(e) = fs::create_dir_all(&resolved) {
            println!("  {} failed: {}", style!(C_RED, "✗"), e);
            return;
        }
        session.project_dir = Some(resolved.clone());
        session.workspace_dir = Some(resolved.clone());
        persist_workspace(&resolved);
        println!(
            "  {} workspace set to {}",
            style!(C_GREEN, "✓"),
            style!(
                C_WHITE_B,
                "{}",
                session.project_dir.as_ref().unwrap().display()
            )
        );
    }
}

fn persist_workspace(dir: &Path) {
    let mut cfg = load_config().unwrap_or_default();
    cfg.workspace_dir = Some(dir.to_string_lossy().to_string());
    if let Err(e) = save_config(&cfg) {
        eprintln!(
            "  {} failed to save workspace config: {}",
            style!(C_RED, "✗"),
            e
        );
    }
}

async fn handle_search(client: &Provider, session: &mut ChatSession, query: &str) {
    println!(
        "{} {} searching the web...",
        style!(C_DIM, "│"),
        style!(C_CYAN, "🔍")
    );
    match perform_web_search(&client.client, query).await {
        Ok(results) => {
            if results.is_empty() {
                println!("{} no results found for: {}", style!(C_YELLOW, "│"), query);
            } else {
                println!(
                    "{} {} results for: {}",
                    style!(C_DIM, "│"),
                    results.len(),
                    style!(C_WHITE_B, "{}", query)
                );
                print_search_results(&results);
                session.last_search = results;
            }
        }
        Err(e) => {
            println!("{} {}", style!(C_RED, "│  search failed:"), e);
        }
    }
}

async fn handle_explain(client: &Provider, session: &mut ChatSession, text: &str) {
    if text.trim().is_empty() {
        println!("{} usage: /explain <code snippet>", style!(C_YELLOW, "│"));
        return;
    }
    println!("{} explaining...", style!(C_CYAN, "\u{2502}"));
    let prompt = format!(
        "Explain what the following code does in clear, plain language. \
         Be concise but thorough. Cover: purpose, key components, control flow. \
         Do NOT generate new code. Just explain.\n\nCode:\n```\n{}\n```",
        text
    );
    match client.complete_chat_stream(
        &prompt,
        "[MODE: CHAT] You are a code explainer. Respond with plain text only — no code generation, no file format, no JSON.",
        "",
    ).await {
        Ok(response) => {
            println!("\n{}", response);
            session.add_history(&format!("/explain {}", text));
            session.history.push((format!("/explain {}", text), response));
        }
        Err(e) => {
            println!("\n{} explain failed: {}", style!(C_RED, "│"), e);
        }
    }
}

async fn handle_test(client: &Provider, session: &mut ChatSession, path: &str) {
    let file_path = Path::new(path.trim());
    if !file_path.exists() {
        println!("{} file not found: {}", style!(C_YELLOW, "│"), path);
        return;
    }
    let content = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            println!("{} cannot read file: {}", style!(C_RED, "│"), e);
            return;
        }
    };
    println!(
        "{} generating tests for {}...",
        style!(C_CYAN, "\u{2502}"),
        path
    );
    let prompt = format!(
        "Generate comprehensive tests for the following code. \
         Include unit tests for all public functions/methods, edge cases, \
         and error handling. Write tests in the same language and testing \
         framework conventions.\n\nSource code:\n```\n{}\n```",
        truncate_to_lines(&content, 200)
    );
    match client.complete_chat_stream(
        &prompt,
        "[MODE: CODE] Generate test code for the given source file. Respond with the test code in a fenced code block.",
        "",
    ).await {
        Ok(response) => {
            println!();
            println!("{}", response);
            session.last_code = extract_code_block(&response);
            session.add_history(&format!("/test {}", path));
            session.history.push((format!("/test {}", path), response));
            if !session.last_code.is_empty() {
                println!("{} tests ready — use {} to save",
                    style!(C_GREEN, "│"),
                    style!(C_WHITE_B, "/write <path>"));
            }
        }
        Err(e) => {
            println!("\n{} test generation failed: {}", style!(C_RED, "│"), e);
        }
    }
}

async fn handle_refactor(client: &Provider, session: &mut ChatSession, path: &str) {
    let file_path = Path::new(path.trim());
    if !file_path.exists() {
        println!("{} file not found: {}", style!(C_YELLOW, "│"), path);
        return;
    }
    let content = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            println!("{} cannot read file: {}", style!(C_RED, "│"), e);
            return;
        }
    };
    println!(
        "{} analyzing {} for refactoring...",
        style!(C_CYAN, "\u{2502}"),
        path
    );
    let prompt = format!(
        "Review the following code and suggest refactoring improvements. \
         Consider: code clarity, DRY principle, performance, error handling, \
         naming, structure. Give specific recommendations with before/after \
         code examples where helpful.\n\nSource code:\n```\n{}\n```",
        truncate_to_lines(&content, 200)
    );
    match client.complete_chat_stream(
        &prompt,
        "[MODE: CHAT] You are a code reviewer. Analyze the code and provide refactoring suggestions. Use clear markdown formatting.",
        "",
    ).await {
        Ok(response) => {
            println!();
            println!("{}", response);
            session.add_history(&format!("/refactor {}", path));
            session.history.push((format!("/refactor {}", path), response));
        }
        Err(e) => {
            println!("\n{} refactor analysis failed: {}", style!(C_RED, "│"), e);
        }
    }
}

fn handle_config(session: &ChatSession, client: &Provider) {
    println!("{}", style!(C_DIM, "\u{2502}"));
    println!(
        "{}  {}{}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_WHITE_B, "\u{2500}\u{2500} CONFIG \u{2500}\u{2500}"),
        style!(C_DIM, "")
    );
    println!(
        "{}   {:<18} {}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_WHITE_B, "model:"),
        style!(C_DIM, "{}", client.model)
    );
    println!(
        "{}   {:<18} {}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_WHITE_B, "ollama url:"),
        style!(C_DIM, "{}", client.base_url)
    );
    println!(
        "{}   {:<18} {}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_WHITE_B, "mode:"),
        style!(C_DIM, "{}", session.mode.label())
    );
    println!(
        "{}   {:<18} {}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_WHITE_B, "workspace:"),
        style!(
            C_DIM,
            "{}",
            session
                .project_dir
                .as_ref()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| "none".to_string())
        )
    );
    println!("{}", style!(C_DIM, "\u{2502}"));
    println!(
        "{} {} {}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_DIM, ""),
        style!(C_DIM, "use /config model <name> to switch models")
    );
    println!("{}", style!(C_CYAN, "\u{2502}"));
}

fn handle_config_set(session: &mut ChatSession, client: &Provider, args: &str) {
    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    if parts.is_empty() {
        handle_config(session, client);
        return;
    }
    match parts[0] {
        "workspace" | "dir" => {
            if parts.len() > 1 {
                handle_dir(session, parts[1]);
            } else {
                println!("{} usage: /config workspace <path>", style!(C_YELLOW, "│"));
            }
        }
        other => {
            println!("{} unknown config key: {}", style!(C_YELLOW, "│"), other);
            println!("{} available: model, workspace", style!(C_DIM, "│"));
        }
    }
}

fn handle_diff(session: &ChatSession) {
    if session.last_files.is_empty() {
        println!(
            "{} No generated files to compare.",
            style!(C_YELLOW, "\u{2502}")
        );
        return;
    }

    let base_dir = session
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    println!("{}", style!(C_DIM, "\u{2502}"));
    println!(
        "{} {}{}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_WHITE_B, "--- DIFF ---"),
        style!(C_DIM, "")
    );
    println!("{}", style!(C_DIM, "\u{2502}"));

    for f in &session.last_files {
        if f.path.is_empty() {
            continue;
        }
        let rel_path = PathBuf::from(&f.path);
        let abs_path = if rel_path.is_relative() {
            base_dir.join(&rel_path)
        } else {
            rel_path
        };

        let icon = file_icon(&f.path);
        if abs_path.exists() {
            let existing = fs::read_to_string(&abs_path).unwrap_or_default();
            if existing == f.content {
                println!(
                    "{} {} {} {}",
                    style!(C_CYAN, "\u{2502}"),
                    icon,
                    style!(C_WHITE_B, "{}", f.path),
                    style!(C_DIM, "(unchanged)")
                );
            } else {
                let added = f
                    .content
                    .lines()
                    .count()
                    .saturating_sub(existing.lines().count());
                let removed = existing
                    .lines()
                    .count()
                    .saturating_sub(f.content.lines().count());
                println!(
                    "{} {} {} {}",
                    style!(C_CYAN, "\u{2502}"),
                    icon,
                    style!(C_WHITE_B, "{}", f.path),
                    style!(C_DIM, "")
                );
                if added > 0 {
                    println!(
                        "{}   {} {}",
                        style!(C_CYAN, "\u{2502}"),
                        style!(C_GREEN, "+{} lines", added),
                        style!(C_DIM, "")
                    );
                }
                if removed > 0 {
                    println!(
                        "{}   {} {}",
                        style!(C_CYAN, "\u{2502}"),
                        style!(C_RED, "-{} lines", removed),
                        style!(C_DIM, "")
                    );
                }
                let old_lines: Vec<&str> = existing.lines().collect();
                let new_lines: Vec<&str> = f.content.lines().collect();
                let max_lines = old_lines.len().max(new_lines.len());
                let mut diff_printed = 0;
                for i in 0..max_lines {
                    let old = old_lines.get(i).copied().unwrap_or("");
                    let new = new_lines.get(i).copied().unwrap_or("");
                    if old != new && diff_printed < 8 {
                        if i < old_lines.len() && !old.is_empty() {
                            println!(
                                "{}     {} {}",
                                style!(C_DIM, "\u{2502}"),
                                style!(C_RED, "-"),
                                style!(C_RED, "{}", old)
                            );
                        }
                        if i < new_lines.len() && !new.is_empty() {
                            println!(
                                "{}     {} {}",
                                style!(C_DIM, "\u{2502}"),
                                style!(C_GREEN, "+"),
                                style!(C_GREEN, "{}", new)
                            );
                        }
                        diff_printed += 1;
                    }
                }
                if max_lines > 8 && diff_printed > 0 {
                    println!("{}     {}", style!(C_DIM, "\u{2502}"), style!(C_DIM, "..."));
                }
            }
        } else {
            println!(
                "{} {} {} {}",
                style!(C_CYAN, "\u{2502}"),
                icon,
                style!(C_WHITE_B, "{}", f.path),
                style!(C_GREEN, "(new file) {} bytes", f.content.len())
            );
        }
    }

    let cmd = std::process::Command::new("git")
        .args(["diff", "--stat", "--"])
        .current_dir(&base_dir)
        .output();

    if let Ok(output) = cmd {
        if !output.stdout.is_empty() {
            println!("{}", style!(C_CYAN, "\u{2502}"));
            println!(
                "{} {}",
                style!(C_CYAN, "\u{2502}"),
                style!(C_DIM, "git diff --stat:")
            );
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                println!(
                    "{}   {}",
                    style!(C_CYAN, "\u{2502}"),
                    style!(C_DIM, "{}", line)
                );
            }
        }
    }

    println!("{}", style!(C_CYAN, "\u{2502}"));
}

fn handle_tokens(session: &ChatSession) {
    let tokens = session.last_tokens;
    let elapsed = session.last_elapsed.as_secs_f64();
    let history_tokens: usize = session
        .history
        .iter()
        .map(|(u, a)| (u.len() + a.len()) / 4)
        .sum();

    println!("{}", style!(C_DIM, "\u{2502}"));
    println!(
        "{}  {}{}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_WHITE_B, "\u{2500}\u{2500} TOKENS \u{2500}\u{2500}"),
        style!(C_DIM, "")
    );
    println!(
        "{}   {:<18} {}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_WHITE_B, "last response:"),
        style!(C_DIM, "~{} tokens", tokens)
    );

    if elapsed > 0.0 && tokens > 0 {
        let tps = tokens as f64 / elapsed;
        println!(
            "{}   {:<18} {}",
            style!(C_CYAN, "\u{2502}"),
            style!(C_WHITE_B, "speed:"),
            style!(C_DIM, "~{:.0} tok/s", tps)
        );
    }

    if session.last_elapsed.as_secs() > 0 {
        println!(
            "{}   {:<18} {}",
            style!(C_CYAN, "\u{2502}"),
            style!(C_WHITE_B, "elapsed:"),
            style!(C_DIM, "{:.1}s", elapsed)
        );
    }

    if history_tokens > 0 {
        println!(
            "{}   {:<18} {}",
            style!(C_CYAN, "\u{2502}"),
            style!(C_WHITE_B, "context history:"),
            style!(
                C_DIM,
                "~{} tokens ({} turns)",
                history_tokens,
                session.history.len()
            )
        );

        // Display uses the new scaled default (was hardcoded 2048). In future this should come from
        // the active Provider or ChatSession (after we store model_ctx + actual prompt budget).
        let pct = (history_tokens as f64 / 4096.0 * 100.0).min(100.0);
        println!(
            "{}   {:<18} {}",
            style!(C_CYAN, "\u{2502}"),
            style!(C_WHITE_B, "context window:"),
            style!(C_DIM, "{:.0}% used (4096 limit)", pct)
        );
    } else {
        println!(
            "{}   {:<18} {}",
            style!(C_CYAN, "\u{2502}"),
            style!(C_WHITE_B, "context:"),
            style!(C_DIM, "empty (no history)")
        );
    }
    println!("{}", style!(C_DIM, "\u{2502}"));
}

fn handle_memory(session: &ChatSession) {
    println!("{}", style!(C_DIM, "\u{2502}"));
    println!(
        "{}  {}{}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_WHITE_B, "\u{2500}\u{2500} MEMORY \u{2500}\u{2500}"),
        style!(C_DIM, "")
    );
    if session.project_memory.loaded && !session.project_memory.content.is_empty() {
        for line in session.project_memory.content.lines() {
            println!(
                "{} {}",
                style!(C_CYAN, "\u{2502}"),
                style!(C_DIM, "{}", line)
            );
        }
    } else {
        println!(
            "{} {} {}",
            style!(C_CYAN, "\u{2502}"),
            style!(C_DIM, "no project memory yet."),
            style!(C_DIM, "")
        );
        println!(
            "{} {} {}",
            style!(C_CYAN, "\u{2502}"),
            style!(C_DIM, ""),
            style!(C_DIM, "use /init to generate, or /memory add <text>")
        );
    }
    println!("{}", style!(C_DIM, "\u{2502}"));
    println!(
        "{} {} {}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_DIM, ""),
        style!(C_DIM, "/memory add <text>  /init  /memory clear")
    );
    println!("{}", style!(C_CYAN, "\u{2502}"));
}

fn handle_memory_set(session: &mut ChatSession, args: &str) {
    if args.eq_ignore_ascii_case("clear") {
        session.project_memory.content.clear();
        session.project_memory.loaded = false;
        let _ = session.project_memory.save();
        println!("{} memory cleared", style!(C_GREEN, "✓"));
        return;
    }
    if let Some(text) = args.strip_prefix("add ") {
        if let Err(e) = session.project_memory.append(text) {
            println!("{} failed: {}", style!(C_RED, "✗"), e);
        } else {
            println!(
                "{} appended to memory ({} bytes)",
                style!(C_GREEN, "✓"),
                text.len()
            );
        }
        return;
    }
    if let Err(e) = session.project_memory.set(args) {
        println!("{} failed: {}", style!(C_RED, "✗"), e);
    } else {
        println!(
            "{} memory saved ({} bytes)",
            style!(C_GREEN, "✓"),
            args.len()
        );
    }
}

fn handle_init(session: &mut ChatSession) {
    let dir = session
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let ptype = detect_project_type(&dir);
    let ptype_label = if ptype.is_empty() { "unknown" } else { ptype };
    println!("{}", style!(C_DIM, "│"));
    println!(
        "{} {}",
        style!(C_CYAN, "│"),
        style!(C_WHITE_B, "detected project type: {}", ptype_label)
    );
    println!(
        "{} {}",
        style!(C_CYAN, "│"),
        style!(C_DIM, "generating .rem/memory.md...")
    );
    let starter = ProjectMemory::generate_starter(&dir, ptype);
    if let Err(e) = session.project_memory.set(&starter) {
        println!(
            "{} {} failed: {}",
            style!(C_RED, "│"),
            style!(C_RED, "✗"),
            e
        );
    } else {
        println!(
            "{} {} {} ({} bytes)",
            style!(C_GREEN, "│"),
            style!(C_GREEN, "✓"),
            style!(C_WHITE_B, ".rem/memory.md created"),
            starter.len()
        );
        println!(
            "{} {} use {} to view",
            style!(C_CYAN, "│"),
            style!(C_DIM, ""),
            style!(C_WHITE_B, "/memory")
        );
    }
    println!("{}", style!(C_DIM, "│"));
}

async fn handle_compact(client: &Provider, session: &mut ChatSession) {
    if session.history.is_empty() {
        println!(
            "{} nothing to compact — history is empty",
            style!(C_YELLOW, "│")
        );
        return;
    }
    let history_text = session.build_chat_history();
    let compact_prompt = format!(
        "[SYSTEM] Summarize this conversation in 3-5 bullet points covering key decisions, code generated, and next actions. Be concise.\n\n{}",
        history_text
    );
    println!(
        "{} compacting {} turns...",
        style!(C_CYAN, "│"),
        session.history.len()
    );
    match client
        .complete_chat_stream(
            &compact_prompt,
            "You are a summarizer. Output only bullet-point summary. No preamble, no code.",
            "",
        )
        .await
    {
        Ok(summary) => {
            let old_count = session.history.len();
            session.history.clear();
            session.history.push((
                "[compacted summary]".to_string(),
                summary.trim().to_string(),
            ));
            println!(
                "{} {} {} → {} turns",
                style!(C_GREEN, "│"),
                style!(C_GREEN, "✓ compacted:"),
                old_count,
                session.history.len()
            );
        }
        Err(e) => {
            println!(
                "{} {} compact failed: {}",
                style!(C_RED, "│"),
                style!(C_RED, "✗"),
                e
            );
        }
    }
}

async fn handle_goal(client: &Provider, session: &mut ChatSession, condition: &str) {
    println!("{}", style!(C_DIM, "\u{2502}"));
    println!(
        "{} {} {}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_WHITE_B, "GOAL: {}", condition),
        style!(C_DIM, "")
    );
    println!(
        "{} {} {}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_DIM, ""),
        style!(C_DIM, "REM will work until goal is met. Ctrl+C to stop.")
    );
    println!("{}", style!(C_DIM, "\u{2502}"));

    let goal_prompt_text = format!(
        "GOAL: {}\n\nYour task is to achieve this goal. You may need to:\n\
         1. Plan your approach\n\
         2. Write code/files using ### path/file headings\n\
         3. We will run tests/linters and report back\n\
         4. Fix any issues based on tool output\n\n\
         When you believe the goal is achieved, say GOAL_ACHIEVED: <summary>.\n\
         If you are stuck, say GOAL_FAILED: <reason>.",
        condition
    );

    let max_iter = 10;
    let mut last_tool_output = String::new();
    let mut last_written_files: Vec<String> = Vec::new();

    for i in 0..max_iter {
        if i > 0 {
            println!("{}", style!(C_DIM, "\u{2502}"));
        }
        println!(
            "{} {} {}/{}",
            style!(C_CYAN, "\u{2502}"),
            style!(C_BOLD, "iteration"),
            i + 1,
            max_iter
        );

        let prompt = if last_tool_output.is_empty() {
            goal_prompt_text.clone()
        } else {
            build_agentic_prompt(&goal_prompt_text, &last_tool_output, i, max_iter)
        };

        match client
            .complete_chat_stream(&prompt, CHAT_SYSTEM_PROMPT_CODE, "")
            .await
        {
            Ok(text) => {
                let cleaned = text.trim().to_string();
                session
                    .history
                    .push((format!("/goal {}", condition), cleaned.clone()));

                let files = extract_code_blocks_with_names(&cleaned);
                let code = extract_code_block(&cleaned);
                if !files.is_empty() {
                    session.last_files = files.clone();
                    session.last_code = if code.is_empty() { String::new() } else { code };
                    auto_write_files(session, &files);
                    last_written_files = files.iter().map(|f| f.path.clone()).collect();
                } else if !code.is_empty() {
                    session.last_code = code;
                    session.last_files.clear();
                    println!(
                        "{} {} use /write <path> to save",
                        style!(C_CYAN, "\u{2502}"),
                        style!(C_DIM, "code detected \u{2014}")
                    );
                }

                if let Some((achieved, msg)) = extract_goal_signal(&cleaned) {
                    if achieved {
                        println!(
                            "{} {} goal achieved! {}",
                            style!(C_GREEN, "\u{2502}"),
                            style!(C_GREEN, "\u{2713}"),
                            msg
                        );
                    } else {
                        println!(
                            "{} {} {}",
                            style!(C_YELLOW, "\u{2502}"),
                            style!(C_YELLOW, "!"),
                            msg
                        );
                    }
                    break;
                }

                if cleaned.contains("GOAL_ACHIEVED") {
                    println!(
                        "{} {} goal achieved!",
                        style!(C_GREEN, "\u{2502}"),
                        style!(C_GREEN, "\u{2713}")
                    );
                    break;
                }
                if cleaned.contains("GOAL_FAILED") {
                    println!(
                        "{} {} goal could not be achieved.",
                        style!(C_YELLOW, "\u{2502}"),
                        style!(C_YELLOW, "!")
                    );
                    break;
                }

                if !last_written_files.is_empty() {
                    let mut tool_results = String::new();
                    for file_path in &last_written_files {
                        let lint_result = run_lint(file_path);
                        println!("{}", agentic::format_tool_output(&lint_result));

                        let test_result = run_test(file_path);
                        if !test_result.stderr.is_empty() || !test_result.stdout.is_empty() {
                            println!("{}", agentic::format_tool_output(&test_result));
                        }

                        tool_results.push_str(&build_tool_context(
                            Some(&lint_result),
                            Some(&test_result),
                            None,
                        ));
                    }
                    last_tool_output = tool_results;
                }
            }
            Err(e) => {
                println!(
                    "{} {} error: {}",
                    style!(C_RED, "\u{2502}"),
                    style!(C_RED, "\u{2717}"),
                    e
                );
                break;
            }
        }
    }
    println!("{}", style!(C_DIM, "\u{2502}"));
}

fn handle_copy(session: &ChatSession, n: usize) {
    let response = if n == 1 || session.history.is_empty() {
        session
            .history
            .last()
            .map(|(_, a)| a.as_str())
            .unwrap_or("")
    } else {
        let total = session.history.len();
        if n > total {
            println!(
                "{} only {} responses in history",
                style!(C_YELLOW, "│"),
                total
            );
            return;
        }
        session
            .history
            .get(total - n)
            .map(|(_, a)| a.as_str())
            .unwrap_or("")
    };

    if response.is_empty() {
        println!("{} nothing to copy", style!(C_YELLOW, "│"));
        return;
    }

    let use_clipboard = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("printf '%s' {:?} | xclip -selection clipboard 2>/dev/null || printf '%s' {:?} | xsel --clipboard 2>/dev/null || printf '%s' {:?} | pbcopy 2>/dev/null || echo 'no-clipboard'", response, response, response))
        .output();

    match use_clipboard {
        Ok(out) if String::from_utf8_lossy(&out.stdout).contains("no-clipboard") => {
            println!("{} copied to console:", style!(C_GREEN, "│ ✓"));
            println!("{}", style!(C_DIM, "│"));
            for line in response.lines().take(20) {
                println!("{} {}", style!(C_DIM, "│"), line);
            }
            if response.lines().count() > 20 {
                println!(
                    "{} ... ({} lines total)",
                    style!(C_DIM, "│"),
                    response.lines().count()
                );
            }
        }
        Ok(_) => {
            println!(
                "{} copied to clipboard ({} chars)",
                style!(C_GREEN, "│ ✓"),
                response.len()
            );
        }
        Err(_) => {
            println!(
                "{} copied to console ({}) — install xclip/xsel for clipboard",
                style!(C_GREEN, "│ ✓"),
                response.chars().count()
            );
            for line in response.lines().take(20) {
                println!("{} {}", style!(C_DIM, "│"), line);
            }
        }
    }
}

fn handle_lint(_session: &mut ChatSession, path: &str) {
    let file_path = Path::new(path);
    if !file_path.exists() {
        println!("{} file not found: {}", style!(C_YELLOW, "│"), path);
        return;
    }

    let path_str = file_path.display().to_string();
    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let cmd: (&str, Vec<String>) = match ext {
        "py" => (
            "python3",
            vec!["-m".into(), "py_compile".into(), path_str.clone()],
        ),
        "rs" => ("cargo", vec!["fmt".into(), "--check".into(), "--".into()]),
        "go" => ("go", vec!["fmt".into()]),
        "js" | "ts" => (
            "npx",
            vec!["--no-install".into(), "eslint".into(), path_str.clone()],
        ),
        "html" => (
            "npx",
            vec!["--no-install".into(), "htmlhint".into(), path_str.clone()],
        ),
        "css" => (
            "npx",
            vec!["--no-install".into(), "stylelint".into(), path_str.clone()],
        ),
        _ => {
            println!(
                "{} no linter configured for .{} files",
                style!(C_YELLOW, "│"),
                ext
            );
            return;
        }
    };

    println!("{} linting {}...", style!(C_CYAN, "│"), path);
    let output = std::process::Command::new(cmd.0).args(&cmd.1).output();

    match output {
        Ok(out) => {
            if out.status.success() {
                println!("{} {} passed lint", style!(C_GREEN, "│ ✓"), path);
            } else {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let stdout = String::from_utf8_lossy(&out.stdout);
                let msg = if stderr.trim().is_empty() {
                    stdout.trim().to_string()
                } else {
                    stderr.trim().to_string()
                };
                if msg.is_empty() {
                    println!(
                        "{} {} lint completed with warnings",
                        style!(C_YELLOW, "│ !"),
                        path
                    );
                } else {
                    println!("{} {} lint issues:", style!(C_YELLOW, "│ !"), path);
                    for line in msg.lines().take(10) {
                        println!("{}   {}", style!(C_YELLOW, "│"), line);
                    }
                }
            }
        }
        Err(e) => {
            println!(
                "{} lint failed: {} (is the tool installed?)",
                style!(C_YELLOW, "│"),
                e
            );
        }
    }
}

async fn handle_review(client: &Provider, session: &mut ChatSession) {
    if session.last_files.is_empty() {
        println!("{} no generated code to review", style!(C_YELLOW, "│"));
        return;
    }

    let mut code_for_review = String::new();
    for f in &session.last_files {
        if f.path.is_empty() {
            continue;
        }
        code_for_review.push_str(&format!(
            "\n### {}\n```\n{}\n```\n",
            f.path,
            truncate_bytes(&f.content, 3000)
        ));
    }
    if code_for_review.is_empty() && !session.last_code.is_empty() {
        code_for_review = format!("```\n{}\n```", truncate_bytes(&session.last_code, 3000));
    }
    if code_for_review.is_empty() {
        println!("{} no code to review", style!(C_YELLOW, "│"));
        return;
    }

    let review_prompt = format!(
        "Review the following code for:\n\
         1. Bugs & correctness issues\n\
         2. Code smells & anti-patterns\n\
         3. Security vulnerabilities\n\
         4. Missing error handling\n\
         5. Style & naming improvements\n\n\
         Be specific — reference line numbers where possible.\n\n{}",
        code_for_review
    );

    println!(
        "{} reviewing {} file(s)...",
        style!(C_CYAN, "│"),
        session.last_files.len()
    );
    match client.complete_chat_stream(
        &review_prompt,
        "[MODE: CHAT] You are a senior code reviewer. Review the code critically. Use clear markdown. Be specific.",
        "",
    ).await {
        Ok(response) => {
            println!();
            println!("{}", response);
            session.history.push(("/review".to_string(), response));
        }
        Err(e) => {
            println!("\n{} review failed: {}", style!(C_RED, "│"), e);
        }
    }
}

fn handle_find(session: &ChatSession, query: &str) {
    if query.is_empty() {
        println!("{}", style!(C_DIM, "\u{2502}"));
        println!(
            "{} {}",
            style!(C_CYAN, "\u{2502}"),
            style!(C_WHITE_B, "usage: /find <query>")
        );
        println!("{}", style!(C_DIM, "\u{2502}"));
        return;
    }

    let root = session
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let report = find_matches(&root, query, &FindOptions::default());

    println!("{}", style!(C_DIM, "\u{2502}"));
    println!(
        "{} {} {}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_WHITE_B, "\u{203a} FIND  {}", query),
        style!(C_DIM, "")
    );
    println!(
        "{} {} {}",
        style!(C_CYAN, "\u{2502}"),
        style!(C_DIM, "in"),
        style!(C_WHITE_B, "{}", root.display())
    );
    println!("{}", style!(C_DIM, "\u{2502}"));

    if report.matches.is_empty() {
        println!(
            "{} {}",
            style!(C_CYAN, "\u{2502}"),
            style!(C_YELLOW, "(no matches)")
        );
        println!("{}", style!(C_DIM, "\u{2502}"));
        return;
    }

    let show_limit = 50usize;
    let shown = report.matches.len().min(show_limit);
    let mut last_path: Option<String> = None;
    for m in report.matches.iter().take(show_limit) {
        let rel = m
            .path
            .strip_prefix(&root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| m.path.display().to_string());
        if last_path.as_deref() != Some(rel.as_str()) {
            if last_path.is_some() {
                println!("{}", style!(C_DIM, "\u{2502}"));
            }
            println!(
                "{} {} {}",
                style!(C_CYAN, "\u{2502}"),
                style!(C_BLUE, "\u{2500}\u{2500} {} \u{2500}\u{2500}", rel),
                style!(C_DIM, "")
            );
            last_path = Some(rel);
        }
        let line_no_w = 4usize;
        let col_w = 3usize;
        println!(
            "{} {}   {:>lw$}:{:<cw$}  {}",
            style!(C_CYAN, "\u{2502}"),
            style!(C_DIM, "\u{251c}\u{2500}\u{2500}"),
            m.line_no,
            m.column,
            style!(C_WHITE_B, "{}", trim_for_display(&m.line, 120)),
            lw = line_no_w,
            cw = col_w
        );
    }
    println!("{}", style!(C_DIM, "\u{2502}"));

    let unique_files: std::collections::BTreeSet<String> = report
        .matches
        .iter()
        .map(|m| {
            m.path
                .strip_prefix(&root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| m.path.display().to_string())
        })
        .collect();

    let mut summary = format!(
        "  {} match{} in {} file{} \u{00b7} scanned {} \u{00b7} skipped {} \u{00b7} {}ms",
        report.matches.len(),
        if report.matches.len() == 1 { "" } else { "es" },
        unique_files.len(),
        if unique_files.len() == 1 { "" } else { "s" },
        report.files_scanned,
        report.files_skipped,
        report.elapsed_ms,
    );
    if report.truncated {
        summary.push_str("  (truncated)");
    }
    if shown < report.matches.len() {
        summary.push_str(&format!("  (showing first {})", shown));
    }
    println!("{}", style!(C_DIM, "\u{2502}"));
    println!("{}", style!(C_GREEN, "{}", summary));
    println!("{}", style!(C_DIM, "\u{2502}"));
}

fn trim_for_display(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('\u{2026}');
        out
    }
}

fn handle_save_session(session: &ChatSession) {
    let dir = session
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let rem_dir = dir.join(".rem");
    let _ = fs::create_dir_all(&rem_dir);
    let session_file = rem_dir.join("session.json");
    let last_files_json: Vec<serde_json::Value> = session
        .last_files
        .iter()
        .map(|f| serde_json::json!({"path": f.path, "content": f.content}))
        .collect();
    let data = serde_json::json!({
        "history": session.history.iter().map(|(u, a)| serde_json::json!({"user": u, "assistant": a})).collect::<Vec<_>>(),
        "mode": session.mode.label(),
        "workspace": session.project_dir.as_ref().map(|d| d.display().to_string()),
        "saved_at": chrono_now(),
        "last_code": session.last_code,
        "last_files": last_files_json,
        "last_files_written": session.last_files_written.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    });
    match fs::write(
        &session_file,
        serde_json::to_string_pretty(&data).unwrap_or_default(),
    ) {
        Ok(()) => println!(
            "{} session saved to {}",
            style!(C_GREEN, "\u{2713}"),
            session_file.display()
        ),
        Err(e) => println!(
            "{} failed to save session: {}",
            style!(C_RED, "\u{2717}"),
            e
        ),
    }
}

fn chrono_now() -> String {
    std::process::Command::new("date")
        .arg("+%Y-%m-%d %H:%M:%S")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn handle_resume_session(session: &mut ChatSession) {
    let dir = session
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let session_file = dir.join(".rem/session.json");
    if !session_file.exists() {
        println!(
            "{} no saved session found at {}",
            style!(C_YELLOW, "\u{2502}"),
            session_file.display()
        );
        return;
    }
    match fs::read_to_string(&session_file) {
        Ok(content) => {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(history) = data["history"].as_array() {
                    let mut restored = 0;
                    for entry in history {
                        if let (Some(u), Some(a)) =
                            (entry["user"].as_str(), entry["assistant"].as_str())
                        {
                            session.history.push((u.to_string(), a.to_string()));
                            restored += 1;
                        }
                    }
                    println!(
                        "{} restored {} turns from {}",
                        style!(C_GREEN, "\u{2713}"),
                        restored,
                        session_file.display()
                    );
                    println!(
                        "{} current conversation is now merged with saved session",
                        style!(C_DIM, "\u{2502}")
                    );
                }
                if let Some(m) = data["mode"].as_str() {
                    println!(
                        "{} {} {}",
                        style!(C_DIM, "\u{2502}"),
                        style!(C_DIM, "saved mode:"),
                        style!(C_WHITE_B, "{}", m)
                    );
                }
                if let Some(code) = data["last_code"].as_str() {
                    if !code.is_empty() {
                        session.last_code = code.to_string();
                        println!(
                            "{} {} {}",
                            style!(C_DIM, "\u{2502}"),
                            style!(C_DIM, "last code:"),
                            style!(C_GREEN, "restored")
                        );
                    }
                }
                if let Some(files) = data["last_files"].as_array() {
                    let restored_files: Vec<FileEntry> = files
                        .iter()
                        .filter_map(|f| {
                            Some(FileEntry {
                                path: f["path"].as_str()?.to_string(),
                                content: f["content"].as_str()?.to_string(),
                            })
                        })
                        .collect();
                    if !restored_files.is_empty() {
                        println!(
                            "{} {} {} file(s) restored",
                            style!(C_DIM, "\u{2502}"),
                            style!(C_DIM, "last files:"),
                            restored_files.len()
                        );
                        session.last_files = restored_files;
                    }
                }
                if let Some(paths) = data["last_files_written"].as_array() {
                    let written: Vec<PathBuf> = paths
                        .iter()
                        .filter_map(|p| p.as_str().map(PathBuf::from))
                        .collect();
                    if !written.is_empty() {
                        session.last_files_written = written;
                    }
                }
            } else {
                println!("{} invalid session file", style!(C_RED, "\u{2502}"));
            }
        }
        Err(e) => println!(
            "{} failed to read session: {}",
            style!(C_RED, "\u{2502}"),
            e
        ),
    }
}

fn print_chat_help() {
    let v = C_CYAN;
    let d = C_DIM;
    let h = C_WHITE_B;
    println!("{}", style!(d, "\u{2502}"));
    println!(
        "{}  {}{}",
        style!(v, "\u{2502}"),
        style!(h, "\u{2500}\u{2500} COMMANDS \u{2500}\u{2500}"),
        style!(d, "")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/help"),
        style!(d, "show this help")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/mode"),
        style!(d, "toggle CHAT → CODE → PLAN")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/plan"),
        style!(d, "switch to PLAN mode (explore & analyze)")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/clear"),
        style!(d, "reset conversation history")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/explain <code>"),
        style!(d, "explain what code does")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/test <file>"),
        style!(d, "generate tests for a file")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/refactor <file>"),
        style!(d, "suggest refactoring for a file")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/write <path>"),
        style!(d, "save last code to file")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/save <path>"),
        style!(d, "same as /write")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/dir <path>"),
        style!(d, "set project root")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/search <q>"),
        style!(d, "search the web (DuckDuckGo)")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/code"),
        style!(d, "show last generated code")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/files"),
        style!(d, "list project files tree")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/undo"),
        style!(d, "delete last written files")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/diff"),
        style!(d, "compare generated vs existing files")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/tokens"),
        style!(d, "show token usage & context stats")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/config"),
        style!(d, "view current configuration")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/memory"),
        style!(d, "view/set project memory (.rem/memory.md)")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/init"),
        style!(d, "auto-generate project memory file")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/compact"),
        style!(d, "summarize & free context window")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/goal <cond>"),
        style!(d, "autonomous loop until goal is met")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/copy [N]"),
        style!(d, "copy last response to clipboard")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/lint [file]"),
        style!(d, "run linter on generated files")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/review"),
        style!(d, "AI code review of generated code")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/find <q>"),
        style!(d, "search text inside the project")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/reset"),
        style!(d, "full reset — clear history & code cache")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/save"),
        style!(d, "save current session to .rem/session.json")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/resume"),
        style!(d, "restore saved session history")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "/why"),
        style!(d, "show why last intent was chosen")
    );
    println!(
        "{}   {:<18} {}",
        style!(v, "\u{2502}"),
        style!(h, "exit / quit"),
        style!(d, "exit REM")
    );
    println!("{}", style!(v, "\u{2502}"));
    println!(
        "{}  {}",
        style!(v, "\u{2502}"),
        style!(h, "\u{2500}\u{2500} TIPS \u{2500}\u{2500}")
    );
    println!(
        "{}   {} use {} to include file context: {}",
        style!(v, "\u{2502}"),
        style!(d, "\u{2022}"),
        style!(h, "@<path>"),
        style!(d, "@src/main.rs")
    );
    println!(
        "{}   {} use {} to toggle between chat, code, and plan modes",
        style!(v, "\u{2502}"),
        style!(d, "\u{2022}"),
        style!(h, "/mode")
    );
    println!(
        "{}   {} {} for analysis first — REM explores codebase before coding",
        style!(v, "\u{2502}"),
        style!(d, "\u{2022}"),
        style!(h, "/plan")
    );
    println!(
        "{}   {} describe what you want \u{2014} REM detects intent",
        style!(v, "\u{2502}"),
        style!(d, "\u{2022}")
    );
    println!(
        "{}   {} multi-file intent and auto-writes after confirmation",
        style!(v, "\u{2502}"),
        style!(d, "\u{2022}")
    );
    println!(
        "{}   {} use {} {} {} for analysis, tests, and refactoring",
        style!(v, "\u{2502}"),
        style!(d, "\u{2022}"),
        style!(h, "/explain"),
        style!(h, "/test"),
        style!(h, "/refactor")
    );
    println!(
        "{}   {} run {} for persistent project memory across sessions",
        style!(v, "\u{2502}"),
        style!(d, "\u{2022}"),
        style!(h, "/init")
    );
    println!(
        "{}   {} run {} to scaffold a new project instantly",
        style!(v, "\u{2502}"),
        style!(d, "\u{2022}"),
        style!(h, "rem new <name>")
    );
    println!("{}", style!(v, "\u{2502}"));
}

// ── Output formatting ──────────────────────────────────────────────────────

fn print_banner(client: &Provider) {
    println!();
    println!(
        "{} {} {}",
        style!(C_CYAN, "╭─"),
        style!(C_BOLD, "REM"),
        style!(C_DIM, "─────────────────────────────")
    );
    println!(
        "{} model {}{} {}",
        style!(C_CYAN, "│"),
        style!(C_GREEN, "{}", client.model),
        style!(C_DIM, " •"),
        style!(C_DIM, "type /help for commands")
    );
}

fn print_reply(reply: &ModelReply, newline: bool) {
    if newline {
        println!();
    }
    if !reply.explanation.trim().is_empty() {
        println!(
            "{} {}",
            style!(C_CYAN, "│"),
            style!(C_WHITE_B, "{}", reply.explanation)
        );
        println!("{}", style!(C_CYAN, "│"));
    }

    if !reply.files.is_empty() {
        println!(
            "{} {} ({} file{})",
            style!(C_CYAN, "│  ┌"),
            style!(C_GREEN, "generated:"),
            reply.files.len(),
            if reply.files.len() == 1 { "" } else { "s" }
        );
        for f in &reply.files {
            let icon = file_icon(&f.path);
            if f.path.is_empty() {
                println!(
                    "{} {}  {}",
                    style!(C_CYAN, "│"),
                    icon,
                    style!(C_GREEN, "(unnamed) {} bytes", f.content.len())
                );
            } else {
                println!(
                    "{} {}  {}",
                    style!(C_CYAN, "│"),
                    icon,
                    style!(C_GREEN, "{}", f.path)
                );
            }
        }
        println!(
            "{}   {}",
            style!(C_CYAN, "│  └"),
            style!(C_DIM, "/write <path> to save")
        );
        println!("{}", style!(C_CYAN, "│"));
    } else if !reply.code.trim().is_empty() {
        println!("{} {}", style!(C_CYAN, "│  ┌"), style!(C_GREEN, "code:"));
        for code_line in reply.code.lines() {
            println!(
                "{} {}",
                style!(C_CYAN, "│"),
                style!(C_GREEN, "  │  {}", code_line)
            );
        }
        println!(
            "{}   {}",
            style!(C_CYAN, "│  └"),
            style!(C_DIM, "/write <path> to save")
        );
        println!("{}", style!(C_CYAN, "│"));
    }
    if !reply.commands.is_empty() {
        println!("{} {}", style!(C_CYAN, "│"), style!(C_MAGENTA, "commands:"));
        for cmd in sanitize_commands(&reply.commands) {
            if is_command_blocked(cmd) {
                println!(
                    "{}   {}",
                    style!(C_CYAN, "│"),
                    style!(C_RED, "[blocked] {}", cmd)
                );
            } else {
                println!(
                    "{}   {}",
                    style!(C_CYAN, "│"),
                    style!(C_MAGENTA, "  $ {}", cmd)
                );
            }
        }
        println!("{}", style!(C_CYAN, "│"));
    }
    if !reply.checks.is_empty() {
        println!("{} {}", style!(C_CYAN, "│"), style!(C_YELLOW, "checks:"));
        for item in &reply.checks {
            println!(
                "{}   {}",
                style!(C_CYAN, "│"),
                style!(C_DIM, "  • {}", item)
            );
        }
        println!("{}", style!(C_CYAN, "│"));
    }
    if !reply.caution.trim().is_empty() {
        println!(
            "{} {}",
            style!(C_CYAN, "│"),
            style!(C_RED, "caution: {}", reply.caution)
        );
        println!("{}", style!(C_CYAN, "│"));
    }
    println!("{}", style!(C_CYAN, "╰──────────────────────────────────"));
}

fn file_icon(path: &str) -> String {
    let lower = path.to_lowercase();
    if lower.ends_with(".html") || lower.ends_with(".htm") {
        style!(C_MAGENTA, "🌐")
    } else if lower.ends_with(".css") {
        style!(C_BLUE, "🎨")
    } else if lower.ends_with(".js") || lower.ends_with(".mjs") || lower.ends_with(".ts") {
        style!(C_YELLOW, "⚡")
    } else if lower.ends_with(".json") {
        style!(C_CYAN, "📋")
    } else if lower.ends_with(".md") || lower.ends_with(".txt") {
        style!(C_DIM, "📄")
    } else if lower.ends_with(".py") {
        style!(C_GREEN, "🐍")
    } else {
        style!(C_DIM, "📄")
    }
}

fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    }
}

// ── Context builder ────────────────────────────────────────────────────────

fn build_context(target: &Path, max_bytes: usize) -> Result<String> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let mut out = String::from("Directory snapshot:\n");
    for entry in WalkDir::new(parent).max_depth(2) {
        let entry = entry?;
        let p = entry.path();
        let rel = p.strip_prefix(parent).unwrap_or(p);
        if rel.as_os_str().is_empty() {
            continue;
        }
        out.push_str(&format!("- {}\n", rel.display()));
        if out.len() > max_bytes {
            break;
        }
    }
    if target.exists() {
        let content = fs::read_to_string(target)
            .with_context(|| format!("failed to read {}", target.display()))?;
        out.push_str("\nTarget file:\n");
        out.push_str(&truncate_bytes(&content, max_bytes / 2));
    }
    Ok(truncate_bytes(&out, max_bytes))
}

fn truncate_bytes(s: &str, max: usize) -> String {
    if max == 0 || s.is_empty() {
        return "[truncated]".to_string();
    }
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    if end == 0 {
        return "[truncated]".to_string();
    }
    format!("{}\n...[truncated]", &s[..end])
}

// ── Project scaffolding ────────────────────────────────────────────────────

fn run_new(args: NewArgs, cfg: &AppConfig) -> Result<()> {
    let dir = if args.name.starts_with('/')
        || args.name.starts_with("./")
        || args.name.starts_with("../")
    {
        PathBuf::from(&args.name)
    } else if let Some(ref ws) = cfg.workspace_dir {
        let base = PathBuf::from(ws);
        base.join(&args.name)
    } else {
        PathBuf::from(&args.name)
    };

    if dir.exists() {
        return Err(anyhow!(
            "Directory '{}' already exists. Choose a different name.",
            dir.display()
        ));
    }

    let files = match args.project_type.as_str() {
        "bare" => template_bare(&args.name),
        "portfolio" => template_portfolio(&args.name),
        "landing" => template_landing(&args.name),
        "blog" => template_blog(&args.name),
        other => {
            return Err(anyhow!(
                "Unknown project type '{}'. Choose: bare, portfolio, landing, blog",
                other
            ))
        }
    };

    for file in &files {
        let file_path = dir.join(&file.path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&file_path, &file.content)?;
    }

    println!(
        "{} {}",
        style!(C_GREEN, "✓"),
        style!(
            C_WHITE_B,
            "created project '{}' ({})",
            args.name,
            args.project_type
        )
    );
    for f in &files {
        let icon = file_icon(&f.path);
        println!(
            "  {} {} ({} bytes)",
            icon,
            style!(C_WHITE_B, "{}", f.path),
            f.content.len()
        );
    }
    println!();
    println!(
        "{} cd {} && open index.html",
        style!(C_DIM, "next:"),
        args.name
    );

    Ok(())
}

fn template_bare(name: &str) -> Vec<FileEntry> {
    let title = name.rsplit('/').next().unwrap_or(name);
    vec![
        FileEntry {
            path: "index.html".into(),
            content: format!(
                r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    <link rel="stylesheet" href="style.css">
</head>
<body>
    <header>
        <h1>{title}</h1>
        <nav>
            <a href="#">Home</a>
            <a href="#">About</a>
            <a href="#">Contact</a>
        </nav>
    </header>

    <main>
        <section class="hero">
            <h2>Welcome to {title}</h2>
            <p>Start building something amazing.</p>
        </section>
    </main>

    <footer>
        <p>&copy; 2026 {title}</p>
    </footer>

    <script src="script.js"></script>
</body>
</html>"##,
                title = title
            ),
        },
        FileEntry {
            path: "style.css".into(),
            content: r##"/* ── Reset ─────────────────────── */
* {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
}

/* ── Layout ────────────────────── */
body {
    font-family: system-ui, -apple-system, sans-serif;
    line-height: 1.6;
    color: #333;
    min-height: 100vh;
    display: flex;
    flex-direction: column;
}

header {
    background: #1a1a2e;
    color: #fff;
    padding: 1rem 2rem;
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-wrap: wrap;
    gap: 1rem;
}

header h1 {
    font-size: 1.5rem;
}

nav {
    display: flex;
    gap: 1.5rem;
}

nav a {
    color: #a0a0c0;
    text-decoration: none;
    transition: color 0.2s;
}

nav a:hover {
    color: #fff;
}

main {
    flex: 1;
    padding: 2rem;
}

.hero {
    text-align: center;
    padding: 4rem 1rem;
}

.hero h2 {
    font-size: 2rem;
    margin-bottom: 0.5rem;
}

.hero p {
    color: #666;
    font-size: 1.1rem;
}

footer {
    background: #f5f5f5;
    text-align: center;
    padding: 1rem;
    color: #888;
    font-size: 0.9rem;
}

/* ── Responsive ────────────────── */
@media (max-width: 600px) {
    header {
        flex-direction: column;
        text-align: center;
    }

    .hero {
        padding: 2rem 1rem;
    }

    .hero h2 {
        font-size: 1.5rem;
    }
}
"##
            .into(),
        },
        FileEntry {
            path: "script.js".into(),
            content: r##"// ── Main ────────────────────────
document.addEventListener('DOMContentLoaded', () => {
    console.log('App ready');
});

// ── Navigation ───────────────────
document.querySelectorAll('nav a').forEach(link => {
    link.addEventListener('click', (e) => {
        e.preventDefault();
        console.log(`Navigate to: ${link.textContent}`);
    });
});
"##
            .into(),
        },
    ]
}

fn template_portfolio(name: &str) -> Vec<FileEntry> {
    let title = name.rsplit('/').next().unwrap_or(name);
    let mut files = template_bare(name);
    files.push(FileEntry {
        path: "about.html".into(),
        content: format!(
            r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>About — {title}</title>
    <link rel="stylesheet" href="style.css">
</head>
<body>
    <header>
        <h1>{title}</h1>
        <nav>
            <a href="index.html">Home</a>
            <a href="about.html">About</a>
            <a href="projects.html">Projects</a>
            <a href="contact.html">Contact</a>
        </nav>
    </header>

    <main>
        <section class="hero">
            <h2>About Me</h2>
            <p>I'm a web developer passionate about building clean, accessible websites.</p>
        </section>

        <section class="content">
            <h3>Skills</h3>
            <ul>
                <li>HTML, CSS, JavaScript</li>
                <li>React & Node.js</li>
                <li>Git & GitHub</li>
            </ul>
        </section>
    </main>

    <footer>
        <p>&copy; 2026 {title}</p>
    </footer>
</body>
</html>"##,
            title = title
        ),
    });
    files.push(FileEntry {
        path: "projects.html".into(),
        content: format!(
            r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Projects — {title}</title>
    <link rel="stylesheet" href="style.css">
</head>
<body>
    <header>
        <h1>{title}</h1>
        <nav>
            <a href="index.html">Home</a>
            <a href="about.html">About</a>
            <a href="projects.html">Projects</a>
            <a href="contact.html">Contact</a>
        </nav>
    </header>

    <main>
        <section class="hero">
            <h2>Projects</h2>
            <p>Things I've built.</p>
        </section>

        <section class="projects-grid">
            <article class="project-card">
                <h3>Project One</h3>
                <p>A web application built with React and Node.js.</p>
                <a href="#">View on GitHub &rarr;</a>
            </article>

            <article class="project-card">
                <h3>Project Two</h3>
                <p>A responsive landing page built with HTML/CSS.</p>
                <a href="#">View on GitHub &rarr;</a>
            </article>

            <article class="project-card">
                <h3>Project Three</h3>
                <p>A CLI tool written in Rust.</p>
                <a href="#">View on GitHub &rarr;</a>
            </article>
        </section>
    </main>

    <footer>
        <p>&copy; 2026 {title}</p>
    </footer>
</body>
</html>"##,
            title = title
        ),
    });
    files.push(FileEntry {
        path: "contact.html".into(),
        content: format!(
            r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Contact — {title}</title>
    <link rel="stylesheet" href="style.css">
</head>
<body>
    <header>
        <h1>{title}</h1>
        <nav>
            <a href="index.html">Home</a>
            <a href="about.html">About</a>
            <a href="projects.html">Projects</a>
            <a href="contact.html">Contact</a>
        </nav>
    </header>

    <main>
        <section class="hero">
            <h2>Contact</h2>
            <p>Get in touch — I'd love to hear from you.</p>
        </section>

        <section class="content">
            <form id="contact-form">
                <label for="name">Name</label>
                <input type="text" id="name" required>

                <label for="email">Email</label>
                <input type="email" id="email" required>

                <label for="message">Message</label>
                <textarea id="message" rows="5" required></textarea>

                <button type="submit">Send</button>
            </form>
        </section>
    </main>

    <footer>
        <p>&copy; 2026 {title}</p>
    </footer>
</body>
</html>"##,
            title = title
        ),
    });
    files.push(FileEntry {
        path: "style.css".into(),
        content: r##"/* ── Reset ─────────────────────── */
* {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
}

body {
    font-family: system-ui, -apple-system, sans-serif;
    line-height: 1.6;
    color: #333;
    min-height: 100vh;
    display: flex;
    flex-direction: column;
}

header {
    background: #1a1a2e;
    color: #fff;
    padding: 1rem 2rem;
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-wrap: wrap;
    gap: 1rem;
}

header h1 {
    font-size: 1.5rem;
}

nav {
    display: flex;
    gap: 1.5rem;
}

nav a {
    color: #a0a0c0;
    text-decoration: none;
    transition: color 0.2s;
}

nav a:hover {
    color: #fff;
}

main {
    flex: 1;
    padding: 2rem;
    max-width: 900px;
    margin: 0 auto;
    width: 100%;
}

.hero {
    text-align: center;
    padding: 4rem 1rem 2rem;
}

.hero h2 {
    font-size: 2rem;
    margin-bottom: 0.5rem;
}

.hero p {
    color: #666;
    font-size: 1.1rem;
}

.content {
    padding: 1rem 0;
}

.content ul {
    list-style: disc;
    padding-left: 1.5rem;
    color: #555;
}

.content li {
    margin: 0.5rem 0;
}

.projects-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
    gap: 1.5rem;
    padding: 1rem 0;
}

.project-card {
    border: 1px solid #e0e0e0;
    border-radius: 8px;
    padding: 1.5rem;
    transition: box-shadow 0.2s;
}

.project-card:hover {
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.08);
}

.project-card h3 {
    margin-bottom: 0.5rem;
}

.project-card p {
    color: #666;
    margin-bottom: 0.75rem;
}

.project-card a {
    color: #1a1a2e;
    font-weight: 600;
    text-decoration: none;
}

form {
    max-width: 500px;
    display: flex;
    flex-direction: column;
    gap: 1rem;
}

form label {
    font-weight: 600;
    color: #555;
}

form input,
form textarea {
    padding: 0.75rem;
    border: 1px solid #ddd;
    border-radius: 6px;
    font-size: 1rem;
}

form button {
    padding: 0.75rem 1.5rem;
    background: #1a1a2e;
    color: #fff;
    border: none;
    border-radius: 6px;
    font-size: 1rem;
    cursor: pointer;
    transition: background 0.2s;
}

form button:hover {
    background: #2a2a4e;
}

footer {
    background: #f5f5f5;
    text-align: center;
    padding: 1rem;
    color: #888;
    font-size: 0.9rem;
}

@media (max-width: 600px) {
    header {
        flex-direction: column;
        text-align: center;
    }

    .hero {
        padding: 2rem 1rem;
    }

    .hero h2 {
        font-size: 1.5rem;
    }

    .projects-grid {
        grid-template-columns: 1fr;
    }
}
"##
        .into(),
    });
    files
}

fn template_landing(name: &str) -> Vec<FileEntry> {
    let title = name.rsplit('/').next().unwrap_or(name);
    vec![
        FileEntry {
            path: "index.html".into(),
            content: format!(
                r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    <link rel="stylesheet" href="style.css">
</head>
<body>
    <header>
        <div class="container">
            <h1 class="logo">{title}</h1>
            <nav>
                <a href="#features">Features</a>
                <a href="#pricing">Pricing</a>
                <a href="#cta" class="btn-nav">Get Started</a>
            </nav>
        </div>
    </header>

    <section class="hero">
        <div class="container">
            <h2>Build Something Great</h2>
            <p class="hero-subtitle">The easiest way to launch your next project. Clean, fast, and beautiful out of the box.</p>
            <div class="hero-actions">
                <a href="#cta" class="btn btn-primary">Start Free Trial</a>
                <a href="#features" class="btn btn-secondary">Learn More</a>
            </div>
        </div>
    </section>

    <section id="features" class="features">
        <div class="container">
            <h3>Why choose {title}?</h3>
            <div class="features-grid">
                <div class="feature-card">
                    <div class="feature-icon">⚡</div>
                    <h4>Fast</h4>
                    <p>Lightning-quick performance with zero configuration.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">🔒</div>
                    <h4>Secure</h4>
                    <p>Enterprise-grade security baked in from day one.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">🎨</div>
                    <h4>Beautiful</h4>
                    <p>Stunning, responsive designs that work everywhere.</p>
                </div>
            </div>
        </div>
    </section>

    <section id="cta" class="cta">
        <div class="container">
            <h3>Ready to start?</h3>
            <p>Join thousands of developers building with {title}.</p>
            <a href="#" class="btn btn-primary">Get Started Free</a>
        </div>
    </section>

    <footer>
        <div class="container">
            <p>&copy; 2026 {title}. All rights reserved.</p>
        </div>
    </footer>

    <script src="script.js"></script>
</body>
</html>"##,
                title = title
            ),
        },
        FileEntry {
            path: "style.css".into(),
            content: r##"* {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
}

:root {
    --primary: #6366f1;
    --primary-dark: #4f46e5;
    --bg: #ffffff;
    --text: #1f2937;
    --text-muted: #6b7280;
    --border: #e5e7eb;
}

body {
    font-family: system-ui, -apple-system, sans-serif;
    color: var(--text);
    line-height: 1.6;
}

.container {
    max-width: 1100px;
    margin: 0 auto;
    padding: 0 1.5rem;
}

/* ── Header ─────────────────────── */
header {
    background: var(--bg);
    border-bottom: 1px solid var(--border);
    padding: 1rem 0;
    position: sticky;
    top: 0;
    z-index: 100;
}

header .container {
    display: flex;
    justify-content: space-between;
    align-items: center;
}

.logo {
    font-size: 1.4rem;
    font-weight: 700;
}

nav {
    display: flex;
    align-items: center;
    gap: 1.5rem;
}

nav a {
    color: var(--text);
    text-decoration: none;
    font-weight: 500;
    transition: color 0.2s;
}

nav a:hover {
    color: var(--primary);
}

.btn-nav {
    background: var(--primary);
    color: #fff;
    padding: 0.5rem 1.25rem;
    border-radius: 8px;
}

.btn-nav:hover {
    color: #fff !important;
    background: var(--primary-dark);
}

/* ── Hero ───────────────────────── */
.hero {
    padding: 6rem 0;
    text-align: center;
    background: linear-gradient(135deg, #f0f4ff 0%, #e8ecff 100%);
}

.hero h2 {
    font-size: 3rem;
    font-weight: 800;
    margin-bottom: 1rem;
}

.hero-subtitle {
    font-size: 1.2rem;
    color: var(--text-muted);
    max-width: 600px;
    margin: 0 auto 2rem;
}

.hero-actions {
    display: flex;
    gap: 1rem;
    justify-content: center;
    flex-wrap: wrap;
}

.btn {
    padding: 0.75rem 2rem;
    border-radius: 8px;
    font-size: 1rem;
    font-weight: 600;
    text-decoration: none;
    transition: all 0.2s;
}

.btn-primary {
    background: var(--primary);
    color: #fff;
}

.btn-primary:hover {
    background: var(--primary-dark);
}

.btn-secondary {
    background: #fff;
    color: var(--text);
    border: 1px solid var(--border);
}

.btn-secondary:hover {
    border-color: var(--primary);
}

/* ── Features ───────────────────── */
.features {
    padding: 5rem 0;
    text-align: center;
}

.features h3 {
    font-size: 2rem;
    margin-bottom: 3rem;
}

.features-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
    gap: 2rem;
}

.feature-card {
    padding: 2rem;
    border-radius: 12px;
    background: #f8fafc;
    transition: transform 0.2s;
}

.feature-card:hover {
    transform: translateY(-4px);
}

.feature-icon {
    font-size: 2.5rem;
    margin-bottom: 1rem;
}

.feature-card h4 {
    font-size: 1.2rem;
    margin-bottom: 0.5rem;
}

.feature-card p {
    color: var(--text-muted);
}

/* ── CTA ────────────────────────── */
.cta {
    padding: 5rem 0;
    text-align: center;
    background: var(--primary);
    color: #fff;
}

.cta h3 {
    font-size: 2rem;
    margin-bottom: 0.5rem;
}

.cta p {
    font-size: 1.1rem;
    margin-bottom: 2rem;
    opacity: 0.9;
}

.cta .btn-primary {
    background: #fff;
    color: var(--primary);
}

.cta .btn-primary:hover {
    background: #f0f0f0;
}

/* ── Footer ─────────────────────── */
footer {
    padding: 2rem 0;
    text-align: center;
    color: var(--text-muted);
    font-size: 0.9rem;
}

/* ── Responsive ─────────────────── */
@media (max-width: 768px) {
    .hero h2 {
        font-size: 2rem;
    }

    .hero {
        padding: 4rem 0;
    }

    nav {
        gap: 1rem;
    }
}
"##
            .into(),
        },
        FileEntry {
            path: "script.js".into(),
            content: r##"document.addEventListener('DOMContentLoaded', () => {
    console.log('Landing page ready');
});

document.querySelectorAll('a[href^="#"]').forEach(anchor => {
    anchor.addEventListener('click', function (e) {
        e.preventDefault();
        const target = document.querySelector(this.getAttribute('href'));
        if (target) {
            target.scrollIntoView({ behavior: 'smooth' });
        }
    });
});
"##
            .into(),
        },
    ]
}

fn template_blog(name: &str) -> Vec<FileEntry> {
    let title = name.rsplit('/').next().unwrap_or(name);
    vec![
        FileEntry {
            path: "index.html".into(),
            content: format!(
                r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    <link rel="stylesheet" href="style.css">
</head>
<body>
    <header>
        <div class="container">
            <h1 class="logo">{title}</h1>
            <nav>
                <a href="index.html">Home</a>
                <a href="#">About</a>
                <a href="#">Tags</a>
            </nav>
        </div>
    </header>

    <main class="container">
        <section class="hero">
            <h2>Welcome to {title}</h2>
            <p>Thoughts on web development, design, and technology.</p>
        </section>

        <section class="posts">
            <article class="post-card">
                <span class="post-date">May 22, 2026</span>
                <h3><a href="#">Getting Started with HTML &amp; CSS</a></h3>
                <p>Learn the fundamentals of building web pages from scratch.</p>
                <span class="post-tag">html</span>
                <span class="post-tag">css</span>
            </article>

            <article class="post-card">
                <span class="post-date">May 20, 2026</span>
                <h3><a href="#">JavaScript Basics for Beginners</a></h3>
                <p>Understanding variables, functions, and the DOM.</p>
                <span class="post-tag">javascript</span>
            </article>

            <article class="post-card">
                <span class="post-date">May 18, 2026</span>
                <h3><a href="#">Why Semantic HTML Matters</a></h3>
                <p>Improve accessibility and SEO with proper HTML structure.</p>
                <span class="post-tag">html</span>
                <span class="post-tag">accessibility</span>
            </article>
        </section>
    </main>

    <footer>
        <div class="container">
            <p>&copy; 2026 {title}</p>
        </div>
    </footer>

    <script src="script.js"></script>
</body>
</html>"##,
                title = title
            ),
        },
        FileEntry {
            path: "style.css".into(),
            content: r##"* {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
}

body {
    font-family: Georgia, 'Times New Roman', serif;
    color: #2d3748;
    line-height: 1.8;
    background: #fefefe;
}

.container {
    max-width: 720px;
    margin: 0 auto;
    padding: 0 1.5rem;
}

/* ── Header ─────────────────────── */
header {
    padding: 2rem 0;
    border-bottom: 1px solid #e2e8f0;
    margin-bottom: 2rem;
}

header .container {
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-wrap: wrap;
    gap: 1rem;
}

.logo {
    font-size: 1.5rem;
    font-weight: 700;
}

nav {
    display: flex;
    gap: 1.5rem;
}

nav a {
    color: #4a5568;
    text-decoration: none;
    font-family: system-ui, sans-serif;
    font-size: 0.95rem;
    transition: color 0.2s;
}

nav a:hover {
    color: #1a202c;
}

/* ── Hero ───────────────────────── */
.hero {
    padding: 3rem 0 2rem;
    text-align: center;
    border-bottom: 1px solid #e2e8f0;
    margin-bottom: 2rem;
}

.hero h2 {
    font-size: 2.2rem;
    margin-bottom: 0.5rem;
}

.hero p {
    color: #718096;
    font-family: system-ui, sans-serif;
}

/* ── Posts ──────────────────────── */
.posts {
    display: flex;
    flex-direction: column;
    gap: 2rem;
    padding-bottom: 3rem;
}

.post-card {
    padding-bottom: 2rem;
    border-bottom: 1px solid #edf2f7;
}

.post-date {
    display: block;
    font-family: system-ui, sans-serif;
    color: #a0aec0;
    font-size: 0.85rem;
    margin-bottom: 0.25rem;
}

.post-card h3 {
    font-size: 1.4rem;
    margin-bottom: 0.5rem;
}

.post-card h3 a {
    color: #1a202c;
    text-decoration: none;
    transition: color 0.2s;
}

.post-card h3 a:hover {
    color: #6366f1;
}

.post-card p {
    color: #4a5568;
    font-family: system-ui, sans-serif;
    margin-bottom: 0.75rem;
}

.post-tag {
    display: inline-block;
    background: #edf2f7;
    color: #4a5568;
    font-family: system-ui, sans-serif;
    font-size: 0.8rem;
    padding: 0.15rem 0.6rem;
    border-radius: 4px;
    margin-right: 0.4rem;
}

/* ── Footer ─────────────────────── */
footer {
    padding: 2rem 0;
    text-align: center;
    color: #a0aec0;
    font-family: system-ui, sans-serif;
    font-size: 0.9rem;
}

@media (max-width: 600px) {
    .hero h2 {
        font-size: 1.5rem;
    }

    .post-card h3 {
        font-size: 1.2rem;
    }
}
"##
            .into(),
        },
        FileEntry {
            path: "script.js".into(),
            content: r##"document.addEventListener('DOMContentLoaded', () => {
    console.log('Blog ready');
});
"##
            .into(),
        },
    ]
}

// ── Utility functions ──────────────────────────────────────────────────────

fn is_command_blocked(cmd: &str) -> bool {
    let lower = cmd.to_lowercase();
    BLOCKED_COMMAND_PATTERNS.iter().any(|p| lower.contains(p))
}

fn looks_like_shell_command(line: &str) -> bool {
    let first = line.split_whitespace().next().unwrap_or_default();
    matches!(
        first,
        "ls" | "pwd"
            | "cd"
            | "mkdir"
            | "cp"
            | "mv"
            | "touch"
            | "cat"
            | "echo"
            | "rm"
            | "find"
            | "grep"
    )
}

fn sanitize_commands(cmds: &[String]) -> Vec<&str> {
    let mut seen = BTreeMap::<String, ()>::new();
    let mut out = Vec::new();
    for cmd in cmds {
        let key = cmd.trim().to_string();
        if key.is_empty() || seen.contains_key(&key) {
            continue;
        }
        seen.insert(key.clone(), ());
        out.push(cmd.trim());
    }
    out
}

// run_index delegates to the indexer module (see src/indexer.rs).
// The thin wrapper keeps the CLI printing / arg handling in main while the
// pure logic (chunking, writing, loading, retrieval) lives in its own module.

fn run_index(args: IndexArgs, cfg: &AppConfig) -> Result<()> {
    let dir = args.dir.clone().unwrap_or_else(|| {
        cfg.workspace_dir
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    });
    let dir = if dir.exists() {
        dir
    } else {
        PathBuf::from(".")
    };

    println!("{}", style!(C_CYAN, "│"));
    println!(
        "{} {} {}",
        style!(C_CYAN, "│"),
        style!(C_WHITE_B, "rem index"),
        style!(C_DIM, "— codebase retrieval index (pure Rust)")
    );
    println!(
        "{} target: {}",
        style!(C_CYAN, "│"),
        style!(C_DIM, "{}", dir.display())
    );

    let refreshing = load_codebase_index(&dir).is_some();
    let chunks = generate_codebase_index(&dir)?;
    if chunks.is_empty() {
        println!(
            "{} {} no indexable files found (after skips)",
            style!(C_CYAN, "│"),
            style!(C_YELLOW, "⚠")
        );
        return Ok(());
    }

    write_codebase_index(&dir, &chunks)?;

    let out_path = dir.join(".rem/codebase_index.json");
    let unique_files = chunks
        .iter()
        .map(|c| &c.path)
        .collect::<std::collections::HashSet<_>>()
        .len();
    let action = if refreshing { "refreshed" } else { "created" };
    println!(
        "{} {} {} {} chunks from {} files",
        style!(C_CYAN, "│"),
        style!(C_GREEN, "✓"),
        action,
        chunks.len(),
        unique_files
    );
    println!(
        "{} index: {}",
        style!(C_CYAN, "│"),
        style!(C_WHITE_B, "{}", out_path.display())
    );
    println!("{} `rem chat` / `rem ask` / `/goal` will now pull relevant chunks instead of full listings.", style!(C_CYAN, "│"));
    println!("{} (keyword retrieval; raise model_ctx in ~/.config/rem-cli/config.toml for large projects)", style!(C_CYAN, "│"));
    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_dangerous_commands() {
        assert!(is_command_blocked("rm -rf /tmp"));
        assert!(is_command_blocked("shutdown now"));
        assert!(!is_command_blocked("ls -la"));
    }

    #[test]
    fn truncates_string() {
        let out = truncate_bytes("abcdef", 3);
        assert!(out.starts_with("abc"));
    }

    #[test]
    fn command_sanitization_dedups() {
        let input = vec![" ls ".to_string(), "ls".to_string(), "".to_string()];
        let out = sanitize_commands(&input);
        assert_eq!(out, vec!["ls"]);
    }

    #[test]
    fn fallback_extracts_commands() {
        let out = ModelReply::fallback("Use:\nmkdir project\ncd project");
        assert!(out.commands.iter().any(|c| c == "mkdir project"));
    }

    #[test]
    fn fallback_extracts_code_block() {
        let out = ModelReply::fallback("Here:\n```html\n<div>hi</div>\n```\ndone");
        assert_eq!(out.code, "<div>hi</div>");
    }

    #[test]
    fn api_url_plain_base() {
        assert_eq!(
            api_url("http://localhost:11434", "generate"),
            "http://localhost:11434/api/generate"
        );
    }

    #[test]
    fn api_url_with_api() {
        assert_eq!(
            api_url("http://localhost:11434/api", "generate"),
            "http://localhost:11434/api/generate"
        );
    }

    #[test]
    fn detects_creation_intent() {
        assert!(intent::has_creation_intent("create a basic html page"));
        assert!(intent::has_creation_intent("build me a navbar"));
        assert!(intent::has_creation_intent("generate a contact form"));
        assert!(!intent::has_creation_intent("explain ls command"));
    }

    #[test]
    fn detects_file_path() {
        assert!(intent::has_file_path("create page at index.html"));
        assert!(intent::has_file_path("write to ./src/style.css"));
        assert!(intent::has_file_path("add this to file app.js"));
        assert!(!intent::has_file_path("create a basic html page"));
    }

    #[test]
    fn extracts_code_block_from_text() {
        let text = "Sure:\n```css\n.card { padding: 12px; }\n```\nThat's it.";
        assert_eq!(extract_code_block(text), ".card { padding: 12px; }");
    }

    #[test]
    fn extracts_code_no_fence() {
        let text = "Here is some code: .card {}";
        assert_eq!(extract_code_block(text), "");
    }

    #[test]
    fn extracts_multi_file_blocks() {
        let text = r##"Here's your site:

### index.html
```html
<!DOCTYPE html>
<html></html>
```

### style.css
```css
body { margin: 0; }
```"##;
        let files = extract_code_blocks_with_names(text);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "index.html");
        assert_eq!(files[1].path, "style.css");
        assert!(files[0].content.contains("<!DOCTYPE"));
    }

    #[test]
    fn extracts_file_from_bold_header() {
        let text = r##"**style.css**
```css
h1 { color: red; }
```"##;
        let files = extract_code_blocks_with_names(text);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "style.css");
        assert!(files[0].content.contains("h1"));
    }

    #[test]
    fn guesses_html_filename() {
        let lines: Vec<&str> = vec!["<!DOCTYPE html>", "<html>", "<head>"];
        assert_eq!(guess_filename(&lines), "index.html");
    }

    #[test]
    fn guesses_css_filename() {
        let lines: Vec<&str> = vec![".card {", "  margin: 0;", "}"];
        assert_eq!(guess_filename(&lines), "style.css");
    }

    #[test]
    fn guesses_js_filename() {
        let lines: Vec<&str> = vec!["const app = () => {", "  console.log('hi')", "}"];
        assert_eq!(guess_filename(&lines), "script.js");
    }

    #[test]
    fn human_size_formats() {
        assert_eq!(human_size(500), "500");
        assert!(human_size(2048).contains("2"));
        assert!(human_size(2048).contains("K"));
    }

    #[test]
    fn greeting_is_fast_answer() {
        assert_eq!(intent::classify_intent("hii"), TaskIntent::FastAnswer);
        assert_eq!(intent::classify_intent("hello"), TaskIntent::FastAnswer);
        assert_eq!(
            intent::classify_intent("heyy there"),
            TaskIntent::FastAnswer
        );
        assert_eq!(intent::classify_intent("thanks!"), TaskIntent::FastAnswer);
    }

    #[test]
    fn question_about_creation_is_fast_answer() {
        assert_eq!(
            intent::classify_intent("explain how to make a file"),
            TaskIntent::FastAnswer
        );
        assert_eq!(
            intent::classify_intent("how to create a React component properly"),
            TaskIntent::FastAnswer
        );
        assert_eq!(
            intent::classify_intent("what is the best way to scaffold a project"),
            TaskIntent::Planning
        );
        assert_eq!(
            intent::classify_intent("how do I build a website from scratch"),
            TaskIntent::FastAnswer
        );
    }

    #[test]
    fn clear_creation_still_code_action() {
        assert_eq!(
            intent::classify_intent("create a React component called Button"),
            TaskIntent::CodeAction
        );
        assert_eq!(
            intent::classify_intent("make a navbar component"),
            TaskIntent::CodeAction
        );
        assert_eq!(
            intent::classify_intent("write a function to sort arrays"),
            TaskIntent::CodeAction
        );
    }

    #[test]
    fn fix_is_still_code_action() {
        assert_eq!(
            intent::classify_intent("fix the bug in auth middleware"),
            TaskIntent::CodeAction
        );
        assert_eq!(
            intent::classify_intent("refactor the user service"),
            TaskIntent::CodeAction
        );
    }

    #[test]
    fn has_creation_intent_regression() {
        assert!(intent::has_creation_intent(
            "create a file called index.html"
        ));
        assert!(intent::has_creation_intent("build me a website"));
        assert!(intent::has_creation_intent("make a component"));
        assert!(!intent::has_creation_intent("explain how to create a file"));
        assert!(!intent::has_creation_intent("how do i make a test"));
        assert!(!intent::has_creation_intent("hii"));
        assert!(!intent::has_creation_intent(
            "what is the best way to build a project"
        ));
    }

    #[test]
    fn is_question_about_works() {
        assert!(intent::is_question_about(
            "how to create a file",
            "create a file"
        ));
        assert!(intent::is_question_about(
            "explain how to make a component",
            "make a component"
        ));
        assert!(!intent::is_question_about(
            "create a file now",
            "create a file"
        ));
        assert!(!intent::is_question_about("how should i", "create a file"));
    }

    #[test]
    fn strip_code_blocks_works() {
        let text = "Here is some text.\n```html\n<div>code</div>\n```\nMore text.";
        let result = strip_code_blocks(text);
        assert!(result.contains("Here is some text"));
        assert!(result.contains("More text"));
        assert!(!result.contains("<div>code</div>"));
        assert!(!result.contains("```"));
    }

    #[test]
    fn validate_chat_response_strips_code() {
        let response = "Here is your site:\n\n### index.html\n```html\n<div>hi</div>\n```";
        let (was_validated, text) =
            validate_chat_response(response, &TaskIntent::FastAnswer, &RunMode::Chat);
        assert!(was_validated);
        assert!(text.contains("Here is your site"));
        assert!(!text.contains("<div>hi</div>"));
    }

    #[test]
    fn validate_chat_response_passes_valid() {
        let response = "Hi there! How can I help you today?";
        let (was_validated, _) =
            validate_chat_response(response, &TaskIntent::FastAnswer, &RunMode::Chat);
        assert!(!was_validated);
    }

    #[test]
    fn validate_chat_allows_code_action() {
        let response = "### app.js\n```js\nconst x = 1;\n```";
        let (was_validated, _) =
            validate_chat_response(response, &TaskIntent::CodeAction, &RunMode::Code);
        assert!(!was_validated);
    }

    #[test]
    fn resolve_safe_path_allows_workspace_relative_path() {
        let base = std::env::temp_dir().join(format!(
            "rem-cli-safe-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).expect("temp base should be created");

        let result = resolve_safe_path(&base, "main.rs");
        assert!(result.is_some());
        let resolved = result.expect("path should resolve");
        assert!(resolved.starts_with(&base));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resolve_safe_path_blocks_parent_traversal() {
        let base = std::env::temp_dir().join(format!(
            "rem-cli-traversal-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).expect("temp base should be created");

        let result = resolve_safe_path(&base, "../escape.txt");
        assert!(result.is_none());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn retrieval_keyword_finds_relevant_chunk() {
        let fake = vec![
            indexer::IndexChunk {
                path: "src/auth.rs".into(),
                name: "login".into(),
                chunk_type: "function".into(),
                content: "pub fn login(user: &str, pass: &str) -> Result<Token> { ... }".into(),
                start_line: 10,
                end_line: 20,
                embedding: None,
            },
            indexer::IndexChunk {
                path: "src/utils.rs".into(),
                name: "hash".into(),
                chunk_type: "function".into(),
                content: "fn hash_password(pw: &str) -> String { ... }".into(),
                start_line: 5,
                end_line: 8,
                embedding: None,
            },
        ];
        let hits =
            retrieve_relevant_chunks(&fake, "implement login with password hashing", 3, 2000);
        assert!(!hits.is_empty());
        // Should prefer the auth/login chunk due to word matches in content + name
        let top = hits[0];
        assert!(top.path.contains("auth") || top.content.to_lowercase().contains("login"));
    }

    #[test]
    fn load_index_returns_none_for_missing() {
        let tmp = std::env::temp_dir().join("rem-no-index-xyz");
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(load_codebase_index(&tmp).is_none());
    }

    #[test]
    fn relevant_context_uses_real_index_when_present() {
        // The pure-Rust indexer (generate_codebase_index + write) or a pre-existing index can be used.
        // This test gracefully handles missing index (falls back) and exercises the path when present.
        let fixture = std::env::temp_dir().join("rem-fixture");
        if let Some(_idx) = load_codebase_index(&fixture) {
            // Session with project dir pointing at indexed tree
            let session = ChatSession::new("test-model", Some(fixture.clone())).expect("session");
            let ctx = session.build_relevant_project_context("hello function");
            // When index present and keyword matches, we should see the content or header
            if !ctx.is_empty() {
                assert!(
                    ctx.contains("hello")
                        || ctx.contains("app.py")
                        || ctx.contains("Relevant code")
                );
            }
        } else {
            // No index in this env run — still validate the fallback path doesn't blow up
            let session = ChatSession::new("test-model", Some(fixture.clone())).expect("session");
            let _ = session.build_relevant_project_context("anything");
        }
    }

    #[test]
    fn pure_rust_indexer_roundtrips_and_retrieves() {
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp = std::env::temp_dir().join(format!("rem-index-test-{}", stamp));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("src")).expect("temp tree");

        // Small file (whole chunk)
        fs::write(
            tmp.join("README.md"),
            "# Demo\n\nThis is a test project for indexing.\nUse it to verify retrieval.\n",
        )
        .unwrap();

        // Larger-ish file that should split
        let big = (0..60)
            .map(|i| format!("fn example_{}() {{ /* chunk candidate {} */ }}\n\n", i, i))
            .collect::<String>();
        fs::write(
            tmp.join("src/lib.rs"),
            format!("//! Example lib\n\n{}", big),
        )
        .unwrap();

        // Run the generator + writer (the real thing `rem index` calls)
        let chunks = generate_codebase_index(&tmp).expect("generate should succeed");
        assert!(
            !chunks.is_empty(),
            "should have produced at least one chunk"
        );

        write_codebase_index(&tmp, &chunks).expect("write should succeed");

        // Round-trip via the normal loader
        let loaded = load_codebase_index(&tmp).expect("load after write should succeed");
        assert!(!loaded.is_empty());

        // Retrieval should find the lib.rs content for a query mentioning "example"
        let hits = retrieve_relevant_chunks(&loaded, "example function in lib", 5, 8000);
        assert!(
            !hits.is_empty(),
            "keyword retrieval should surface relevant chunks"
        );
        let hit_paths: Vec<_> = hits.iter().map(|h| h.path.as_str()).collect();
        assert!(
            hit_paths
                .iter()
                .any(|p| p.contains("lib.rs") || p.contains("README")),
            "expected project files in hits"
        );

        // Lines should be populated for at least some chunks (we use them in build_retrieved_context now)
        assert!(loaded
            .iter()
            .any(|c| c.start_line > 0 && c.end_line >= c.start_line));

        // Cleanup
        let _ = fs::remove_dir_all(&tmp);
    }
}
