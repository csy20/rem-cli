//! Binary entry point for the REM coding assistant CLI.
//! Defines top-level types ([`FileEntry`], [`ModelReply`]), prompt constants,
//! utility functions, and dispatches to subcommands or the REPL loop.

use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use anyhow::{anyhow, Context, Result};
use clap::Parser;

use cli::{AppConfig, AskArgs, Cli, Commands, ExplainArgs, IndexArgs, NewArgs, PatchArgs};
use walkdir::WalkDir;

mod agentic;
mod chat;
mod cli;
mod commands;
mod config;
mod feedback;
mod find;
mod highlight;
mod indexer;
mod intent;
mod memory;
mod parsing;
mod provider;
mod repl;
mod search;
mod templates;
mod types;
mod ui;

use crate::chat::check_system_resources;
use crate::config::{build_provider, load_config, load_system_prompt};
use crate::intent::{classify_intent, TaskIntent};
use crate::types::*;
use crate::ui::output::SpinnerGuard;
use indexer::{generate_codebase_index, load_codebase_index, write_codebase_index};

use provider::Provider;

static CTRL_C_COUNT: AtomicU8 = AtomicU8::new(0);
static SHOULD_EXIT: AtomicBool = AtomicBool::new(false);

/// Registers a global Ctrl+C handler that cancels streams on first press,
/// and signals graceful exit on second press.
fn setup_global_ctrlc_handler() {
    let _handle = tokio::spawn(async {
        loop {
            let _ = tokio::signal::ctrl_c().await;
            let count = CTRL_C_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
            if count >= 2 {
                SHOULD_EXIT.store(true, Ordering::SeqCst);
                eprintln!("\n  \u{00d7} exiting (Ctrl+C pressed twice)");
            } else {
                provider::STREAM_CANCELLED.store(true, Ordering::SeqCst);
                eprintln!("\n  \u{00d7} cancelling... (Ctrl+C again to exit)");
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

pub(crate) const CHAT_SYSTEM_PROMPT_CONVERSATIONAL: &str = r##"You are REM, a helpful coding assistant in conversation mode.

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

pub(crate) const CHAT_SYSTEM_PROMPT_CODE: &str = r##"You are REM, a coding assistant in code generation mode.

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

pub(crate) const CHAT_SYSTEM_PROMPT_PLAN: &str = r##"You are REM, a coding assistant in planning mode.

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
    let mut client = build_provider(&cfg, system_prompt)?;
    client.healthcheck().await?;
    let models = client.list_models().await?;
    if !models.iter().any(|m| m == &cfg.model) {
        let fallback = models.first().cloned().unwrap_or_else(|| cfg.model.clone());
        eprintln!(
            "\x1b[33mwarning\x1b[0m: model '{}' not found; using '{}'",
            cfg.model, fallback
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
            let is_pipe = !std::io::stdin().is_terminal();
            if is_pipe {
                let mut stdin_data = String::new();
                if io::stdin().read_to_string(&mut stdin_data).is_ok()
                    && !stdin_data.trim().is_empty()
                {
                    return run_pipe(&client, &cfg, stdin_data.trim(), verbose).await;
                }
            }
            repl::run_chat(&mut client, &mut cfg, verbose).await
        }
        Some(Commands::Index(_)) => {
            // handled by early return before client creation
            unreachable!("Index command should have been handled earlier")
        }
    }
}

async fn run_pipe(client: &Provider, _cfg: &AppConfig, input: &str, verbose: bool) -> Result<()> {
    let t = ui::theme::active();
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
                eprintln!(
                    "\n  {} raw:\n{}\n",
                    ui::theme::paint_dim(&t, "verbose:"),
                    text
                );
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
    let t = ui::theme::active();
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
            let text = client
                .complete_chat_stream(&composed, system_prompt, "")
                .await?;
            Ok(ModelReply {
                explanation: text.trim().to_string(),
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
            ui::theme::paint_dim(&t, "verbose:"),
            reply.explanation
        );
        eprintln!(
            "{} raw files: {:?}",
            ui::theme::paint_dim(&t, "verbose:"),
            reply.files
        );
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
    let t = ui::theme::active();
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
        ui::theme::paint(
            &t,
            "accent",
            &format!("Patch preview for {}", args.file.display()),
            true
        )
    );
    print_reply(&reply, true);
    Ok(())
}

// ── Output formatting ──────────────────────────────────────────────────────

fn print_banner(client: &Provider) {
    let t = ui::theme::active();
    println!();
    ui::theme::println(&ui::theme::paint_rail(&t, "accent", "text_muted", "REM"));
    ui::theme::println(&format!(
        "  {} {} {}  {}",
        ui::theme::paint(&t, "accent_dim", "\u{258C}", true),
        ui::theme::paint(&t, "text_faint", "provider", false),
        ui::theme::paint(&t, "accent", &client.provider_label(), false),
        ui::theme::paint(&t, "text_faint", "\u{00B7} type /help for commands", false)
    ));
}

fn print_reply(reply: &ModelReply, newline: bool) {
    let t = ui::theme::active();
    if newline {
        println!();
    }
    if !reply.explanation.trim().is_empty() {
        ui::theme::println(&format!(
            "  {} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            reply.explanation
        ));
    }

    if !reply.files.is_empty() {
        ui::theme::println(&format!(
            "  {}",
            ui::theme::paint_success(&t, &format!("generated: {} file(s)", reply.files.len()))
        ));
        for f in &reply.files {
            let icon = file_icon(&f.path);
            if f.path.is_empty() {
                ui::theme::println(&format!(
                    "    {}  {}",
                    icon,
                    ui::theme::paint(
                        &t,
                        "accent_dim",
                        &format!("(unnamed) {} bytes", f.content.len()),
                        false
                    )
                ));
            } else {
                ui::theme::println(&format!(
                    "    {}  {}",
                    icon,
                    ui::theme::paint(&t, "accent", &f.path, false)
                ));
            }
        }
        ui::theme::println(&format!(
            "    {}",
            ui::theme::paint(&t, "text_faint", "/write <path> to save", false)
        ));
    } else if !reply.code.trim().is_empty() {
        ui::theme::println(&format!("  {}", ui::theme::paint_success(&t, "code:")));
        for code_line in reply.code.lines() {
            ui::theme::println(&format!(
                "    {}",
                ui::theme::paint(&t, "accent_dim", code_line, false)
            ));
        }
        ui::theme::println(&format!(
            "    {}",
            ui::theme::paint(&t, "text_faint", "/write <path> to save", false)
        ));
    }
    if !reply.commands.is_empty() {
        ui::theme::println(&format!(
            "  {}",
            ui::theme::paint(&t, "accent", "commands:", true)
        ));
        for cmd in sanitize_commands(&reply.commands) {
            if is_command_blocked(cmd) {
                ui::theme::println(&format!(
                    "    {}",
                    ui::theme::paint_error(&t, &format!("[blocked] {}", cmd))
                ));
            } else {
                ui::theme::println(&format!(
                    "    $ {}",
                    ui::theme::paint(&t, "accent_dim", cmd, false)
                ));
            }
        }
    }
    if !reply.checks.is_empty() {
        ui::theme::println(&format!(
            "  {}",
            ui::theme::paint(&t, "accent", "checks:", true)
        ));
        for item in &reply.checks {
            ui::theme::println(&format!(
                "    {}",
                ui::theme::paint(&t, "text_muted", &format!("\u{2022} {}", item), false)
            ));
        }
    }
    if !reply.caution.trim().is_empty() {
        ui::theme::println(&format!(
            "  {}",
            ui::theme::paint_error(&t, &format!("caution: {}", reply.caution))
        ));
    }
}

// ── Context builder ────────────────────────────────────────────────────────

/// Builds a directory snapshot and file content context string.
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

// ── Project scaffolding ────────────────────────────────────────────────────

fn run_new(args: NewArgs, cfg: &AppConfig) -> Result<()> {
    let t = ui::theme::active();
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
        "bare" => templates::template_bare(&args.name),
        "portfolio" => templates::template_portfolio(&args.name),
        "landing" => templates::template_landing(&args.name),
        "blog" => templates::template_blog(&args.name),
        "rust" => templates::template_rust(&args.name),
        "python" => templates::template_python(&args.name),
        "go" => templates::template_go(&args.name),
        "javascript" => templates::template_javascript(&args.name),
        other => {
            return Err(anyhow!(
                "Unknown project type '{}'. Choose: bare, portfolio, landing, blog, rust, python, go, javascript",
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
        ui::theme::paint_success_label(&t, "✓"),
        ui::theme::paint_bright(
            &t,
            &format!("created project '{}' ({})", args.name, args.project_type)
        )
    );
    for f in &files {
        let icon = file_icon(&f.path);
        println!(
            "  {} {} ({} bytes)",
            icon,
            ui::theme::paint_bright(&t, &f.path),
            f.content.len()
        );
    }
    println!();
    println!(
        "{} cd {} && open index.html",
        ui::theme::paint_dim(&t, "next:"),
        args.name
    );

    Ok(())
}
// run_index delegates to the indexer module (see src/indexer.rs).
// The thin wrapper keeps the CLI printing / arg handling in main while the
// pure logic (chunking, writing, loading, retrieval) lives in its own module.

fn run_index(args: IndexArgs, cfg: &AppConfig) -> Result<()> {
    let t = ui::theme::active();
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

    println!("{}", ui::theme::paint(&t, "accent", "\u{258C}", true));
    println!(
        "{} {} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "rem index"),
        ui::theme::paint_dim(&t, "— codebase retrieval index (pure Rust)")
    );
    println!(
        "{} target: {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_dim(&t, &dir.display().to_string())
    );

    let refreshing = load_codebase_index(&dir).is_some();
    let chunks = generate_codebase_index(&dir)?;
    if chunks.is_empty() {
        println!(
            "{} {} no indexable files found (after skips)",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_warning(&t, "⚠")
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
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_success_label(&t, "✓"),
        action,
        chunks.len(),
        unique_files
    );
    println!(
        "{} index: {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, &out_path.display().to_string())
    );
    println!("{} `rem chat` / `rem ask` / `/goal` will now pull relevant chunks instead of full listings.", ui::theme::paint(&t, "accent", "\u{258C}", true));
    println!("{} (keyword retrieval; raise model_ctx in ~/.config/rem-cli/config.toml for large projects)", ui::theme::paint(&t, "accent", "\u{258C}", true));
    Ok(())
}
