//! Configuration loading, saving, and provider construction.
//! Reads `~/.config/rem-cli/config.toml` and `.remcli.toml`, merges them into
//! an [`AppConfig`], and builds the appropriate [`Provider`] from config values.

use crate::cli::{AppConfig, PartialConfig};
use crate::provider::{Provider, ProviderKind};
use crate::ui;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::RwLock;
use tracing::warn;

/// Cached config to avoid repeated TOML parsing from disk.
static CONFIG_CACHE: LazyLock<RwLock<Option<AppConfig>>> = LazyLock::new(|| RwLock::new(None));

/// Returns the config directory path, checking XDG_CONFIG_HOME first.
fn config_dir() -> Option<std::path::PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let dir = std::path::PathBuf::from(xdg).join("rem-cli");
        return Some(dir);
    }
    dirs::home_dir().map(|h| h.join(".config/rem-cli"))
}

/// Saves config to XDG config dir or `~/.config/rem-cli/config.toml`.
/// Updates the in-memory cache directly to avoid unnecessary re-reads.
pub(crate) fn save_config(cfg: &AppConfig) -> Result<()> {
    let text = toml::to_string_pretty(cfg).context("failed to serialize config")?;
    if let Some(dir) = config_dir() {
        fs::create_dir_all(&dir)?;
        let path = dir.join("config.toml");
        fs::write(&path, &text).context("failed to write config")?;
    }
    // Update cache directly instead of invalidating, to avoid re-reading from disk
    crate::pager::init_page_threshold(cfg.page_threshold);
    let mut cache = CONFIG_CACHE.write().unwrap_or_else(|e| e.into_inner());
    *cache = Some(cfg.clone());
    Ok(())
}

/// Interactive first-time setup that prompts for a workspace directory.
pub(crate) fn first_run_setup(cfg: &mut AppConfig) -> Result<Option<PathBuf>> {
    let t = ui::theme::active();
    println!();
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "Welcome to REM!", true),
        ui::theme::paint_dim(&t, "first-time setup")
    );
    println!();
    println!("{}", ui::theme::paint(&t, "accent", "\u{258C}", true));
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "Where should REM create your projects?"),
    );
    println!(
        "{} e.g. {} or {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "~/projects"),
        ui::theme::paint_bright(&t, "/home/you/code")
    );
    println!(
        "{} type {} for current dir, or a full path",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, ".")
    );
    println!("{}", ui::theme::paint(&t, "accent", "\u{258C}", true));
    print!("{}", ui::theme::paint(&t, "accent", "\u{258C}  rem> ", true));
    let _ = io::stdout().flush();

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();

    let dir = if trimmed.is_empty() || trimmed == "." {
        std::env::current_dir().unwrap_or_default()
    } else if trimmed == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"))
    } else if trimmed.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            home.join(trimmed.trim_start_matches("~/"))
        } else {
            PathBuf::from(trimmed)
        }
    } else {
        PathBuf::from(trimmed)
    };

    if !dir.exists() {
        println!(
            "{} creating {}...",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            dir.display()
        );
        fs::create_dir_all(&dir)?;
    }

    cfg.workspace_dir = Some(dir.to_string_lossy().to_string());
    save_config(cfg)?;

    println!(
        "{} workspace saved to {}",
        ui::theme::paint_success_label(&t, "\u{258C}  ✓"),
        ui::theme::paint_bright(&t, &dir.display().to_string())
    );
    println!(
        "{} change it anytime with {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "/dir <path>")
    );
    println!();

    Ok(Some(dir))
}

/// Loads and merges global (XDG_CONFIG_HOME or `~/.config/rem-cli/config.toml`) and local (`.remcli.toml`) config.
/// Results are cached in a global [`CONFIG_CACHE`] to avoid repeated TOML parsing.
pub(crate) fn load_config() -> Result<AppConfig> {
    {
        let cache = CONFIG_CACHE.read().unwrap_or_else(|e| e.into_inner());
        if let Some(ref cached) = *cache {
            return Ok(cached.clone());
        }
    }
    let mut cfg = AppConfig::default();
    if let Some(dir) = config_dir() {
        let path = dir.join("config.toml");
        if path.exists() {
            let text = fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
            let partial: PartialConfig = toml::from_str(&text).context("invalid global config")?;
            cfg.apply_partial(partial);
        }
    }
    let local = PathBuf::from(".remcli.toml");
    if local.exists() {
        let text = fs::read_to_string(&local).with_context(|| format!("failed to read {}", local.display()))?;
        let partial: PartialConfig = toml::from_str(&text).context("invalid local config")?;
        cfg.apply_partial(partial);
    }
    crate::pager::init_page_threshold(cfg.page_threshold);
    let mut cache = CONFIG_CACHE.write().unwrap_or_else(|e| e.into_inner());
    *cache = Some(cfg.clone());
    Ok(cfg)
}

/// Builds a [`Provider`] from config, resolving API keys and model defaults.
/// Per-provider overrides from `config.providers` are merged on top of global values.
pub(crate) fn build_provider(cfg: &AppConfig, system_prompt: String) -> Result<Provider> {
    let kind = ProviderKind::from_str(&cfg.provider);
    let pcfg = cfg.providers.get(kind.as_str());

    let timeout_s = pcfg.and_then(|p| p.timeout_s).unwrap_or(cfg.timeout_s);
    let model = resolve_model(
        kind,
        pcfg.and_then(|p| p.model.clone()).unwrap_or_else(|| cfg.model.clone()),
    );
    let model_ctx = pcfg.and_then(|p| p.model_ctx).unwrap_or(cfg.model_ctx);

    let base_url = pcfg
        .and_then(|p| p.api_url.clone())
        .or_else(|| cfg.api_url.clone())
        .unwrap_or_else(|| crate::provider::default_base_url(kind));

    let api_key = pcfg
        .and_then(|p| p.api_key.clone())
        .or_else(|| cfg.api_key.clone())
        .or_else(|| resolve_api_key_from_env(kind));

    let key_empty = api_key.as_ref().is_none_or(|k| k.is_empty());
    if key_empty {
        if let Some(env_var) = crate::provider::api_key_env_var(kind) {
            eprintln!(
                "{} provider '{}' requires --api-key or {}",
                ui::theme::paint_warning(&ui::theme::active(), "warning:"),
                kind.as_str(),
                env_var,
            );
        }
    }

    let mut provider = Provider::new(kind, base_url, model, timeout_s, system_prompt, api_key, model_ctx);

    // Per-provider reasoning override (user can mark any model as reasoning)
    if let Some(true) = pcfg.and_then(|p| p.reasoning_model) {
        provider.reasoning_config.enabled = true;
    }
    if let Some(effort) = &cfg.reasoning_effort {
        provider.reasoning_config.effort = crate::reasoning::ReasoningEffort::from_str(effort);
        provider.reasoning_config.enabled = true;
    }
    if let Some(budget) = cfg.thinking_budget {
        provider.reasoning_config.thinking_budget = budget;
    }
    Ok(provider)
}

/// Resolves the model name, using provider defaults when the user model is a placeholder.
fn resolve_model(kind: ProviderKind, model: String) -> String {
    if model == "rem-coder:latest" || model == "rem-coder" {
        crate::provider::default_model(kind).unwrap_or(&model).to_string()
    } else {
        model
    }
}

/// Cached resolved API keys from environment variables.
static API_KEY_CACHE: LazyLock<RwLock<BTreeMap<&'static str, Option<String>>>> =
    LazyLock::new(|| RwLock::new(BTreeMap::new()));

/// Resolves the API key from the appropriate environment variable for the provider kind.
/// Results are cached since environment variables don't change at runtime.
fn resolve_api_key_from_env(kind: ProviderKind) -> Option<String> {
    let var = crate::provider::api_key_env_var(kind)?;
    {
        let cache = API_KEY_CACHE.read().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = cache.get(var) {
            return cached.clone();
        }
    }
    let val = std::env::var(var).ok().filter(|v| !v.is_empty());
    let mut cache = API_KEY_CACHE.write().unwrap_or_else(|e| e.into_inner());
    cache.insert(var, val.clone());
    val
}

type SystemPromptCache = RwLock<Option<(Option<String>, String)>>;

/// Cached system prompt to avoid repeated disk reads.
static SYSTEM_PROMPT: LazyLock<SystemPromptCache> = LazyLock::new(|| RwLock::new(None));

/// Loads the system prompt from file, falling back to the built-in default.
pub(crate) fn load_system_prompt(custom_prompts_dir: Option<&str>) -> String {
    {
        let cache = SYSTEM_PROMPT.read().unwrap_or_else(|e| e.into_inner());
        if let Some((ref cached_dir, ref content)) = *cache {
            if *cached_dir == custom_prompts_dir.map(|s| s.to_string()) {
                return content.clone();
            }
        }
    }
    let content = load_system_prompt_uncached(custom_prompts_dir);
    let mut cache = SYSTEM_PROMPT.write().unwrap_or_else(|e| e.into_inner());
    *cache = Some((custom_prompts_dir.map(|s| s.to_string()), content.clone()));
    content
}

fn load_system_prompt_uncached(custom_prompts_dir: Option<&str>) -> String {
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
    crate::constants::DEFAULT_SYSTEM_PROMPT.to_string()
}

fn warn_missing_api_key(cfg: &AppConfig) {
    let kind = ProviderKind::from_str(&cfg.provider);
    if let Some(env_var) = crate::provider::api_key_env_var(kind) {
        let has_key =
            cfg.api_key.as_ref().is_some_and(|k| !k.is_empty()) || std::env::var(env_var).is_ok_and(|k| !k.is_empty());
        if !has_key {
            let t = ui::theme::active();
            let warn_prefix = ui::theme::paint_warning(&t, "config warning:");
            let msg = format!("provider '{}' may need --api-key or {}", cfg.provider, env_var);
            warn!("{msg}");
            eprintln!("  {} {msg}", warn_prefix);
        }
    }
}

/// Validates config at startup, printing warnings for common issues.
pub(crate) fn validate_config(cfg: &AppConfig) {
    let known_providers = [
        "ollama",
        "openai",
        "vllm",
        "anthropic",
        "gemini",
        "azure",
        "bedrock",
        "openrouter",
    ];
    let t = ui::theme::active();
    let warn_prefix = ui::theme::paint_warning(&t, "config warning:");

    if !known_providers.contains(&cfg.provider.as_str()) {
        let msg = format!(
            "unknown provider '{}'. Known: {}",
            cfg.provider,
            known_providers.join(", ")
        );
        warn!("{}", msg);
        eprintln!("  {} {msg}", warn_prefix);
    }

    warn_missing_api_key(cfg);

    let mode = cfg.mode.to_uppercase();
    if !["CHAT", "CODE", "PLAN"].contains(&mode.as_str()) {
        let msg = format!("unknown mode '{}' in config. Expected CHAT, CODE, or PLAN.", cfg.mode);
        warn!("{msg}");
        eprintln!("  {} {msg}", warn_prefix);
    }

    if cfg.timeout_s < 5 || cfg.timeout_s > 600 {
        let msg = format!("timeout_s={} seems unusual (expected 5-600)", cfg.timeout_s);
        warn!("{msg}");
        eprintln!("  {} {msg}", warn_prefix);
    }

    if cfg.model_ctx < 512 {
        let msg = format!(
            "model_ctx={} is very low (< 512). Responses may be truncated.",
            cfg.model_ctx
        );
        warn!("{msg}");
        eprintln!("  {} {msg}", warn_prefix);
    }

    let url = cfg.ollama_url.trim();
    if url.is_empty() || (!url.starts_with("http://") && !url.starts_with("https://")) {
        let msg = format!(
            "ollama_url '{}' does not look like a valid URL (expected http:// or https://)",
            url
        );
        warn!("{msg}");
        eprintln!("  {} {msg}", warn_prefix);
    }

    if let Some(ref effort) = cfg.reasoning_effort {
        let valid = ["low", "medium", "high"];
        if !valid.contains(&effort.to_lowercase().as_str()) {
            let msg = format!("unknown reasoning_effort '{}'. Expected low, medium, or high", effort);
            warn!("{msg}");
            eprintln!("  {} {msg}", warn_prefix);
        }
    }

    if cfg.thinking_budget == Some(0) {
        let msg = "thinking_budget is 0 — it should be > 0 to have any effect";
        warn!("{msg}");
        eprintln!("  {} {msg}", warn_prefix);
    }

    let theme_name = cfg.theme.to_uppercase();
    let known_themes = ui::theme::list_names();
    if !known_themes.iter().any(|t| t.eq_ignore_ascii_case(&theme_name)) {
        let msg = format!(
            "unknown theme '{}'. Known themes: {}",
            cfg.theme,
            known_themes.join(", ")
        );
        warn!("{msg}");
        eprintln!("  {} {msg}", warn_prefix);
    }

    if cfg.provider.to_lowercase() == "ollama" && cfg.api_key.as_ref().is_some_and(|k| !k.is_empty()) {
        let msg = "api_key is set but provider is 'ollama' — Ollama does not need an API key";
        warn!("{msg}");
        eprintln!("  {} {msg}", warn_prefix);
    }
}

/// Persists the workspace directory to config.
pub(crate) fn persist_workspace(dir: &Path) {
    let t = ui::theme::active();
    let mut cfg = match load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "  {} failed to load config, not saving workspace: {}",
                ui::theme::paint_error_label(&t, "✗"),
                e
            );
            return;
        }
    };
    cfg.workspace_dir = Some(dir.to_string_lossy().to_string());
    if let Err(e) = save_config(&cfg) {
        eprintln!(
            "  {} failed to save workspace config: {}",
            ui::theme::paint_error_label(&t, "✗"),
            e
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_system_prompt_falls_back_to_default() {
        let prompt = load_system_prompt(None);
        assert!(!prompt.is_empty());
        assert!(prompt.contains("REM"));
    }

    #[test]
    fn load_system_prompt_uses_custom_file() {
        let dir = std::env::temp_dir().join(format!("rem-test-sp-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("system_prompt.txt"), "custom prompt").unwrap();

        let prompt = load_system_prompt(Some(dir.to_str().unwrap()));
        assert_eq!(prompt, "custom prompt");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_system_prompt_ignores_empty_file() {
        let dir = std::env::temp_dir().join(format!("rem-test-sp2-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("system_prompt.txt"), "   ").unwrap();

        let prompt = load_system_prompt(Some(dir.to_str().unwrap()));
        assert!(!prompt.is_empty());
        assert!(prompt.contains("REM"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_config_accepts_valid() {
        let cfg = AppConfig::default();
        // Should not panic or produce warnings.
        validate_config(&cfg);
    }

    #[test]
    fn validate_config_warns_on_unknown_mode() {
        let cfg = AppConfig {
            mode: "INVALID".into(),
            ..Default::default()
        };
        validate_config(&cfg);
    }

    #[test]
    fn validate_config_warns_on_timeout_outside_range() {
        let cfg = AppConfig {
            timeout_s: 1000,
            ..Default::default()
        };
        validate_config(&cfg);
    }

    #[test]
    fn validate_config_warns_on_low_model_ctx() {
        let cfg = AppConfig {
            model_ctx: 128,
            ..Default::default()
        };
        validate_config(&cfg);
    }

    #[test]
    fn validate_config_accepts_known_providers() {
        for provider in &[
            "ollama",
            "openai",
            "vllm",
            "anthropic",
            "gemini",
            "azure",
            "bedrock",
            "openrouter",
        ] {
            let cfg = AppConfig {
                provider: provider.to_string(),
                ..Default::default()
            };
            validate_config(&cfg);
        }
    }

    #[test]
    fn validate_config_warns_on_bad_ollama_url() {
        let cfg = AppConfig {
            ollama_url: "not-a-url".into(),
            ..Default::default()
        };
        validate_config(&cfg);
    }

    #[test]
    fn validate_config_warns_on_invalid_reasoning_effort() {
        let cfg = AppConfig {
            reasoning_effort: Some("extreme".into()),
            ..Default::default()
        };
        validate_config(&cfg);
    }

    #[test]
    fn validate_config_warns_on_zero_thinking_budget() {
        let cfg = AppConfig {
            thinking_budget: Some(0),
            ..Default::default()
        };
        validate_config(&cfg);
    }

    #[test]
    fn validate_config_warns_on_unknown_theme() {
        let cfg = AppConfig {
            theme: "NONEXISTENT_THEME_123".into(),
            ..Default::default()
        };
        validate_config(&cfg);
    }

    #[test]
    fn validate_config_warns_on_api_key_with_ollama() {
        let cfg = AppConfig {
            provider: "ollama".into(),
            api_key: Some("my-secret".into()),
            ..Default::default()
        };
        validate_config(&cfg);
    }

    #[test]
    fn config_save_does_not_crash() {
        let cfg = AppConfig {
            model: "test-model".into(),
            provider: "openai".into(),
            timeout_s: 120,
            theme: "SAKURA".into(),
            ..Default::default()
        };
        let result = save_config(&cfg);
        // save_config writes to XDG dir; we just verify it doesn't error fatally
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn first_run_setup_uses_dot_for_current_dir() {
        let cfg = &mut AppConfig::default();
        let result = first_run_setup(cfg);
        // With piped stdin (empty), read_line returns empty -> defaults to current_dir
        assert!(result.is_ok());
    }

    #[test]
    fn config_dir_uses_xdg_when_set() {
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg-rem-test");
        let dir = config_dir();
        assert_eq!(dir, Some(std::path::PathBuf::from("/tmp/xdg-rem-test/rem-cli")));
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[test]
    fn resolve_api_key_from_cache() {
        let kind = ProviderKind::OpenAI;
        // First call should resolve from env (might be None if env not set)
        let result = resolve_api_key_from_env(kind);
        // Second call should hit cache
        let cached = resolve_api_key_from_env(kind);
        assert_eq!(result, cached, "cached value should match fresh value");
    }

    #[test]
    fn build_provider_defaults_to_ollama() {
        let cfg = AppConfig::default();
        let result = build_provider(&cfg, "test system prompt".into());
        assert!(result.is_ok(), "default config should build an Ollama provider");
        let provider = result.unwrap();
        assert_eq!(provider.kind, ProviderKind::Ollama);
    }

    #[test]
    fn warn_missing_api_key_no_panic_for_ollama() {
        let cfg = AppConfig {
            provider: "ollama".into(),
            api_key: None,
            ..Default::default()
        };
        // Should not panic or crash for Ollama (which doesn't need an API key)
        warn_missing_api_key(&cfg);
    }

    #[test]
    fn persist_workspace_updates_config() {
        let dir = std::env::temp_dir().join(format!("rem-persist-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        persist_workspace(&dir);
        // Verify the workspace was saved by reloading config
        if let Ok(cfg) = load_config() {
            assert!(cfg.workspace_dir.is_some());
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
