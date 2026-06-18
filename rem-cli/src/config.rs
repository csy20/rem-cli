//! Configuration loading, saving, and provider construction.
//! Reads `~/.config/rem-cli/config.toml` and `.remcli.toml`, merges them into
//! an [`AppConfig`], and builds the appropriate [`Provider`] from config values.

use crate::cli::{AppConfig, PartialConfig};
use crate::provider::{Provider, ProviderKind};
use crate::ui;
use anyhow::{Context, Result};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tracing::warn;

/// Returns the config directory path, checking XDG_CONFIG_HOME first.
fn config_dir() -> Option<std::path::PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let dir = std::path::PathBuf::from(xdg).join("rem-cli");
        return Some(dir);
    }
    dirs::home_dir().map(|h| h.join(".config/rem-cli"))
}

/// Saves config to XDG config dir or `~/.config/rem-cli/config.toml`.
pub(crate) fn save_config(cfg: &AppConfig) -> Result<()> {
    if let Some(dir) = config_dir() {
        fs::create_dir_all(&dir)?;
        let path = dir.join("config.toml");
        let text = toml::to_string_pretty(cfg).context("failed to serialize config")?;
        fs::write(&path, text).context("failed to write config")?;
    }
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
    print!(
        "{}",
        ui::theme::paint(&t, "accent", "\u{258C}  rem> ", true)
    );
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
pub(crate) fn load_config() -> Result<AppConfig> {
    let mut cfg = AppConfig::default();
    if let Some(dir) = config_dir() {
        let path = dir.join("config.toml");
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

/// Builds a [`Provider`] from config, resolving API keys and model defaults.
/// Per-provider overrides from `config.providers` are merged on top of global values.
pub(crate) fn build_provider(cfg: &AppConfig, system_prompt: String) -> Result<Provider> {
    let kind = ProviderKind::from_str(&cfg.provider);
    let pcfg = cfg.providers.get(kind.as_str());

    let timeout_s = pcfg.and_then(|p| p.timeout_s).unwrap_or(cfg.timeout_s);
    let model = pcfg
        .and_then(|p| p.model.clone())
        .unwrap_or_else(|| cfg.model.clone());
    let model_ctx = pcfg.and_then(|p| p.model_ctx).unwrap_or(cfg.model_ctx);

    let api_url = pcfg
        .and_then(|p| p.api_url.clone())
        .or_else(|| cfg.api_url.clone());
    let api_key = pcfg
        .and_then(|p| p.api_key.clone())
        .or_else(|| cfg.api_key.clone());

    let mut provider = match kind {
        ProviderKind::OpenAI => {
            let base_url = api_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string());
            let key =
                api_key.unwrap_or_else(|| std::env::var("OPENAI_API_KEY").unwrap_or_default());
            if key.is_empty() {
                eprintln!(
                    "{} provider 'openai' requires --api-key or OPENAI_API_KEY",
                    ui::theme::paint_warning(&ui::theme::active(), "warning:"),
                );
            }
            Provider::new_openai(base_url, model, timeout_s, system_prompt, key, model_ctx)
        }
        ProviderKind::Gemini => {
            let key =
                api_key.unwrap_or_else(|| std::env::var("GEMINI_API_KEY").unwrap_or_default());
            if key.is_empty() {
                eprintln!(
                    "{} provider 'gemini' requires --api-key or GEMINI_API_KEY",
                    ui::theme::paint_warning(&ui::theme::active(), "warning:"),
                );
            }
            let model = if model == "rem-coder:latest" || model == "rem-coder" {
                "gemini-2.0-flash".to_string()
            } else {
                model
            };
            Provider::new_gemini(key, model, timeout_s, system_prompt, model_ctx)
        }
        ProviderKind::Anthropic => {
            let key =
                api_key.unwrap_or_else(|| std::env::var("ANTHROPIC_API_KEY").unwrap_or_default());
            if key.is_empty() {
                eprintln!(
                    "{} provider 'anthropic' requires --api-key or ANTHROPIC_API_KEY",
                    ui::theme::paint_warning(&ui::theme::active(), "warning:"),
                );
            }
            let model = if model == "rem-coder:latest" || model == "rem-coder" {
                "claude-sonnet-4-20250514".to_string()
            } else {
                model
            };
            Provider::new_anthropic(key, model, timeout_s, system_prompt, model_ctx)
        }
        ProviderKind::Azure => {
            let base_url = api_url.unwrap_or_else(|| {
                std::env::var("AZURE_OPENAI_ENDPOINT")
                    .unwrap_or_else(|_| "https://api.openai.azure.com".to_string())
            });
            let key = api_key.unwrap_or_else(|| {
                std::env::var("AZURE_OPENAI_API_KEY")
                    .or_else(|_| std::env::var("AZURE_OPENAI_KEY"))
                    .unwrap_or_default()
            });
            if key.is_empty() {
                eprintln!(
                    "{} provider 'azure' requires --api-key or AZURE_OPENAI_API_KEY env var",
                    ui::theme::paint_warning(&ui::theme::active(), "warning:"),
                );
            }
            Provider::new_azure(base_url, model, timeout_s, system_prompt, key, model_ctx)
        }
        ProviderKind::Bedrock => {
            let model = if model == "rem-coder:latest" || model == "rem-coder" {
                "anthropic.claude-sonnet-4-20250514".to_string()
            } else {
                model
            };
            let key =
                api_key.unwrap_or_else(|| std::env::var("AWS_ACCESS_KEY_ID").unwrap_or_default());
            Provider::new_bedrock(model, timeout_s, system_prompt, key, model_ctx)
        }
        ProviderKind::OpenRouter => {
            let key =
                api_key.unwrap_or_else(|| std::env::var("OPENROUTER_API_KEY").unwrap_or_default());
            if key.is_empty() {
                eprintln!(
                    "{} provider 'openrouter' requires --api-key or OPENROUTER_API_KEY env var",
                    ui::theme::paint_warning(&ui::theme::active(), "warning:"),
                );
            }
            let model = if model == "rem-coder:latest" || model == "rem-coder" {
                "openai/gpt-4o".to_string()
            } else {
                model
            };
            Provider::new_openrouter(model, timeout_s, system_prompt, key, model_ctx)
        }
        ProviderKind::Ollama => {
            let base_url = api_url.unwrap_or_else(|| cfg.ollama_url.clone());
            Provider::new_ollama(base_url, model, timeout_s, system_prompt, model_ctx)
        }
    };
    if let Some(effort) = &cfg.reasoning_effort {
        provider.reasoning_config.effort = crate::reasoning::ReasoningEffort::from_str(effort);
        provider.reasoning_config.enabled = true;
    }
    if let Some(budget) = cfg.thinking_budget {
        provider.reasoning_config.thinking_budget = budget;
    }
    Ok(provider)
}

/// Loads the system prompt from file, falling back to the built-in default.
pub(crate) fn load_system_prompt(custom_prompts_dir: Option<&str>) -> String {
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
    crate::DEFAULT_SYSTEM_PROMPT.to_string()
}

/// Validates config at startup, printing warnings for common issues.
pub(crate) fn validate_config(cfg: &AppConfig) {
    let t = ui::theme::active();
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
    if !known_providers.contains(&cfg.provider.as_str()) {
        warn!(
            "unknown provider '{}'. Known: {}",
            cfg.provider,
            known_providers.join(", ")
        );
        eprintln!(
            "{} unknown provider '{}'. Known: {}",
            ui::theme::paint_warning(&t, "warning:"),
            cfg.provider,
            known_providers.join(", ")
        );
    }

    if cfg.provider == "openai" || cfg.provider == "vllm" {
        let has_key = cfg.api_key.as_ref().is_some_and(|k| !k.is_empty())
            || std::env::var("OPENAI_API_KEY").is_ok_and(|k| !k.is_empty());
        if !has_key {
            eprintln!(
                "{} provider '{}' may need --api-key or OPENAI_API_KEY",
                ui::theme::paint_warning(&t, "warning:"),
                cfg.provider
            );
        }
    }
    if cfg.provider == "anthropic" {
        let has_key = cfg.api_key.as_ref().is_some_and(|k| !k.is_empty())
            || std::env::var("ANTHROPIC_API_KEY").is_ok_and(|k| !k.is_empty());
        if !has_key {
            eprintln!(
                "{} provider 'anthropic' may need --api-key or ANTHROPIC_API_KEY",
                ui::theme::paint_warning(&t, "warning:"),
            );
        }
    }
    if cfg.provider == "gemini" {
        let has_key = cfg.api_key.as_ref().is_some_and(|k| !k.is_empty())
            || std::env::var("GEMINI_API_KEY").is_ok_and(|k| !k.is_empty());
        if !has_key {
            eprintln!(
                "{} provider 'gemini' may need --api-key or GEMINI_API_KEY",
                ui::theme::paint_warning(&t, "warning:"),
            );
        }
    }
    if cfg.provider == "azure" {
        let has_key = cfg.api_key.as_ref().is_some_and(|k| !k.is_empty())
            || std::env::var("AZURE_OPENAI_API_KEY").is_ok_and(|k| !k.is_empty());
        if !has_key {
            eprintln!(
                "{} provider 'azure' may need --api-key or AZURE_OPENAI_API_KEY",
                ui::theme::paint_warning(&t, "warning:"),
            );
        }
    }
    if cfg.provider == "openrouter" {
        let has_key = cfg.api_key.as_ref().is_some_and(|k| !k.is_empty())
            || std::env::var("OPENROUTER_API_KEY").is_ok_and(|k| !k.is_empty());
        if !has_key {
            eprintln!(
                "{} provider 'openrouter' may need --api-key or OPENROUTER_API_KEY",
                ui::theme::paint_warning(&t, "warning:"),
            );
        }
    }

    let mode = cfg.mode.to_uppercase();
    if !["CHAT", "CODE", "PLAN"].contains(&mode.as_str()) {
        eprintln!(
            "{} unknown mode '{}' in config. Expected CHAT, CODE, or PLAN.",
            ui::theme::paint_warning(&t, "warning:"),
            cfg.mode
        );
    }

    if cfg.timeout_s < 5 || cfg.timeout_s > 600 {
        eprintln!(
            "{} timeout_s={} seems unusual (expected 5-600)",
            ui::theme::paint_warning(&t, "warning:"),
            cfg.timeout_s
        );
    }

    if cfg.model_ctx < 512 {
        eprintln!(
            "{} model_ctx={} is very low (< 512). Responses may be truncated.",
            ui::theme::paint_warning(&t, "warning:"),
            cfg.model_ctx
        );
    }
}

/// Persists the workspace directory to config.
pub(crate) fn persist_workspace(dir: &Path) {
    let t = ui::theme::active();
    let mut cfg = load_config().unwrap_or_default();
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
        let mut cfg = AppConfig::default();
        cfg.mode = "INVALID".into();
        validate_config(&cfg);
    }

    #[test]
    fn validate_config_warns_on_timeout_outside_range() {
        let mut cfg = AppConfig::default();
        cfg.timeout_s = 1000;
        validate_config(&cfg);
    }

    #[test]
    fn validate_config_warns_on_low_model_ctx() {
        let mut cfg = AppConfig::default();
        cfg.model_ctx = 128;
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
            let mut cfg = AppConfig::default();
            cfg.provider = provider.to_string();
            validate_config(&cfg);
        }
    }
}
