//! CLI argument parsing and configuration types.
//! Uses clap to define the command-line interface, subcommands,
//! and serializable config structs ([`AppConfig`], [`PartialConfig`]).

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};

/// Top-level CLI argument parser.
#[derive(Parser, Debug)]
#[command(
    name = "rem",
    version,
    about = "REM — Coding assistant CLI. Run `rem` to start interactive chat. Pipe input for non-interactive mode.",
    long_about = None,
)]
pub struct Cli {
    #[arg(long, global = true, help = "Ollama model name")]
    pub model: Option<String>,
    #[arg(long, global = true, help = "Ollama API URL")]
    pub ollama_url: Option<String>,
    #[arg(
        long,
        global = true,
        help = "Provider: ollama (default), openai, anthropic, gemini, azure, bedrock, openrouter, deepseek, github, xai"
    )]
    pub provider: Option<String>,
    #[arg(long, global = true, help = "API key for OpenAI-compatible providers")]
    pub api_key: Option<String>,
    #[arg(long, short = 'v', global = true, help = "Verbose output (show raw model responses)")]
    pub verbose: bool,
    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// REM subcommands.
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
    #[command(about = "Pull a model via Ollama (e.g. `rem pull qwen2.5-coder:1.5b`)")]
    Pull(PullArgs),
    #[command(about = "List or switch color themes")]
    Theme(ThemeArgs),
    #[command(about = "Generate shell completion scripts")]
    Completions(CompletionsArgs),
    #[command(
        about = "Query SigNoz observability (MCP) and answer with the active LLM — e.g. `rem observe \"which tasks used fireworks\"`"
    )]
    Observe(ObserveArgs),
}

/// Arguments for `rem observe`.
#[derive(Args, Debug)]
pub struct ObserveArgs {
    #[arg(help = "Natural-language observability question about router-agent / SigNoz traces")]
    pub query: String,
}

/// Arguments for `rem ask`.
#[derive(Args, Debug)]
pub struct AskArgs {
    #[arg(help = "Your coding question")]
    pub prompt: String,
    #[arg(long, help = "Optional file for context")]
    pub file: Option<PathBuf>,
    #[arg(long, default_value = "text", help = "Output format: text, json, json-pretty")]
    pub format: String,
}

/// Arguments for `rem explain`.
#[derive(Args, Debug)]
pub struct ExplainArgs {
    #[arg(help = "Terminal command to explain")]
    pub command: String,
}

/// Arguments for `rem patch`.
#[derive(Args, Debug)]
pub struct PatchArgs {
    #[arg(long, help = "Target file to patch")]
    pub file: PathBuf,
    #[arg(long, help = "Description of changes needed")]
    pub task: String,
}

/// Arguments for `rem new`.
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

/// Arguments for `rem index`.
#[derive(Args, Debug)]
pub struct IndexArgs {
    #[arg(help = "Project directory to index (defaults to current workspace or .)")]
    pub dir: Option<PathBuf>,
    #[arg(long, help = "Preview what would be indexed without writing any files")]
    pub dry_run: bool,
}

/// Arguments for `rem theme`.
#[derive(Args, Debug)]
pub struct ThemeArgs {
    #[arg(help = "Theme name to switch to (e.g. GHOST, PAPER, SAKURA). Omit to list.")]
    pub name: Option<String>,
}

/// Arguments for `rem pull`.
#[derive(Args, Debug)]
pub struct PullArgs {
    #[arg(help = "Model name to pull (e.g. qwen2.5-coder:1.5b)")]
    pub model: Option<String>,
}

/// Arguments for `rem completions`.
#[derive(Args, Debug)]
pub struct CompletionsArgs {
    #[arg(help = "Shell type: bash, zsh, fish, powershell, elvish")]
    pub shell: String,
}

/// Per-provider configuration overrides.
/// Values here override the top-level `AppConfig` fields when the matching provider is active.
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProviderSettings {
    pub model: Option<String>,
    pub api_url: Option<String>,
    pub api_key: Option<String>,
    pub timeout_s: Option<u64>,
    pub model_ctx: Option<usize>,
    pub reasoning_model: Option<bool>,
}

impl fmt::Debug for ProviderSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderSettings")
            .field("model", &self.model)
            .field("api_url", &self.api_url)
            .field("api_key", &self.api_key.as_deref().map(|_| "***"))
            .field("timeout_s", &self.timeout_s)
            .field("model_ctx", &self.model_ctx)
            .field("reasoning_model", &self.reasoning_model)
            .finish()
    }
}

/// Global and local merged configuration.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    pub providers: HashMap<String, ProviderSettings>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub thinking_budget: Option<u32>,
    #[serde(default = "default_search_provider")]
    pub search_provider: String,
    #[serde(default)]
    pub search_api_key: Option<String>,
    #[serde(default)]
    pub search_cse_id: Option<String>,
    #[serde(default = "default_page_threshold")]
    pub page_threshold: usize,
    #[serde(default = "default_auto_resume")]
    pub auto_resume: bool,
    #[serde(default = "default_true")]
    pub show_perf: bool,
    /// SigNoz MCP HTTP endpoint (self-host default: http://localhost:8000/mcp).
    #[serde(default = "default_signoz_mcp_url")]
    pub signoz_mcp_url: String,
    /// Optional SigNoz service-account API key for MCP auth.
    #[serde(default)]
    pub signoz_api_key: Option<String>,
    /// Optional SigNoz UI/API base URL (header X-SigNoz-URL).
    #[serde(default)]
    pub signoz_url: Option<String>,
    /// service.name filter for observe queries (default: router-agent).
    #[serde(default = "default_signoz_service")]
    pub signoz_service: String,
}

fn default_signoz_mcp_url() -> String {
    "http://localhost:8000/mcp".to_string()
}

fn default_signoz_service() -> String {
    "router-agent".to_string()
}

fn default_true() -> bool {
    true
}

impl fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppConfig")
            .field("model", &self.model)
            .field("ollama_url", &self.ollama_url)
            .field("timeout_s", &self.timeout_s)
            .field("max_context_bytes", &self.max_context_bytes)
            .field("model_ctx", &self.model_ctx)
            .field("prompts_dir", &self.prompts_dir)
            .field("workspace_dir", &self.workspace_dir)
            .field("provider", &self.provider)
            .field("api_url", &self.api_url)
            .field("api_key", &self.api_key.as_deref().map(|_| "***"))
            .field("theme", &self.theme)
            .field("mode", &self.mode)
            .field("providers", &self.providers)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("thinking_budget", &self.thinking_budget)
            .field("search_provider", &self.search_provider)
            .field("search_api_key", &self.search_api_key.as_deref().map(|_| "***"))
            .field("search_cse_id", &self.search_cse_id)
            .field("page_threshold", &self.page_threshold)
            .field("auto_resume", &self.auto_resume)
            .field("show_perf", &self.show_perf)
            .field("signoz_mcp_url", &self.signoz_mcp_url)
            .field("signoz_api_key", &self.signoz_api_key.as_deref().map(|_| "***"))
            .field("signoz_url", &self.signoz_url)
            .field("signoz_service", &self.signoz_service)
            .finish()
    }
}

fn default_page_threshold() -> usize {
    50
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
fn default_search_provider() -> String {
    "ddg".to_string()
}
fn default_auto_resume() -> bool {
    true
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
            providers: HashMap::new(),
            reasoning_effort: None,
            thinking_budget: None,
            search_provider: "ddg".to_string(),
            search_api_key: None,
            search_cse_id: None,
            page_threshold: default_page_threshold(),
            auto_resume: default_auto_resume(),
            show_perf: default_true(),
            signoz_mcp_url: default_signoz_mcp_url(),
            signoz_api_key: None,
            signoz_url: None,
            signoz_service: default_signoz_service(),
        }
    }
}

/// Partial config for incremental merging (from TOML files).
/// Unknown fields are rejected to catch typos early.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    pub providers: Option<HashMap<String, ProviderSettings>>,
    pub reasoning_effort: Option<String>,
    pub thinking_budget: Option<u32>,
    pub search_provider: Option<String>,
    pub search_api_key: Option<String>,
    pub search_cse_id: Option<String>,
    pub page_threshold: Option<usize>,
    pub auto_resume: Option<bool>,
    pub show_perf: Option<bool>,
    pub signoz_mcp_url: Option<String>,
    pub signoz_api_key: Option<String>,
    pub signoz_url: Option<String>,
    pub signoz_service: Option<String>,
}

impl fmt::Debug for PartialConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PartialConfig")
            .field("model", &self.model)
            .field("ollama_url", &self.ollama_url)
            .field("timeout_s", &self.timeout_s)
            .field("max_context_bytes", &self.max_context_bytes)
            .field("model_ctx", &self.model_ctx)
            .field("prompts_dir", &self.prompts_dir)
            .field("workspace_dir", &self.workspace_dir)
            .field("provider", &self.provider)
            .field("api_url", &self.api_url)
            .field("api_key", &self.api_key.as_deref().map(|_| "***"))
            .field("theme", &self.theme)
            .field("mode", &self.mode)
            .field("providers", &self.providers)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("thinking_budget", &self.thinking_budget)
            .field("search_provider", &self.search_provider)
            .field("search_api_key", &self.search_api_key.as_deref().map(|_| "***"))
            .field("search_cse_id", &self.search_cse_id)
            .field("page_threshold", &self.page_threshold)
            .field("auto_resume", &self.auto_resume)
            .field("show_perf", &self.show_perf)
            .field("signoz_mcp_url", &self.signoz_mcp_url)
            .field("signoz_api_key", &self.signoz_api_key.as_deref().map(|_| "***"))
            .field("signoz_url", &self.signoz_url)
            .field("signoz_service", &self.signoz_service)
            .finish()
    }
}

impl AppConfig {
    /// Merges [`PartialConfig`] values into this config (non-`None` fields win).
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
        if let Some(v) = part.providers {
            self.providers.extend(v);
        }
        if let Some(v) = part.reasoning_effort {
            self.reasoning_effort = Some(v);
        }
        if let Some(v) = part.thinking_budget {
            self.thinking_budget = Some(v);
        }
        if let Some(v) = part.search_provider {
            self.search_provider = v;
        }
        if let Some(v) = part.search_api_key {
            self.search_api_key = Some(v);
        }
        if let Some(v) = part.search_cse_id {
            self.search_cse_id = Some(v);
        }
        if let Some(v) = part.page_threshold {
            self.page_threshold = v;
        }
        if let Some(v) = part.auto_resume {
            self.auto_resume = v;
        }
        if let Some(v) = part.show_perf {
            self.show_perf = v;
        }
        if let Some(v) = part.signoz_mcp_url {
            self.signoz_mcp_url = v;
        }
        if let Some(v) = part.signoz_api_key {
            self.signoz_api_key = Some(v);
        }
        if let Some(v) = part.signoz_url {
            self.signoz_url = Some(v);
        }
        if let Some(v) = part.signoz_service {
            self.signoz_service = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_config_default() {
        let config = AppConfig::default();
        assert_eq!(config.model, "rem-coder:latest");
        assert_eq!(config.ollama_url, "http://localhost:11434");
        assert_eq!(config.timeout_s, 120);
        assert_eq!(config.max_context_bytes, 16_000);
        assert_eq!(config.model_ctx, 4096);
        assert_eq!(config.provider, "ollama");
        assert_eq!(config.theme, "GHOST");
        assert_eq!(config.mode, "CHAT");
        assert_eq!(config.search_provider, "ddg");
    }

    #[test]
    fn test_apply_partial_overrides_model() {
        let mut config = AppConfig::default();
        let partial = PartialConfig {
            model: Some("test-model".to_string()),
            ..Default::default()
        };
        config.apply_partial(partial);
        assert_eq!(config.model, "test-model");
        // Ensure other fields remain default
        assert_eq!(config.ollama_url, "http://localhost:11434");
    }

    #[test]
    fn test_apply_partial_overrides_all() {
        let mut config = AppConfig::default();
        let partial = PartialConfig {
            model: Some("m1".to_string()),
            ollama_url: Some("u1".to_string()),
            timeout_s: Some(1),
            max_context_bytes: Some(2),
            model_ctx: Some(3),
            prompts_dir: Some("pd".to_string()),
            workspace_dir: Some("wd".to_string()),
            provider: Some("p1".to_string()),
            api_url: Some("au".to_string()),
            api_key: Some("ak".to_string()),
            theme: Some("t1".to_string()),
            mode: Some("m1".to_string()),
            providers: Some(HashMap::new()),
            reasoning_effort: Some("re".to_string()),
            thinking_budget: Some(42),
            search_provider: Some("sp".to_string()),
            search_api_key: Some("sak".to_string()),
            search_cse_id: Some("sci".to_string()),
            page_threshold: Some(75),
            auto_resume: Some(true),
            show_perf: Some(true),
            signoz_mcp_url: Some("http://mcp.example/mcp".to_string()),
            signoz_api_key: Some("sk".to_string()),
            signoz_url: Some("http://signoz.example".to_string()),
            signoz_service: Some("svc".to_string()),
        };
        config.apply_partial(partial);
        assert_eq!(config.model, "m1");
        assert_eq!(config.ollama_url, "u1");
        assert_eq!(config.timeout_s, 1);
        assert_eq!(config.max_context_bytes, 2);
        assert_eq!(config.model_ctx, 3);
        assert_eq!(config.prompts_dir, Some("pd".to_string()));
        assert_eq!(config.workspace_dir, Some("wd".to_string()));
        assert_eq!(config.provider, "p1");
        assert_eq!(config.api_url, Some("au".to_string()));
        assert_eq!(config.api_key, Some("ak".to_string()));
        assert_eq!(config.theme, "t1");
        assert_eq!(config.mode, "m1");
        assert_eq!(config.signoz_mcp_url, "http://mcp.example/mcp");
        assert_eq!(config.signoz_api_key, Some("sk".to_string()));
        assert_eq!(config.signoz_url, Some("http://signoz.example".to_string()));
        assert_eq!(config.signoz_service, "svc");
        assert!(config.providers.is_empty());
        assert_eq!(config.reasoning_effort, Some("re".to_string()));
        assert_eq!(config.thinking_budget, Some(42));
        assert_eq!(config.search_provider, "sp");
        assert_eq!(config.search_api_key, Some("sak".to_string()));
        assert_eq!(config.search_cse_id, Some("sci".to_string()));
        assert_eq!(config.page_threshold, 75);
        assert!(config.auto_resume);
    }

    #[test]
    fn test_apply_partial_none_overrides_nothing() {
        let mut config = AppConfig::default();
        let original = config.clone();
        let partial = PartialConfig::default();
        config.apply_partial(partial);
        assert_eq!(config.model, original.model);
        assert_eq!(config.ollama_url, original.ollama_url);
        assert_eq!(config.timeout_s, original.timeout_s);
        assert_eq!(config.max_context_bytes, original.max_context_bytes);
        assert_eq!(config.model_ctx, original.model_ctx);
        assert_eq!(config.prompts_dir, original.prompts_dir);
        assert_eq!(config.workspace_dir, original.workspace_dir);
        assert_eq!(config.provider, original.provider);
        assert_eq!(config.api_url, original.api_url);
        assert_eq!(config.api_key, original.api_key);
        assert_eq!(config.theme, original.theme);
        assert_eq!(config.mode, original.mode);
        assert_eq!(config.reasoning_effort, original.reasoning_effort);
        assert_eq!(config.thinking_budget, original.thinking_budget);
        assert_eq!(config.search_provider, original.search_provider);
        assert_eq!(config.search_api_key, original.search_api_key);
        assert_eq!(config.search_cse_id, original.search_cse_id);
    }

    #[test]
    fn test_apply_partial_partial_overrides() {
        let mut config = AppConfig::default();
        let partial = PartialConfig {
            model: Some("partial-model".to_string()),
            timeout_s: Some(99),
            ..Default::default()
        };
        config.apply_partial(partial);
        assert_eq!(config.model, "partial-model");
        assert_eq!(config.timeout_s, 99);
        // All other fields remain at default
        assert_eq!(config.ollama_url, "http://localhost:11434");
        assert_eq!(config.max_context_bytes, 16_000);
        assert_eq!(config.model_ctx, 4096);
        assert_eq!(config.provider, "ollama");
        assert_eq!(config.theme, "GHOST");
        assert_eq!(config.mode, "CHAT");
        assert_eq!(config.search_provider, "ddg");
    }

    #[test]
    fn test_apply_partial_clears_optionals() {
        let mut config = AppConfig::default();
        assert_eq!(config.api_url, None);
        let partial = PartialConfig {
            api_url: Some("http://override.com".to_string()),
            ..Default::default()
        };
        config.apply_partial(partial);
        assert_eq!(config.api_url, Some("http://override.com".to_string()));
    }

    #[test]
    fn test_default_theme_name() {
        assert_eq!(default_theme_name(), "GHOST");
    }

    #[test]
    fn test_default_mode_name() {
        assert_eq!(default_mode_name(), "CHAT");
    }

    #[test]
    fn test_default_provider() {
        assert_eq!(default_provider(), "ollama");
    }

    #[test]
    fn test_default_search_provider() {
        assert_eq!(default_search_provider(), "ddg");
    }

    #[test]
    fn test_show_perf_defaults_to_true() {
        let config = AppConfig::default();
        assert!(config.show_perf);
    }
}
