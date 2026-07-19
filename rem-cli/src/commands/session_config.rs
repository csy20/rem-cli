use crate::chat::ChatSession;
use crate::provider::Provider;
use crate::token_count::{context_usage_percent, estimate_tokens_batch};
use crate::ui;

/// Displays current configuration (`/config` command).
pub(crate) fn handle_config(session: &ChatSession, client: &Provider) {
    let t = ui::theme::active();
    println!("{}", ui::theme::paint_rail_header(&t, "CONFIG"));
    println!(
        "{}   {:<18} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "provider:"),
        ui::theme::paint_dim(&t, client.kind.as_str())
    );
    println!(
        "{}   {:<18} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "model:"),
        ui::theme::paint_dim(&t, &client.ctx.model)
    );
    println!(
        "{}   {:<18} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "base url:"),
        ui::theme::paint_dim(&t, &client.ctx.base_url)
    );
    println!(
        "{}   {:<18} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "mode:"),
        ui::theme::paint_dim(&t, session.mode.label())
    );
    println!(
        "{}   {:<18} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "workspace:"),
        ui::theme::paint_dim(
            &t,
            &session
                .ctx
                .project_dir
                .as_ref()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| "none".to_string())
        )
    );
    println!("{}", ui::theme::paint_rail_empty(&t));
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_dim(&t, "/model <name>  /provider <name>  /config workspace <path>")
    );
    println!("{}", ui::theme::paint(&t, "accent", "\u{258C}", true));
}

/// Sets a configuration value (`/config workspace <path>`).
pub(crate) fn handle_config_set(session: &mut ChatSession, client: &Provider, args: &str) {
    let t = ui::theme::active();
    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    if parts.is_empty() {
        handle_config(session, client);
        return;
    }
    match parts[0] {
        "workspace" | "dir" => {
            if parts.len() > 1 {
                crate::commands::handle_dir(session, parts[1]);
            } else {
                println!(
                    "{} usage: /config workspace <path>",
                    ui::theme::paint_warning(&t, "\u{258C}")
                );
            }
        }
        "edit" => {
            let config_path = crate::config::config_dir().unwrap_or_default().join("config.toml");
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
            println!(
                "{} {} opening {} with {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_dim(&t, "\u{270E}"),
                ui::theme::paint_bright(&t, &config_path.display().to_string()),
                editor
            );
            match std::process::Command::new(&editor).arg(&config_path).status() {
                Ok(status) if status.success() => {
                    crate::config::invalidate_config_cache();
                    println!(
                        "{} {} config reloaded",
                        ui::theme::paint(&t, "accent", "\u{258C}", true),
                        ui::theme::paint_success_label(&t, "\u{2713}")
                    );
                }
                Ok(_) => println!(
                    "{} {} editor exited with error",
                    ui::theme::paint(&t, "accent", "\u{258C}", true),
                    ui::theme::paint_warning(&t, "\u{2717}")
                ),
                Err(e) => println!(
                    "{} {} failed to launch {}: {}",
                    ui::theme::paint(&t, "accent", "\u{258C}", true),
                    ui::theme::paint_error_label(&t, "\u{2717}"),
                    editor,
                    e
                ),
            }
        }
        other => {
            println!(
                "{} unknown config key: {}",
                ui::theme::paint_warning(&t, "\u{258C}"),
                other
            );
            println!(
                "{} usage: /config workspace <path> | /config edit",
                ui::theme::paint_rail_empty(&t)
            );
        }
    }
}

/// Shows token usage and speed stats (`/tokens` command).
pub(crate) fn handle_tokens(session: &ChatSession, client: &Provider) {
    let tokens = session.last_tokens;
    let elapsed = session.last_elapsed.as_secs_f64();
    let history_tokens: usize = session
        .history_mgr
        .history
        .iter()
        .map(|(u, a)| estimate_tokens_batch(&[u, a]))
        .sum();
    let model_ctx = client.ctx.model_ctx;
    let pct = context_usage_percent(history_tokens, model_ctx);
    let t = ui::theme::active();

    println!("{}", ui::theme::paint_rail_empty(&t));
    println!(
        "{}  {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "\u{2500}\u{2500} TOKENS \u{2500}\u{2500}"),
    );
    println!(
        "{}   {:<18} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "last response:"),
        ui::theme::paint_dim(&t, &format!("~{} tokens", tokens))
    );

    if elapsed > 0.0 && tokens > 0 {
        let tps = tokens as f64 / elapsed;
        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "speed:"),
            ui::theme::paint_dim(&t, &format!("~{:.0} tok/s", tps))
        );
    }

    if session.last_elapsed.as_secs() > 0 {
        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "elapsed:"),
            ui::theme::paint_dim(&t, &format!("{:.1}s", elapsed))
        );
    }

    if history_tokens > 0 {
        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "context history:"),
            ui::theme::paint_dim(
                &t,
                &format!(
                    "~{} tokens ({} turns)",
                    history_tokens,
                    session.history_mgr.history.len()
                )
            )
        );

        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "context window:"),
            ui::theme::paint_dim(&t, &format!("{:.0}% used ({} limit)", pct, model_ctx))
        );
    } else {
        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "context:"),
            ui::theme::paint_dim(&t, "empty (no history)")
        );
    }

    let usage = client.anthropic_usage();
    if usage.input_tokens > 0 || usage.output_tokens > 0 {
        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "input tokens:"),
            ui::theme::paint_dim(&t, &usage.input_tokens.to_string())
        );
        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "output tokens:"),
            ui::theme::paint_dim(&t, &usage.output_tokens.to_string())
        );
        if usage.cache_creation_input_tokens > 0 {
            println!(
                "{}   {:<18} {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_bright(&t, "cache created:"),
                ui::theme::paint_dim(&t, &format!("{} tokens", usage.cache_creation_input_tokens))
            );
        }
        if usage.cache_read_input_tokens > 0 {
            println!(
                "{}   {:<18} {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_bright(&t, "cache read:"),
                ui::theme::paint_dim(&t, &format!("{} tokens", usage.cache_read_input_tokens))
            );
        }
    }

    println!("{}", ui::theme::paint_rail_empty(&t));
}

/// Reloads config from disk and re-scans project context (`/reload` command).
pub(crate) fn handle_reload(session: &mut ChatSession, cfg: &mut crate::cli::AppConfig) {
    use crate::config::{invalidate_config_cache, load_config};
    let t = ui::theme::active();
    invalidate_config_cache();
    match load_config() {
        Ok(new_cfg) => {
            let old_workspace = cfg.workspace_dir.clone();
            let old_theme = cfg.theme.clone();
            *cfg = new_cfg;
            if cfg.workspace_dir.is_none() {
                cfg.workspace_dir = old_workspace;
            }
            if old_theme != cfg.theme {
                ui::theme::set_active(&cfg.theme);
            }
            crate::pager::init_page_threshold(cfg.page_threshold);
            session.ctx.invalidate_caches();
            if let Some(ref dir) = cfg.workspace_dir {
                let path = std::path::PathBuf::from(dir);
                if path.exists() {
                    session.ctx.project_dir = Some(path);
                    session.ctx.project_memory = crate::memory::ProjectMemory::load(
                        session
                            .ctx
                            .project_dir
                            .as_deref()
                            .unwrap_or_else(|| std::path::Path::new(".")),
                    );
                }
            }
            println!(
                "{} {}",
                ui::theme::paint_success_label(&t, "\u{2713}"),
                ui::theme::paint_dim(&t, "config reloaded from disk")
            );
        }
        Err(e) => {
            println!(
                "{} {}",
                ui::theme::paint_error_label(&t, "\u{2717}"),
                ui::theme::paint(&t, "error", &format!("failed to reload config: {e}"), false)
            );
        }
    }
}

/// Shows debug context being sent to the model (`/context` command).
pub(crate) fn handle_context(session: &ChatSession, client: &Provider) {
    let t = ui::theme::active();
    let history = session.history_mgr.build_chat_history();
    let turn_count = session.history_mgr.history.len();
    let elapsed = session.session_start.elapsed();

    println!("{}", ui::theme::paint_rail_header(&t, "CONTEXT DEBUG"));
    println!(
        "{} {} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "Provider:"),
        ui::theme::paint_dim(&t, client.kind.as_str()),
    );
    println!(
        "{} {} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "Model:"),
        ui::theme::paint_dim(&t, &client.ctx.model),
    );
    println!(
        "{} {} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "Mode:"),
        ui::theme::paint_dim(&t, session.mode.label()),
    );
    println!(
        "{} {} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "Turns:"),
        ui::theme::paint_dim(&t, &turn_count.to_string()),
    );
    println!(
        "{} {} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "Total Tokens:"),
        ui::theme::paint_dim(&t, &session.total_tokens.to_string()),
    );
    println!(
        "{} {} {:.1}s",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "Duration:"),
        elapsed.as_secs_f64(),
    );
    println!("{}", ui::theme::paint_rail_empty(&t));

    println!(
        "{} Chat history: {} chars, ~{} tokens",
        ui::theme::paint_bright(&t, "\u{258C}"),
        history.len(),
        crate::token_count::estimate_tokens(&history),
    );
    if !history.is_empty() {
        println!("{}", ui::theme::paint_rail_header(&t, "HISTORY"));
        println!("{}", history);
    }
}
