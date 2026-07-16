//! Interactive REPL (read-eval-print loop) for chat mode.
//! The [`run_chat`] function handles user input, dispatches slash commands,
//! calls the LLM provider, and manages the conversational workflow.

use anyhow::Result;
use std::borrow::Cow;
use std::io;
use std::path::PathBuf;

use crate::chat::{ChatSession, RunMode};
use crate::cli::AppConfig;
use crate::commands::{
    auto_write_files, handle_apply, handle_clear, handle_commit, handle_compact, handle_compact_dry_run,
    handle_compact_undo, handle_config, handle_config_set, handle_context, handle_copy, handle_diff, handle_dir,
    handle_explain, handle_export_session, handle_export_session_md, handle_find, handle_goal, handle_import_session,
    handle_init, handle_lint_with_fallback, handle_list_files, handle_list_models, handle_list_sessions, handle_memory,
    handle_memory_set, handle_mode, handle_model, handle_page, handle_ping, handle_plan, handle_provider,
    handle_pull_model, handle_reasoning, handle_refactor, handle_reload, handle_reset, handle_resume_session,
    handle_review, handle_save_session, handle_search, handle_status, handle_summary, handle_test, handle_theme,
    handle_tokens, handle_undo, handle_vision, handle_why, handle_write, print_chat_help, print_command_help,
    print_last_files, prompt_for_path,
};
use crate::config::first_run_setup;
use crate::constants::{CHAT_SYSTEM_PROMPT_CODE, CHAT_SYSTEM_PROMPT_CONVERSATIONAL, CHAT_SYSTEM_PROMPT_PLAN};
use crate::highlight::{detect_language_from_content, highlight_code};
use crate::intent::{classify_intent, has_file_path, intent_instruction, TaskIntent};
use crate::pager::maybe_page;
use crate::parsing::extract_code_block;
use crate::provider::Provider;
use crate::session_io::{build_prompt, language_specific_guidance, validate_chat_response};
use crate::text_util::levenshtein_distance;
use crate::token_count::estimate_tokens;
use crate::types::{extract_code_blocks_with_names, file_icon};
use crate::ui;
use crate::{exit_requested, reset_ctrlc_count};

/// Initializes a chat session from configuration, setting up workspace,
/// theme, and mode.
fn initialize_session(client: &Provider, cfg: &mut AppConfig) -> Result<ChatSession> {
    let workspace = if let Some(ref dir) = cfg.workspace_dir {
        let path = PathBuf::from(dir);
        if !path.exists() {
            std::fs::create_dir_all(&path)?;
        }
        Some(path)
    } else {
        first_run_setup(cfg)?
    };

    ui::theme::set_active(&cfg.theme);
    let mut session = ChatSession::new(&client.ctx.model, workspace.clone())?;
    session
        .history_mgr
        .set_max_history_tokens_from_ctx(client.ctx.model_ctx);
    let saved_mode = cfg.mode.to_uppercase();
    session.mode = match saved_mode.as_str() {
        "CODE" => RunMode::Code,
        "PLAN" => RunMode::Plan,
        _ => RunMode::Chat,
    };
    let t = ui::theme::active();
    // Pass actual session mode to the welcome banner instead of hardcoded "CHAT"
    let mode_label = match session.mode {
        RunMode::Code => "CODE",
        RunMode::Plan => "PLAN",
        _ => "CHAT",
    };
    crate::session_io::print_welcome_with_mode(client, mode_label);
    if let Some(ref cfg_dir) = crate::config::config_dir() {
        let cfg_path = cfg_dir.join("config.toml");
        let exists = if cfg_path.exists() { "" } else { " (not yet created)" };
        ui::theme::println(&format!(
            "  {} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint(
                &t,
                "text_faint",
                &format!("config \u{2192} {}{}", cfg_path.display(), exists),
                false
            )
        ));
    }
    if let Some(ref wd) = workspace {
        ui::theme::println(&format!(
            "  {} {} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint(&t, "text_faint", "workspace \u{2192}", false),
            ui::theme::paint(&t, "accent_dim", &wd.display().to_string(), false)
        ));
    }
    Ok(session)
}

/// Returns true if the input has unbalanced brackets that need continuation.
/// Skips bracket counting inside string literals to avoid false positives
/// (e.g. `print("{")` should not trigger multi-line mode).
fn needs_continuation(line: &str) -> bool {
    let trimmed = line.trim_end();
    if trimmed.ends_with('\\') {
        return true;
    }
    let mut opens: Vec<char> = Vec::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_backtick = false;
    let mut escape = false;
    for c in trimmed.chars() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' && (in_single_quote || in_double_quote) {
            escape = true;
            continue;
        }
        if c == '\'' && !in_double_quote && !in_backtick {
            in_single_quote = !in_single_quote;
            continue;
        }
        if c == '"' && !in_single_quote && !in_backtick {
            in_double_quote = !in_double_quote;
            continue;
        }
        if c == '`' && !in_single_quote && !in_double_quote {
            in_backtick = !in_backtick;
            continue;
        }
        if in_single_quote || in_double_quote || in_backtick {
            continue;
        }
        match c {
            '{' | '(' | '[' => opens.push(c),
            '}' if opens.last() == Some(&'{') => {
                opens.pop();
            }
            ')' if opens.last() == Some(&'(') => {
                opens.pop();
            }
            ']' if opens.last() == Some(&'[') => {
                opens.pop();
            }
            _ => {}
        }
    }
    // Only check bracket balance — backtick state alone should NOT trigger
    // continuation (triple-backtick code fences would otherwise trap the
    // user in multi-line mode indefinitely).
    !opens.is_empty()
}

/// Reads user input with multi-line support.
/// Uses a local `interrupted_once` flag to track Ctrl+C presses within
/// this single call, avoiding races with the global handler on CTRL_C_COUNT.
fn read_user_input(session: &mut ChatSession, prompt: &str, t: &crate::ui::theme::Theme) -> Option<String> {
    let mut error_count = 0u8;
    let mut combined = String::new();
    let mut interrupted_once = false;

    loop {
        let current_prompt = if combined.is_empty() {
            prompt.to_string()
        } else {
            let line_count = combined.matches('\n').count() + 1;
            format!(
                "\x01{}\x02│...{}> \x01\x1b[0m\x02",
                ui::theme::paint_dim(t, "\u{2502}"),
                ui::theme::paint_dim(t, &format!("{}", line_count)),
            )
        };

        let line = session.readline(&current_prompt);
        match line {
            Ok(s) => {
                let trimmed = s.trim_end().to_string();

                // Slash commands don't get multi-line treatment
                if combined.is_empty() && (trimmed.starts_with('/') || trimmed.is_empty()) {
                    // /edit opens external editor and returns content as input
                    if trimmed == "/edit" {
                        if let Some(content) = crate::commands::handle_edit() {
                            session.add_history(&content);
                            return Some(content);
                        }
                        continue;
                    }
                    return Some(trimmed);
                }

                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str(&trimmed);

                // Check if we need more input
                if needs_continuation(&combined) {
                    continue;
                }

                // Save multi-line assembled input to readline history
                session.add_history(&combined);
                return Some(combined);
            }
            Err(e) => {
                if e.kind() == io::ErrorKind::Interrupted || e.kind() == io::ErrorKind::UnexpectedEof {
                    crate::provider::STREAM_CANCELLED.store(true, std::sync::atomic::Ordering::SeqCst);

                    if interrupted_once || crate::SHOULD_EXIT.load(std::sync::atomic::Ordering::SeqCst) {
                        println!("  {} Ctrl+C pressed twice -- bye!", ui::theme::paint_dim(t, "!"));
                        session.save_history();
                        session.auto_save_session();
                        return None;
                    }
                    interrupted_once = true;

                    if combined.is_empty() {
                        println!("  {} press Ctrl+C again to exit", ui::theme::paint_dim(t, "!"));
                        continue;
                    }
                    // Cancel multi-line input on first Ctrl+C during continuation
                    combined.clear();
                    println!("  {} input cancelled", ui::theme::paint_dim(t, "!"));
                    continue;
                }
                eprintln!("  {} input error: {}", ui::theme::paint_error_label(t, "err:"), e);
                error_count += 1;
                if error_count >= 3 {
                    eprintln!("  {} too many errors, exiting", ui::theme::paint_error_label(t, "err:"));
                    return None;
                }
                continue;
            }
        }
    }
}

/// Dispatches a recognized slash command to the appropriate handler.
/// Returns `true` if the program should exit, `false` to continue the loop.
/// For non-command input (or unrecognized commands), returns `false` and
/// the caller should proceed with the LLM call.
async fn dispatch_slash_command(
    trimmed: &str,
    session: &mut ChatSession,
    client: &mut Provider,
    cfg: &mut AppConfig,
    t: &crate::ui::theme::Theme,
) -> bool {
    let reg = crate::commands::registry();
    let (name, args) = reg.parse(trimmed);

    if !reg.is_command(trimmed) {
        return false; // not a command, continue to LLM
    }

    // Handle exit/quit specially
    if name == "exit" || name == "quit" {
        println!("  {}", ui::theme::paint_dim(t, "bye!"));
        session.save_history();
        session.auto_save_session();
        return true;
    }

    // Sync commands
    match name {
        "/help" | "help" => {
            if args.is_empty() {
                print_chat_help();
            } else {
                print_command_help(args);
            }
            return false;
        }
        "/theme" => {
            handle_theme(cfg, if args.is_empty() { None } else { Some(args) });
            return false;
        }
        "/model" => {
            handle_model(client, cfg, if args.is_empty() { None } else { Some(args) });
            return false;
        }
        "/provider" => {
            handle_provider(client, cfg, if args.is_empty() { None } else { Some(args) });
            return false;
        }
        "/write" => {
            handle_write(session, args);
            return false;
        }
        "/dir" => {
            handle_dir(session, args);
            return false;
        }
        "/code" => {
            print_last_files(session);
            return false;
        }
        "/undo" => {
            let n: usize = match args.parse() {
                Ok(n) => n,
                Err(_) => {
                    tracing::warn!("invalid /undo argument '{}', defaulting to 1", args);
                    1
                }
            };
            handle_undo(session, n);
            return false;
        }
        "/files" => {
            handle_list_files(session);
            return false;
        }
        "/mode" => {
            handle_mode(session, cfg);
            return false;
        }
        "/plan" => {
            handle_plan(session, cfg);
            return false;
        }
        "/clear" => {
            handle_clear(session);
            return false;
        }
        "/config" => {
            if args.is_empty() {
                handle_config(session, client);
            } else {
                handle_config_set(session, client, args);
            }
            return false;
        }
        "/diff" => {
            handle_diff(session);
            return false;
        }
        "/apply" => {
            handle_apply(session);
            return false;
        }
        "/tokens" => {
            handle_tokens(session, client);
            return false;
        }
        "/context" => {
            handle_context(session, client);
            return false;
        }
        "/memory" => {
            if args.is_empty() {
                handle_memory(session);
            } else {
                handle_memory_set(session, args);
            }
            return false;
        }
        "/init" => {
            handle_init(session);
            return false;
        }
        "/reset" => {
            handle_reset(session);
            return false;
        }
        "/why" => {
            handle_why(session);
            return false;
        }
        "/reasoning" => {
            handle_reasoning(client, cfg, if args.is_empty() { None } else { Some(args) });
            return false;
        }
        "/resume" => {
            handle_resume_session(session);
            return false;
        }
        "/session" => {
            let sub = args.trim();
            if sub == "compact-undo" {
                handle_compact_undo(session);
            } else if sub == "list" {
                handle_list_sessions(session);
            } else if let Some(export_path) = sub.strip_prefix("export-md ") {
                handle_export_session_md(session, export_path.trim());
            } else if let Some(export_path) = sub.strip_prefix("export ") {
                handle_export_session(session, export_path.trim());
            } else if let Some(import_path) = sub.strip_prefix("import ") {
                handle_import_session(session, import_path.trim());
            } else {
                let t = ui::theme::active();
                println!(
                    "{} usage: /session compact-undo | list | export <path> | export-md <path> | import <path>",
                    ui::theme::paint_warning(&t, "\u{258C}")
                );
            }
            return false;
        }
        "/copy" => {
            let n: usize = match args.parse() {
                Ok(n) => n,
                Err(_) => {
                    tracing::warn!("invalid /copy argument '{}', defaulting to 1", args);
                    1
                }
            };
            handle_copy(session, n);
            return false;
        }
        "/find" => {
            handle_find(session, args);
            return false;
        }
        "/save" => {
            if args.is_empty() {
                handle_save_session(session);
            } else {
                handle_write(session, args);
            }
            return false;
        }
        "/page" => {
            handle_page();
            return false;
        }
        _ => {}
    }

    // Async commands (require .await)
    match name {
        "/search" => {
            handle_search(client, session, cfg, args).await;
            false
        }
        "/explain" => {
            handle_explain(client, session, args).await;
            false
        }
        "/test" => {
            handle_test(client, session, args).await;
            false
        }
        "/refactor" => {
            handle_refactor(client, session, args).await;
            false
        }
        "/lint" => {
            handle_lint_with_fallback(session, args).await;
            false
        }
        "/review" => {
            handle_review(client, session).await;
            false
        }
        "/compact" => {
            handle_compact(client, session).await;
            false
        }
        "/compact-dry-run" => {
            handle_compact_dry_run(session);
            false
        }
        "/goal" => {
            handle_goal(client, session, args).await;
            false
        }
        "/reload" => {
            handle_reload(session, cfg);
            false
        }
        "/vision" => {
            handle_vision(client, session, args).await;
            false
        }
        "/ping" => {
            handle_ping(client).await;
            false
        }
        "/models" => {
            handle_list_models(client).await;
            false
        }
        "/pull" => {
            handle_pull_model(client, args).await;
            false
        }
        "/status" => {
            handle_status(session, client).await;
            false
        }
        "/commit" => {
            handle_commit(session, args).await;
            false
        }
        "/summary" => {
            handle_summary(client, session, if args.is_empty() { None } else { Some(args) }).await;
            false
        }
        _ => false,
    }
}

/// Builds the full LLM prompt from all context sources.
fn build_llm_prompt(session: &mut ChatSession, trimmed: &str, intent: &TaskIntent) -> (String, String) {
    let instruction = intent_instruction(intent);

    let needs_path = (session.mode == RunMode::Code || *intent == TaskIntent::CodeAction) && !has_file_path(trimmed);
    let final_prompt = if needs_path {
        let path = prompt_for_path(session).unwrap_or_else(|_| trimmed.to_string());
        format!("User request: {}\n\nSave file at: {}", trimmed, path)
    } else if let Some(ref dir) = session.ctx.project_dir {
        format!("User request: {}\n\nWorking directory: {}", trimmed, dir.display())
    } else {
        format!("User request: {}", trimmed)
    };

    let search_ctx = session.build_search_context();
    let history_ctx = session.build_chat_history();
    let memory_ctx = session.build_memory_context();
    let (resolved_input, at_context) = session.resolve_at_references(&final_prompt);
    let project_ctx = session.build_relevant_project_context(&resolved_input);

    let last_code_ctx = build_last_code_context(session, trimmed);

    let full_prompt = {
        let estimated = instruction.len()
            + memory_ctx.len()
            + project_ctx.len()
            + last_code_ctx.len()
            + at_context.len()
            + resolved_input.len()
            + search_ctx.len()
            + 64; // extra for newlines and separators
        let mut p = String::with_capacity(estimated);
        p.push_str(instruction);
        p.push('\n');
        if !memory_ctx.is_empty() {
            p.push_str(&memory_ctx);
        }
        if !project_ctx.is_empty() {
            p.push_str(&project_ctx);
        }
        if !last_code_ctx.is_empty() {
            p.push_str(&last_code_ctx);
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

    (full_prompt, history_ctx)
}

/// Builds the "last generated code/files" context for follow-up requests.
fn build_last_code_context(session: &ChatSession, trimmed: &str) -> String {
    if session.code_out.last_code.is_empty() && session.code_out.last_files.is_empty() {
        return String::new();
    }
    let mod_triggers = [
        "add",
        "update",
        "change",
        "modify",
        "edit",
        "append",
        "improve",
        "enhance",
        "refactor",
        "rewrite",
        "transform",
        "convert",
        "extend",
        "expand",
    ];
    let lower_in = trimmed.to_lowercase();
    let words: Vec<&str> = lower_in.split_whitespace().collect();
    let has_trigger = mod_triggers.iter().any(|t| {
        words
            .iter()
            .any(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == *t)
    });
    let has_qualifier = words.contains(&"also") || words.contains(&"more") || lower_in.contains("and then");
    if !has_trigger && !has_qualifier {
        return String::new();
    }

    let mut ctx = String::new();
    if !session.code_out.last_code.is_empty() {
        let truncated = crate::truncate_bytes(&session.code_out.last_code, 6000);
        ctx = format!("\n[Last generated code (for reference)]:\n```\n{}\n```\n", truncated);
    }
    if !session.code_out.last_files.is_empty() {
        let mut files_ctx = String::from("\n[Last generated files]:\n");
        for f in &session.code_out.last_files {
            if !f.path.is_empty() {
                let truncated = crate::truncate_bytes(&f.content, 3000);
                files_ctx.push_str(&format!("\n### {}\n```\n{}\n```\n", f.path, truncated));
            }
        }
        ctx.push_str(&files_ctx);
    }
    ctx
}

/// Processes the LLM response: validates, displays, and updates session state.
#[allow(clippy::too_many_arguments)]
fn handle_llm_response(
    session: &mut ChatSession,
    trimmed: &str,
    text: String,
    intent: &TaskIntent,
    elapsed: std::time::Duration,
    client: &Provider,
    verbose: bool,
    t: &crate::ui::theme::Theme,
) {
    if verbose {
        eprintln!("\n  {} raw response:\n{}\n", ui::theme::paint_dim(t, "verbose:"), text);
    }

    let (was_validated, validated_text) = validate_chat_response(&text, intent, &session.mode);
    let cleaned = if was_validated && session.mode != RunMode::Code {
        let warn = ui::theme::paint_warning(t, "\u{258C}");
        let note = ui::theme::paint_dim(t, "(response contained unexpected code \u{2014} showing text only)");
        println!("{warn} {note}");
        validated_text
    } else {
        text.trim().to_string()
    };

    // Prefer real token counts from providers that track usage (e.g. Anthropic)
    let usage = client.anthropic_usage();
    session.last_tokens = if usage.output_tokens > 0 {
        usage.output_tokens
    } else {
        estimate_tokens(&cleaned) as u32
    };
    session.total_tokens += session.last_tokens as u64;

    let treat_as_code = *intent == TaskIntent::CodeAction || session.mode == RunMode::Code;
    if treat_as_code {
        session.code_out.last_trigger_input = trimmed.to_string();
        display_code_files(session, &cleaned, t);
    } else if cleaned.is_empty() {
        println!(
            "{} {}",
            ui::theme::paint_warning(t, "\u{258C}"),
            ui::theme::paint_dim(t, "(empty response)")
        );
    } else if crate::provider::HAD_STREAMING_OUTPUT.load(std::sync::atomic::Ordering::SeqCst) {
        // Content was already streamed to stdout line by line — skip re-display
    } else {
        display_text_output(&cleaned, t);
    }

    display_performance_stats(client, session, elapsed, t);

    if !cleaned.is_empty() {
        session.history_mgr.push_turn(trimmed.to_string(), cleaned);
        if session.history_mgr.messages_since_save >= crate::constants::AUTO_SAVE_INTERVAL {
            session.auto_save_session();
            session.history_mgr.messages_since_save = 0;
            eprint!("{}", ui::theme::paint_dim(t, "\u{258C} \u{00B7}\r"));
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
    }
}

/// Main interactive REPL loop: reads user input, dispatches slash commands,
/// calls the LLM, and manages conversation history.
pub(crate) async fn run_chat(client: &mut Provider, cfg: &mut AppConfig, verbose: bool) -> Result<()> {
    reset_ctrlc_count();
    crate::REPL_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut session = initialize_session(client, cfg)?;
    let t = ui::theme::active();

    // Auto-restore previous session if enabled and session file exists
    if cfg.auto_resume {
        let dir = session
            .ctx
            .project_dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let session_file = dir.join(".rem/session.json.gz");
        if session_file.exists() {
            handle_resume_session(&mut session);
        }
    }

    // Pre-warm the HTTP client so the first API call doesn't pay lazy-init cost
    let _ = crate::provider::HTTP_CLIENT.clone();

    loop {
        let prompt = build_prompt(&session, client);

        let line = match read_user_input(&mut session, &prompt, &t) {
            Some(l) => l,
            None => {
                crate::REPL_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
                return Ok(());
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed == session.last_user_input && !trimmed.starts_with('/') {
            println!(
                "  {} duplicate input ignored (same as last message)",
                ui::theme::paint_dim(&t, "!")
            );
            continue;
        }

        let handled = dispatch_slash_command(trimmed, &mut session, client, cfg, &t).await;
        if handled {
            // "exit" or "quit" triggered a break
            break;
        }
        // Skip LLM call for slash commands
        if trimmed.starts_with('/') {
            // Check if it was actually a recognized command (not an unknown one)
            if !crate::commands::registry().is_command(trimmed) {
                let warning = ui::theme::paint_warning(&t, "!");
                let unknown = ui::theme::paint_dim(&t, trimmed);
                println!("  {warning} unknown command: {unknown}");
                // Suggest closest match
                if let Some(suggestion) = did_you_mean(trimmed) {
                    let hint = ui::theme::paint(&t, "accent", &suggestion, false);
                    let msg = ui::theme::paint_dim(&t, "did you mean?");
                    println!("  {}   {msg} {hint}", ui::theme::paint_rail_empty(&t));
                }
            }
            continue;
        }

        let intent = classify_intent(trimmed);
        session.last_intent = intent.clone();
        session.last_user_input = trimmed.to_string();
        session.code_out.last_trigger_input.clear();

        let (full_prompt, history_ctx) = build_llm_prompt(&mut session, trimmed, &intent);

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

        let label = ui::theme::paint(&t, "accent", "\u{258C}", true);
        let model_tag = ui::theme::paint(&t, "accent", &client.ctx.model, false);
        let mode_tag = ui::theme::paint_chip(&t, session.mode.label());
        let dot = ui::theme::paint_dim(&t, "\u{00B7}");
        println!("{label} {model_tag} {dot} {mode_tag}");

        let start = std::time::Instant::now();
        let system_prompt = match session.mode {
            RunMode::Chat => CHAT_SYSTEM_PROMPT_CONVERSATIONAL,
            RunMode::Code => CHAT_SYSTEM_PROMPT_CODE,
            RunMode::Plan => CHAT_SYSTEM_PROMPT_PLAN,
        };

        let lang_guidance = {
            let ptype = session.get_project_type();
            if !ptype.is_empty() {
                language_specific_guidance(ptype)
            } else {
                ""
            }
        };

        let system_prompt: Cow<'static, str> = if lang_guidance.is_empty() {
            system_prompt.into()
        } else {
            format!("{}{}", system_prompt, lang_guidance).into()
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

        // Reset cancellation and streaming tracking flags right before the LLM call
        crate::provider::STREAM_CANCELLED.store(false, std::sync::atomic::Ordering::SeqCst);
        crate::provider::HAD_STREAMING_OUTPUT.store(false, std::sync::atomic::Ordering::SeqCst);

        // Spinner during LLM call (writes to stderr; tokens stream to stdout, no conflict)
        let _spinner = crate::ui::output::SpinnerGuard::new("thinking...");

        crate::provider::STREAM_TOKENS.store(true, std::sync::atomic::Ordering::SeqCst);
        let result = client
            .complete_chat_stream(&full_prompt, &system_prompt, &history_ctx)
            .await;
        crate::provider::STREAM_TOKENS.store(false, std::sync::atomic::Ordering::SeqCst);

        // Spinner stops automatically when _spinner is dropped
        let elapsed = start.elapsed();
        session.last_elapsed = elapsed;

        match result {
            Ok(text) => {
                handle_llm_response(&mut session, trimmed, text, &intent, elapsed, client, verbose, &t);
            }
            Err(e) => {
                let rail = ui::theme::paint_rail_empty(&t);
                let err_label = ui::theme::paint_error_label(&t, "\u{2717}");
                let err_msg = ui::theme::paint(&t, "error", &e.to_string(), false);
                let timer = ui::theme::paint_dim(&t, &format!("\u{23f1} {:.1}s", elapsed.as_secs_f64()));
                println!("{rail}");
                println!("{rail} {err_label} {err_msg}");
                println!("{rail} {timer}");
                println!("{rail}");
            }
        }

        if exit_requested() {
            session.save_history();
            session.auto_save_session();
            break;
        }
    }
    crate::REPL_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

/// Displays code/files from an LLM response in the REPL.
fn display_code_files(session: &mut ChatSession, cleaned: &str, t: &crate::ui::theme::Theme) {
    let rail_chr = || ui::theme::paint(t, "accent", "\u{258C}", true);
    let code = extract_code_block(cleaned);
    let files = extract_code_blocks_with_names(cleaned);

    if !files.is_empty() {
        session.code_out.last_files = files.clone();
        session.code_out.last_code = if code.is_empty() { String::new() } else { code };
        let gen_label = ui::theme::paint_success_label(t, "generated:");
        let gen_count = ui::theme::paint_bright(t, &format!("{} file(s)", files.len()));
        println!("{}", rail_chr());
        println!("{} {} {}", rail_chr(), gen_label, gen_count);
        for f in &files {
            let icon = file_icon(&f.path);
            let line_count = f.content.lines().count();
            let lang = if !f.path.is_empty() {
                std::path::Path::new(&f.path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
            } else {
                ""
            };
            let meta = if !lang.is_empty() {
                format!("{} lines, {} bytes, .{}", line_count, f.content.len(), lang)
            } else {
                format!("{} lines, {} bytes", line_count, f.content.len())
            };
            if f.path.is_empty() {
                println!("{}   {} unnamed ({})", rail_chr(), icon, meta);
            } else {
                let path = ui::theme::paint_bright(t, &f.path);
                println!("{}   {} {} ({})", rail_chr(), icon, path, meta);
            }
        }
        println!("{}", rail_chr());
        auto_write_files(session, &files);
    } else if !code.is_empty() {
        session.code_out.last_code = code;
        session.code_out.last_files.clear();
        let msg = ui::theme::paint_success_label(t, "detected code block \u{2014} use /write <path> to save");
        println!("{}", rail_chr());
        println!("{} {}", rail_chr(), msg);
        println!("{}", rail_chr());
    } else {
        display_text_output(cleaned, t);
    }
}

/// Returns the visible width of text, excluding ANSI escape codes.
fn visible_width(text: &str) -> usize {
    let mut width = 0;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            chars.find(|&n| n == 'm');
        } else {
            width += 1;
        }
    }
    width
}

/// Wraps text to a given max width at word boundaries, aware of ANSI escape codes.
/// Strips ANSI sequences when measuring width but preserves them in output.
fn word_wrap(text: &str, max_width: usize) -> String {
    let mut result = String::with_capacity(text.len());
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            result.push('\n');
        }
        if visible_width(line) <= max_width {
            result.push_str(line);
        } else {
            let mut remaining = line;
            while visible_width(remaining) > max_width {
                // Find split point at max_width visible characters
                let mut vis = 0;
                let mut split_pos = 0;
                let mut chars = remaining.char_indices();
                while let Some((idx, c)) = chars.next() {
                    if c == '\x1b' {
                        chars.find(|&(_, n)| n == 'm');
                        continue;
                    }
                    vis += 1;
                    if vis > max_width {
                        break;
                    }
                    split_pos = idx + c.len_utf8();
                }
                // Try to break at a space before split_pos
                let split_at = remaining[..split_pos]
                    .rfind(' ')
                    .filter(|&pos| pos > 0)
                    .unwrap_or(split_pos);
                result.push_str(&remaining[..split_at]);
                result.push('\n');
                remaining = remaining[split_at..].trim_start();
            }
            if !remaining.is_empty() {
                result.push_str(remaining);
            }
        }
    }
    result
}

/// Cached fallback terminal width from `tput cols`.
static FALLBACK_WIDTH: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

fn detect_fallback_width() -> usize {
    if let Ok(output) = std::process::Command::new("sh")
        .args(["-c", "tput cols 2>/dev/null || echo 80"])
        .output()
    {
        if let Ok(cols) = String::from_utf8_lossy(&output.stdout).trim().parse::<usize>() {
            if cols > 0 {
                return cols;
            }
        }
    }
    80usize
}

fn terminal_width() -> usize {
    // Check COLUMNS env var first (bash updates this on SIGWINCH)
    if let Ok(cols) = std::env::var("COLUMNS") {
        if let Ok(w) = cols.parse::<usize>() {
            return w.saturating_sub(4);
        }
    }
    let fallback = *FALLBACK_WIDTH.get_or_init(detect_fallback_width);
    fallback.saturating_sub(4)
}

/// Prints plain text output line by line, using pager for long output.
/// Auto-detects code blocks and applies syntax highlighting.
fn display_text_output(cleaned: &str, t: &crate::ui::theme::Theme) {
    let rail_chr = || ui::theme::paint(t, "accent", "\u{258C}", true);
    let max_width = terminal_width();
    let processed = crate::ui::markdown::render_markdown(cleaned, t);
    let highlighted = if processed.contains("```") && processed.lines().count() > 3 {
        let mut result = String::new();
        let mut in_code = false;
        let mut lang = "";
        for line in processed.lines() {
            if let Some(stripped) = line.trim().strip_prefix("```") {
                if in_code {
                    in_code = false;
                } else {
                    in_code = true;
                    lang = stripped.trim();
                }
                continue;
            }
            if in_code && !lang.is_empty() {
                result.push_str(&highlight_code(line, lang));
                result.push('\n');
            } else {
                result.push_str(line);
                result.push('\n');
            }
        }
        result
    } else if processed.lines().count() > 2 {
        let lang_hint = detect_language_from_content(&processed);
        if !lang_hint.is_empty() {
            highlight_code(&processed, lang_hint)
        } else {
            processed.to_string()
        }
    } else {
        processed.to_string()
    };
    let wrapped = word_wrap(&highlighted, max_width);
    let line_count = wrapped.lines().count();
    let mut buf = String::new();
    for line in wrapped.lines() {
        buf.push_str(&format!("{} {}\n", rail_chr(), line));
    }
    crate::pager::store_output(&buf);
    if line_count > 50 {
        maybe_page(&buf);
        return;
    }
    print!("{}", buf);
}

/// Prints model, elapsed time, tokens-per-second, session duration, and total tokens.
fn display_performance_stats(
    client: &Provider,
    session: &ChatSession,
    elapsed: std::time::Duration,
    t: &crate::ui::theme::Theme,
) {
    let tps = if elapsed.as_secs_f64() > 0.0 && session.last_tokens > 0 {
        session.last_tokens as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };
    let session_dur = session.session_start.elapsed();
    let rail = ui::theme::paint_rail_empty(t);
    let model_tag = ui::theme::paint_dim(t, &client.ctx.model);
    let dur = ui::theme::paint_dim(t, &format!("\u{23f1} {:.1}s", elapsed.as_secs_f64()));
    let speed = if session.last_tokens > 0 {
        ui::theme::paint_dim(t, &format!("{:.0} tok/s", tps))
    } else {
        ui::theme::paint_dim(t, "? tok/s")
    };
    let total = ui::theme::paint_dim(t, &format!("\u{2211}{} tok", session.total_tokens));
    let sess = ui::theme::paint_dim(t, &format!("\u{29d6}{:.0}m", session_dur.as_secs_f64() / 60.0));
    let dot = ui::theme::paint_dim(t, "\u{00B7}");
    println!("{rail}");
    println!("{rail} {model_tag} {dot} {dur} {dot} {speed} {dot} {total} {dot} {sess}");
    println!("{rail}");
}

/// Simple Levenshtein distance for "did you mean?" suggestions.
/// Suggests a close command name when an unknown `/command` is entered.
fn did_you_mean(input: &str) -> Option<String> {
    let cmd_name = input.split(' ').next().unwrap_or(input);
    let reg = crate::commands::registry();
    let names = reg.command_names();
    let mut best_name: Option<String> = None;
    let mut best_dist = usize::MAX;
    for name in names {
        let dist = levenshtein_distance(cmd_name, name);
        if dist > 0 && dist < best_dist {
            best_dist = dist;
            best_name = Some(name.to_string());
        }
    }
    if best_dist <= 2 {
        best_name
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_continuation_trailing_backslash() {
        assert!(needs_continuation("hello \\"));
    }

    #[test]
    fn needs_continuation_unclosed_brace() {
        assert!(needs_continuation("fn foo() {"));
    }

    #[test]
    fn needs_continuation_unclosed_paren() {
        assert!(needs_continuation("if (x > 0"));
    }

    #[test]
    fn needs_continuation_unclosed_bracket() {
        assert!(needs_continuation("let xs = [1, 2, 3"));
    }

    #[test]
    fn needs_continuation_closed_brackets_returns_false() {
        assert!(!needs_continuation("fn foo() { return 1; }"));
        assert!(!needs_continuation("if (x > 0) { }"));
        assert!(!needs_continuation("let xs = [1, 2, 3];"));
    }

    #[test]
    fn needs_continuation_empty_line() {
        assert!(!needs_continuation(""));
    }

    #[test]
    fn needs_continuation_mismatched_close_does_not_count() {
        // A close without open should not trigger continuation
        assert!(!needs_continuation("some ) text"));
    }

    #[test]
    fn needs_continuation_plain_text() {
        assert!(!needs_continuation("hello world"));
    }

    #[test]
    fn needs_continuation_nested_brackets() {
        assert!(needs_continuation("fn outer() { fn inner(x: &[u8"));
        assert!(!needs_continuation("fn outer() { fn inner(x: &[u8]); }"));
    }

    #[test]
    fn word_wrap_ansi_escape_codes_preserved() {
        let red = "\x1b[31m";
        let reset = "\x1b[0m";
        let input = format!("{}bold red text{} normal", red, reset);
        let wrapped = word_wrap(&input, 80);
        // ANSI codes should be preserved in output
        assert!(wrapped.contains(red), "ANSI red code should be preserved");
        assert!(wrapped.contains(reset), "ANSI reset code should be preserved");
        assert!(wrapped.contains("bold red text"), "text content preserved");
    }

    #[test]
    fn word_wrap_ansi_with_emoji() {
        let green = "\x1b[32m";
        let reset = "\x1b[0m";
        let input = format!("{}✅ complete{} more text", green, reset);
        let wrapped = word_wrap(&input, 40);
        assert!(wrapped.contains("✅"), "emoji preserved");
        assert!(wrapped.contains(green), "ANSI preserved");
    }

    #[test]
    fn word_wrap_ansi_long_line_preserves_codes() {
        let bold = "\x1b[1m";
        let reset = "\x1b[0m";
        let long = format!("{}hello world this is a very long line that should wrap at some point because it exceeds the max width threshold{}", bold, reset);
        let wrapped = word_wrap(&long, 30);
        // The ANSI codes should appear in the output somewhere
        assert!(wrapped.contains(bold));
        assert!(wrapped.contains(reset));
        // Each line should have content
        for line in wrapped.lines() {
            assert!(!line.is_empty(), "no empty lines in wrapped output");
        }
    }
}
