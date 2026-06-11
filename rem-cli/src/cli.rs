use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};

#[derive(Parser, Debug)]
#[command(
    name = "rem",
    version,
    about = "REM — Coding assistant CLI. Run `rem` to start interactive chat. Type /mode to toggle CHAT ↔ CODE ↔ PLAN.",
    long_about = None,
)]
pub struct Cli {
    #[arg(long, global = true, help = "Ollama model name")]
    pub model: Option<String>,
    #[arg(long, global = true, help = "Ollama API URL")]
    pub ollama_url: Option<String>,
    #[arg(long, global = true, help = "Provider: ollama (default), openai, vllm")]
    pub provider: Option<String>,
    #[arg(long, global = true, help = "API key for OpenAI-compatible providers")]
    pub api_key: Option<String>,
    #[arg(
        long,
        short = 'v',
        global = true,
        help = "Verbose output (show raw model responses)"
    )]
    pub verbose: bool,
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
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
pub struct AskArgs {
    #[arg(help = "Your coding question")]
    pub prompt: String,
    #[arg(long, help = "Optional file for context")]
    pub file: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct ExplainArgs {
    #[arg(help = "Terminal command to explain")]
    pub command: String,
}

#[derive(Args, Debug)]
pub struct PatchArgs {
    #[arg(long, help = "Target file to patch")]
    pub file: PathBuf,
    #[arg(long, help = "Description of changes needed")]
    pub task: String,
}

#[derive(Args, Debug)]
pub struct NewArgs {
    #[arg(help = "Project name / directory path")]
    pub name: String,
    #[arg(
        long,
        default_value = "bare",
        help = "Project type: bare, portfolio, landing, blog, rust, python, go, javascript"
    )]
    pub project_type: String,
}

#[derive(Args, Debug)]
pub struct IndexArgs {
    #[arg(help = "Project directory to index (defaults to current workspace or .)")]
    pub dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub model: String,
    pub ollama_url: String,
    pub timeout_s: u64,
    pub max_context_bytes: usize,
    pub model_ctx: usize,
    pub prompts_dir: Option<String>,
    #[serde(default)]
    pub workspace_dir: Option<String>,
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default)]
    pub api_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_theme_name")]
    pub theme: String,
    #[serde(default = "default_mode_name")]
    pub mode: String,
}

fn default_theme_name() -> String {
    "GHOST".to_string()
}
fn default_mode_name() -> String {
    "CHAT".to_string()
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
            model_ctx: 4096,
            prompts_dir: None,
            workspace_dir: None,
            provider: "ollama".to_string(),
            api_url: None,
            api_key: None,
            theme: default_theme_name(),
            mode: default_mode_name(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct PartialConfig {
    pub model: Option<String>,
    pub ollama_url: Option<String>,
    pub timeout_s: Option<u64>,
    pub max_context_bytes: Option<usize>,
    pub model_ctx: Option<usize>,
    pub prompts_dir: Option<String>,
    pub workspace_dir: Option<String>,
    pub provider: Option<String>,
    pub api_url: Option<String>,
    pub api_key: Option<String>,
    pub theme: Option<String>,
    pub mode: Option<String>,
}

impl AppConfig {
    pub fn apply_partial(&mut self, part: PartialConfig) {
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
        if let Some(v) = part.theme {
            self.theme = v;
        }
        if let Some(v) = part.mode {
            self.mode = v;
        }
    }
}
