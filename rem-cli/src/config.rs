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

/// Saves config to `~/.config/rem-cli/config.toml`.
pub(crate) fn save_config(cfg: &AppConfig) -> Result<()> {
    if let Some(home) = dirs::home_dir() {
        let dir = home.join(".config/rem-cli");
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

/// Loads and merges global (`~/.config/rem-cli/config.toml`) and local (`.remcli.toml`) config.
pub(crate) fn load_config() -> Result<AppConfig> {
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

/// Builds a [`Provider`] from config, resolving API keys and model defaults.
pub(crate) fn build_provider(cfg: &AppConfig, system_prompt: String) -> Result<Provider> {
    let kind = ProviderKind::from_str(&cfg.provider);
    match kind {
        ProviderKind::OpenAI => {
            let base_url = cfg
                .api_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
            let key = cfg
                .api_key
                .clone()
                .unwrap_or_else(|| std::env::var("OPENAI_API_KEY").unwrap_or_default());
            if key.is_empty() {
                eprintln!(
                    "\x1b[33mwarning\x1b[0m: provider 'openai' requires --api-key or OPENAI_API_KEY"
                );
            }
            Ok(Provider::new_openai(
                base_url,
                cfg.model.clone(),
                cfg.timeout_s,
                system_prompt,
                key,
                cfg.model_ctx,
            ))
        }
        ProviderKind::Gemini => {
            let key = cfg
                .api_key
                .clone()
                .unwrap_or_else(|| std::env::var("GEMINI_API_KEY").unwrap_or_default());
            if key.is_empty() {
                eprintln!(
                    "\x1b[33mwarning\x1b[0m: provider 'gemini' requires --api-key or GEMINI_API_KEY"
                );
            }
            let model = if cfg.model == "rem-coder:latest" || cfg.model == "rem-coder" {
                "gemini-2.0-flash".to_string()
            } else {
                cfg.model.clone()
            };
            Ok(Provider::new_gemini(
                key,
                model,
                cfg.timeout_s,
                system_prompt,
                cfg.model_ctx,
            ))
        }
        ProviderKind::Anthropic => {
            let key = cfg
                .api_key
                .clone()
                .unwrap_or_else(|| std::env::var("ANTHROPIC_API_KEY").unwrap_or_default());
            if key.is_empty() {
                eprintln!(
                    "\x1b[33mwarning\x1b[0m: provider 'anthropic' requires --api-key or ANTHROPIC_API_KEY"
                );
            }
            let model = if cfg.model == "rem-coder:latest" || cfg.model == "rem-coder" {
                "claude-sonnet-4-20250514".to_string()
            } else {
                cfg.model.clone()
            };
            Ok(Provider::new_anthropic(
                key,
                model,
                cfg.timeout_s,
                system_prompt,
                cfg.model_ctx,
            ))
        }
        _ => {
            let base_url = cfg
                .api_url
                .clone()
                .unwrap_or_else(|| cfg.ollama_url.clone());
            Ok(Provider::new_ollama(
                base_url,
                cfg.model.clone(),
                cfg.timeout_s,
                system_prompt,
                cfg.model_ctx,
            ))
        }
    }
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
}
