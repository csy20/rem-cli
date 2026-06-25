//! Binary entry point for the REM coding assistant CLI.
//! Defines top-level types ([`FileEntry`], [`ModelReply`]), prompt constants,
//! utility functions, and dispatches to subcommands or the REPL loop.

use std::io::{IsTerminal, Read};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use anyhow::Result;
use clap::Parser;

mod agentic;
mod blocklist;
mod chat;
mod cli;
mod commands;
mod config;
mod constants;
mod feedback;
mod find;
mod highlight;
mod indexer;
mod intent;
mod memory;
mod pager;
mod parsing;
mod provider;
mod reasoning;
mod repl;
mod search;
mod session_io;
mod templates;
mod text_util;
mod token_count;
mod tool_executor;
mod types;
mod ui;
mod vision;
mod watcher;

use crate::cli::{Cli, Commands};
use crate::commands::runner::*;
use crate::config::{build_provider, load_config, load_system_prompt, validate_config};
use crate::session_io::check_system_resources;
use crate::ui::theme;
use crate::{text_util::*, types::*};

pub(crate) static CTRL_C_COUNT: AtomicU8 = AtomicU8::new(0);
pub(crate) static SHOULD_EXIT: AtomicBool = AtomicBool::new(false);

/// Registers a global Ctrl+C handler that cancels streams on first press,
/// and signals graceful exit on second press.
/// Prints nothing — UI messages come from the REPL readline handler.
fn setup_global_ctrlc_handler() {
    let _handle = tokio::spawn(async {
        loop {
            match tokio::signal::ctrl_c().await {
                Ok(()) => {
                    let count = CTRL_C_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
                    if count >= 2 {
                        SHOULD_EXIT.store(true, Ordering::SeqCst);
                    }
                    provider::STREAM_CANCELLED.store(true, Ordering::SeqCst);
                }
                Err(e) => {
                    tracing::error!("ctrl-c handler error: {}", e);
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }
    });
}

/// Resets the Ctrl+C state (called before entering the REPL loop).
pub(crate) fn reset_ctrlc_count() {
    CTRL_C_COUNT.store(0, Ordering::SeqCst);
    SHOULD_EXIT.store(false, Ordering::SeqCst);
}

/// Returns `true` if the user pressed Ctrl+C twice and wants to exit.
pub(crate) fn exit_requested() -> bool {
    SHOULD_EXIT.load(Ordering::SeqCst)
}

// ── Entry point ────────────────────────────────────────────────────────────

fn init_tracing() {
    use tracing_subscriber::filter::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
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

    validate_config(&cfg);

    if let Some(Commands::New(args)) = cli.command {
        return run_new(args, &cfg);
    }
    if let Some(Commands::Index(args)) = cli.command {
        return run_index(args, &cfg).await;
    }
    if let Some(Commands::Pull(args)) = cli.command {
        return run_pull(args, &cfg);
    }

    let system_prompt = load_system_prompt(cfg.prompts_dir.as_deref());
    let mut client = build_provider(&cfg, system_prompt)?;
    let models = client.list_models().await?;
    if !models.iter().any(|m| m == &cfg.model) {
        let fallback = models.first().cloned().unwrap_or_else(|| cfg.model.clone());
        let t = theme::active();
        eprintln!(
            "{} model '{}' not found; using '{}'",
            theme::paint_warning(&t, "warning:"),
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
        Some(Commands::Theme(args)) => run_theme(args),
        Some(Commands::New(_)) | Some(Commands::Pull(_)) | Some(Commands::Index(_)) => {
            unreachable!("New/Pull/Index handled by early return before client creation")
        }
        None => {
            let is_pipe = !std::io::stdin().is_terminal();
            if is_pipe {
                let mut stdin_data = String::new();
                if std::io::stdin().read_to_string(&mut stdin_data).is_ok() && !stdin_data.trim().is_empty() {
                    return run_pipe(&client, &cfg, stdin_data.trim(), verbose).await;
                }
            }
            repl::run_chat(&mut client, &mut cfg, verbose).await
        }
    }
}
