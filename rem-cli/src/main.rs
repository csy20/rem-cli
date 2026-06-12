use std::collections::BTreeMap;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::LazyLock;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use regex::Regex;
use serde::{Deserialize, Serialize};

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
mod search;
mod templates;
mod ui;

use crate::chat::build_prompt;
use crate::chat::{
    check_system_resources, detect_project_type, language_specific_guidance, print_welcome,
    validate_chat_response, ChatSession, RunMode,
};
use crate::commands::{
    auto_write_files, handle_compact, handle_config, handle_config_set, handle_copy, handle_diff,
    handle_dir, handle_explain, handle_find, handle_goal, handle_init, handle_lint,
    handle_list_files, handle_memory, handle_memory_set, handle_refactor, handle_resume_session,
    handle_review, handle_save_session, handle_search, handle_test, handle_tokens, handle_undo,
    handle_write, print_chat_help, print_last_files, prompt_for_path,
};
use crate::config::{
    build_provider, first_run_setup, load_config, load_system_prompt, save_config,
};
use indexer::{generate_codebase_index, load_codebase_index, write_codebase_index};
use intent::{classify_intent, has_creation_intent, has_file_path, intent_instruction, TaskIntent};
use parsing::{current_name_from_bold, extract_code_block, guess_filename};
use provider::Provider;
use ui::output::SpinnerGuard;

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

// ── Lazy-compiled regexes ──────────────────────────────────────────────────

static RE_AT_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@([^\s]+)").expect("invalid regex literal"));

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

const BLOCKED_COMMAND_PATTERNS: [&str; 6] = [
    "rm -rf /",
    "mkfs",
    "dd if=",
    ":(){:|:&};:",
    "shutdown",
    "reboot",
];

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct FileEntry {
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

pub(crate) fn extract_code_blocks_with_names(text: &str) -> Vec<FileEntry> {
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

pub(crate) fn resolve_safe_path(base: &Path, rel: &str) -> Option<PathBuf> {
    let t = ui::theme::active();
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
            ui::theme::paint_error_label(&t, "\u{2717}"),
            ui::theme::paint_warning(&t, rel)
        );
        None
    }
}

// ── Model reply schema ─────────────────────────────────────────────────────

pub(crate) fn truncate_to_lines(s: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = s.lines().take(max_lines).collect();
    let mut result = lines.join("\n");
    if s.lines().count() > max_lines {
        result.push_str("\n...[truncated]");
    }
    result
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
            run_chat(&mut client, &mut cfg, verbose).await
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

async fn run_chat(client: &mut Provider, cfg: &mut AppConfig, verbose: bool) -> Result<()> {
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

    ui::theme::set_active(&cfg.theme);
    let mut session = ChatSession::new(&client.model, workspace.clone())?;
    let saved_mode = cfg.mode.to_uppercase();
    session.mode = match saved_mode.as_str() {
        "CODE" => RunMode::Code,
        "PLAN" => RunMode::Plan,
        _ => RunMode::Chat,
    };
    let t = ui::theme::active();
    print_welcome(client);
    if let Some(ref wd) = workspace {
        ui::theme::println(&format!(
            "  {} {} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint(&t, "text_faint", "workspace \u{2192}", false),
            ui::theme::paint(&t, "accent_dim", &wd.display().to_string(), false)
        ));
    }

    loop {
        let prompt = build_prompt(&session, client);
        let mut error_count = 0u8;
        let line = loop {
            let line = session.readline(&prompt);
            match line {
                Ok(s) => break s,
                Err(e) => {
                    eprintln!(
                        "  {} input error: {}",
                        ui::theme::paint_error_label(&t, "err:"),
                        e
                    );
                    if e.kind() == io::ErrorKind::Interrupted
                        || e.kind() == io::ErrorKind::UnexpectedEof
                    {
                        return Ok(());
                    }
                    error_count += 1;
                    if error_count >= 3 {
                        eprintln!(
                            "  {} too many errors, exiting",
                            ui::theme::paint_error_label(&t, "err:")
                        );
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
            println!("  {}", ui::theme::paint_dim(&t, "bye!"));
            break;
        }

        if trimmed.eq_ignore_ascii_case("/help") || trimmed.eq_ignore_ascii_case("help") {
            print_chat_help();
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/theme") {
            let themes = ui::theme::list_names();
            println!("{}", ui::theme::paint_rail_empty(&t));
            println!(
                "{} {}",
                ui::theme::paint_rail_empty(&t),
                ui::theme::paint_bright(&t, "themes")
            );
            println!("{}", ui::theme::paint_rail_empty(&t));
            for name in &themes {
                let preview = ui::theme::by_name(name);
                let is_active = name == &t.name;
                let marker = if is_active { "\u{25C8}" } else { "\u{25C7}" };
                let accent = ui::theme::paint(&preview, "accent", marker, true);
                let label = if is_active {
                    ui::theme::paint_bright(&preview, &format!(" {} (active)", name))
                } else {
                    ui::theme::paint(&preview, "accent_dim", &format!(" {}", name), false)
                };
                let swatch = ui::theme::paint_on(&preview, "accent", "surface", "  ", false);
                println!("{accent} {label}  {swatch}");
            }
            println!("{}", ui::theme::paint_rail_empty(&t));
            println!(
                "{} {}",
                ui::theme::paint_rail_empty(&t),
                ui::theme::paint_dim(&t, "use /theme <name> to switch")
            );
            println!("{}", ui::theme::paint_rail_empty(&t));
            continue;
        }
        if let Some(tail) = trimmed.strip_prefix("/theme ") {
            let name = tail.trim();
            if ui::theme::set_active(name) {
                let active_theme = ui::theme::active();
                cfg.theme = active_theme.name.clone();
                let _ = save_config(cfg);
                let rail = ui::theme::paint_rail_empty(&t);
                let msg = ui::theme::paint_success_label(
                    &t,
                    &format!("theme \u{2192} {}", active_theme.name),
                );
                println!("{rail}");
                println!("{rail} {msg}");
                println!("{rail}");
            } else {
                let rail = ui::theme::paint_rail_empty(&t);
                let msg = ui::theme::paint_warning(&t, &format!("unknown theme '{}'", name));
                println!("{rail} {msg}");
                println!(
                    "{rail} {}",
                    ui::theme::paint_dim(
                        &t,
                        "available: GHOST, PHOSPHOR, MIST, EMBER, SAKURA, PAPER"
                    )
                );
                println!("{rail}");
            }
            continue;
        }

        if let Some(tail) = trimmed.strip_prefix("/model ") {
            let new_model = tail.trim().to_string();
            if new_model.is_empty() {
                println!(
                    "{} model: {}",
                    ui::theme::paint_rail_empty(&t),
                    client.model
                );
            } else {
                client.set_model(new_model.clone());
                cfg.model = new_model;
                let _ = save_config(cfg);
                let rail = ui::theme::paint_rail_empty(&t);
                let msg =
                    ui::theme::paint_success_label(&t, &format!("model \u{2192} {}", client.model));
                println!("{rail}");
                println!("{rail} {msg}");
                println!("{rail}");
            }
            continue;
        }

        if let Some(tail) = trimmed.strip_prefix("/provider ") {
            let new_provider = tail.trim().to_lowercase();
            if new_provider.is_empty() {
                let rail = ui::theme::paint_rail_empty(&t);
                let label = ui::theme::paint_bright(&t, "current provider:");
                let val = ui::theme::paint_dim(&t, client.kind.as_str());
                println!("{rail}");
                println!("{rail} {label} {val}");
                println!("{rail}");
                continue;
            }
            cfg.provider = new_provider;
            let _ = save_config(cfg);
            // Rebuild provider with new config
            let system_prompt = load_system_prompt(cfg.prompts_dir.as_deref());
            match build_provider(cfg, system_prompt) {
                Ok(new_client) => {
                    *client = new_client;
                    let rail = ui::theme::paint_rail_empty(&t);
                    let msg = ui::theme::paint_success_label(
                        &t,
                        &format!("provider \u{2192} {}", client.kind.as_str()),
                    );
                    println!("{rail}");
                    println!("{rail} {msg}");
                    let model_msg = ui::theme::paint_dim(&t, &format!("model: {}", client.model));
                    println!("{rail}  {model_msg}");
                    println!("{rail}");
                }
                Err(e) => {
                    let rail = ui::theme::paint_rail_empty(&t);
                    let msg = ui::theme::paint_error_label(
                        &t,
                        &format!("failed to switch provider: {}", e),
                    );
                    println!("{rail} {msg}");
                    println!("{rail}");
                }
            }
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
            cfg.mode = mode_label.to_string();
            let _ = save_config(cfg);
            let mode_key = ui::theme::accent_for_mode(mode_label);
            let hint = match session.mode {
                RunMode::Chat => "reply in plain text \u{2014} ask questions, chat",
                RunMode::Code => "generate code/files \u{2014} create, fix, build",
                RunMode::Plan => "explore & plan \u{2014} analyze, propose approach, no code",
            };
            let rail = ui::theme::paint_rail_empty(&t);
            let status = ui::theme::paint(
                &t,
                mode_key,
                &format!("switched to {mode_label} mode"),
                true,
            );
            let sub = ui::theme::paint_dim(&t, hint);
            println!("{rail}");
            println!("{rail} {status}");
            println!("{rail}  {sub}");
            println!("{rail}");
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/plan") {
            session.mode = RunMode::Plan;
            cfg.mode = "PLAN".to_string();
            let _ = save_config(cfg);
            let rail = ui::theme::paint_rail_empty(&t);
            let status = ui::theme::paint(&t, "accent_info", "switched to PLAN mode", true);
            let sub = ui::theme::paint_dim(
                &t,
                "explore & plan \u{2014} analyze, propose approach, no code",
            );
            println!("{rail}");
            println!("{rail} {status}");
            println!("{rail}  {sub}");
            println!("{rail}");
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/clear") {
            session.history.clear();
            session.last_search.clear();
            session.last_tokens = 0;
            let rail = ui::theme::paint_rail_empty(&t);
            let msg = ui::theme::paint_success_label(&t, "conversation cleared");
            println!("{rail}");
            println!("{rail} {msg}");
            println!("{rail}");
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
            let rail = ui::theme::paint_rail_empty(&t);
            let msg = ui::theme::paint_success_label(
                &t,
                "full reset \u{2014} history, code cache, and results cleared",
            );
            let sub = ui::theme::paint_dim(
                &t,
                "(memory preserved \u{2014} use /memory to clear project memory)",
            );
            println!("{rail}");
            println!("{rail} {msg}");
            println!("{rail}   {sub}");
            println!("{rail}");
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
                println!(
                    "{} usage: /copy [N] — N is a number",
                    ui::theme::paint_warning(&t, "│")
                );
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
                    ui::theme::paint_warning(&t, "│")
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
            let rail = ui::theme::paint_rail_empty(&t);
            let usage = ui::theme::paint_bright(&t, "usage: /find <query>");
            let detail = ui::theme::paint_dim(
                &t,
                "search text inside the project (skips node_modules, target, .git, ...)",
            );
            println!("{rail}");
            println!("{rail} {usage}");
            println!("{rail}  {detail}");
            println!("{rail}");
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
            let rail = ui::theme::paint_rail_empty(&t);
            let intent_label = ui::theme::paint_bright(&t, "last intent:");
            let intent_val = ui::theme::paint_success_label(&t, intent_name);
            let input_label = ui::theme::paint_bright(&t, "last input:");
            let input_val = ui::theme::paint_dim(&t, &format!("\"{}\"", session.last_user_input));
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
            let debug_intent =
                ui::theme::paint_dim(&t, &format!("  has_creation_intent={create_hit}"));
            let debug_fix =
                ui::theme::paint_dim(&t, &format!("  fix_window={fix_hit}  is_question={is_q}"));
            println!("{rail}");
            println!("{rail} {intent_label} {intent_val}");
            println!("{rail} {input_label} {input_val}");
            println!("{rail} {debug_intent}");
            println!("{rail} {debug_fix}");
            println!("{rail}");
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
            let rail = ui::theme::paint(&t, "accent", "\u{258C}", true);
            let msg = ui::theme::paint(&t, "accent_info", "generating code...", true);
            println!("{rail} {msg}");
        } else if session.mode == RunMode::Plan {
            let rail = ui::theme::paint(&t, "accent", "\u{258C}", true);
            let msg = ui::theme::paint(&t, "accent_info", "analyzing & planning...", true);
            println!("{rail} {msg}");
        } else if intent == TaskIntent::CodeAction {
            let rail = ui::theme::paint(&t, "accent", "\u{258C}", true);
            let msg = ui::theme::paint(&t, "accent", "Analyzing...", true);
            println!("{rail} {msg}");
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

        let label = ui::theme::paint(&t, "accent", "\u{258C}", true);
        let model_tag = ui::theme::paint(&t, "accent", &client.model, false);
        let mode_tag = ui::theme::paint_chip(&t, session.mode.label());
        let dot = ui::theme::paint_dim(&t, "\u{00B7}");
        println!("{label} {model_tag} {dot} {mode_tag}");

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
            let rail = ui::theme::paint_rail_empty(&t);
            let hint_label = ui::theme::paint_warning(&t, "hint:");
            let hint_msg = ui::theme::paint(
                &t,
                "accent",
                "this looks like a code request \u{2014} type /mode to switch to CODE",
                false,
            );
            println!("{rail}");
            println!("{rail}  {hint_label} {hint_msg}");
            println!("{rail}");
        }
        if session.mode == RunMode::Plan && intent == TaskIntent::CodeAction {
            let rail = ui::theme::paint_rail_empty(&t);
            let hint_label = ui::theme::paint_warning(&t, "hint:");
            let hint_msg = ui::theme::paint(
                &t,
                "accent_info",
                "in PLAN mode \u{2014} I'll analyze first, then you can switch to CODE",
                false,
            );
            println!("{rail}");
            println!("{rail}  {hint_label} {hint_msg}");
            println!("{rail}");
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
                        ui::theme::paint_dim(&t, "verbose:"),
                        text
                    );
                }

                let (was_validated, validated_text) =
                    validate_chat_response(&text, &intent, &session.mode);
                let cleaned = if was_validated && session.mode != RunMode::Code {
                    let warn = ui::theme::paint_warning(&t, "\u{258C}");
                    let note = ui::theme::paint_dim(
                        &t,
                        "(response contained unexpected code \u{2014} showing text only)",
                    );
                    println!("{warn} {note}");
                    validated_text
                } else {
                    text.trim().to_string()
                };

                session.last_tokens = (cleaned.len() / 4) as u32;

                let treat_as_code =
                    intent == TaskIntent::CodeAction || session.mode == RunMode::Code;

                let rail_chr = || ui::theme::paint(&t, "accent", "\u{258C}", true);

                if treat_as_code {
                    let code = extract_code_block(&cleaned);
                    let files = extract_code_blocks_with_names(&cleaned);

                    if !files.is_empty() {
                        session.last_files = files.clone();
                        session.last_code = if code.is_empty() { String::new() } else { code };
                        let gen_label = ui::theme::paint_success_label(&t, "generated:");
                        let gen_count =
                            ui::theme::paint_bright(&t, &format!("{} file(s)", files.len()));
                        println!("{}", rail_chr());
                        println!("{} {} {}", rail_chr(), gen_label, gen_count);
                        for f in &files {
                            let icon = file_icon(&f.path);
                            if f.path.is_empty() {
                                println!(
                                    "{}   {} unnamed ({} bytes)",
                                    rail_chr(),
                                    icon,
                                    f.content.len()
                                );
                            } else {
                                let path = ui::theme::paint_bright(&t, &f.path);
                                println!(
                                    "{}   {} {} ({} bytes)",
                                    rail_chr(),
                                    icon,
                                    path,
                                    f.content.len()
                                );
                            }
                        }
                        println!("{}", rail_chr());
                        auto_write_files(&mut session, &files);
                    } else if !code.is_empty() {
                        session.last_code = code;
                        session.last_files.clear();
                        let msg = ui::theme::paint_success_label(
                            &t,
                            "detected code block \u{2014} use /write <path> to save",
                        );
                        println!("{}", rail_chr());
                        println!("{} {}", rail_chr(), msg);
                        println!("{}", rail_chr());
                    } else {
                        for line in cleaned.lines() {
                            println!("{} {}", rail_chr(), line);
                        }
                    }
                } else if cleaned.is_empty() {
                    println!(
                        "{} {}",
                        ui::theme::paint_warning(&t, "\u{258C}"),
                        ui::theme::paint_dim(&t, "(empty response)")
                    );
                } else {
                    for line in cleaned.lines() {
                        println!("{} {}", rail_chr(), line);
                    }
                }

                let tps = if elapsed.as_secs_f64() > 0.0 {
                    session.last_tokens as f64 / elapsed.as_secs_f64()
                } else {
                    0.0
                };

                let rail = ui::theme::paint_rail_empty(&t);
                let provider_tag = ui::theme::paint_chip(&t, client.kind.as_str());
                let dur =
                    ui::theme::paint_dim(&t, &format!("\u{23f1} {:.1}s", elapsed.as_secs_f64()));
                let speed = ui::theme::paint_dim(&t, &format!("{:.0} tok/s", tps));
                let dot = ui::theme::paint_dim(&t, "\u{00B7}");
                println!("{rail}");
                println!("{rail} {provider_tag} {dot} {dur} {dot} {speed}");
                println!("{rail}");

                if !cleaned.is_empty() {
                    session.history.push((trimmed.to_string(), cleaned));
                    if session.history.len() > 12 {
                        session.history.remove(0);
                    }
                }
            }
            Err(e) => {
                let rail = ui::theme::paint_rail_empty(&t);
                let err_label = ui::theme::paint_error_label(&t, "\u{2717}");
                let err_msg = ui::theme::paint(&t, "error", &e.to_string(), false);
                let timer =
                    ui::theme::paint_dim(&t, &format!("\u{23f1} {:.1}s", elapsed.as_secs_f64()));
                println!("{rail}");
                println!("{rail} {err_label} {err_msg}");
                println!("{rail} {timer}");
                println!("{rail}");
            }
        }
    }
    session.feedback.flush();
    Ok(())
}

fn format_timestamp() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = dur.as_secs();

    let days = total_secs / 86400;
    let time_secs = total_secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    let mut y = 1970i64;
    let mut d = days as i64;
    loop {
        let year_days = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
            366
        } else {
            365
        };
        if d < year_days {
            break;
        }
        d -= year_days;
        y += 1;
    }
    let is_leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let month_days = [
        31u64,
        if is_leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1usize;
    let mut day = d as u64;
    for &md in &month_days {
        if day < md {
            break;
        }
        day -= md;
        month += 1;
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y,
        month,
        day + 1,
        hours,
        minutes,
        seconds
    )
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

fn file_icon(path: &str) -> String {
    let t = ui::theme::active();
    let emoji = file_icon_for(path);
    ui::theme::paint(&t, "text_muted", emoji, false)
}

fn file_icon_for(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".html") || lower.ends_with(".htm") {
        "\u{1F310}"
    } else if lower.ends_with(".css") {
        "\u{1F3A8}"
    } else if lower.ends_with(".js") || lower.ends_with(".mjs") || lower.ends_with(".ts") {
        "\u{26A1}"
    } else if lower.ends_with(".json") {
        "\u{1F4CB}"
    } else if lower.ends_with(".md") || lower.ends_with(".txt") {
        "\u{1F4C4}"
    } else if lower.ends_with(".py") {
        "\u{1F40D}"
    } else {
        "\u{1F4C4}"
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

pub(crate) fn truncate_bytes(s: &str, max: usize) -> String {
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

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::retrieve_relevant_chunks;

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
                .any(|p: &&str| p.contains("lib.rs") || p.contains("README")),
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
