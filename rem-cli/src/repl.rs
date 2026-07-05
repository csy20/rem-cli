//! Interactive REPL (read-eval-print loop) for chat mode.
//! The [`run_chat`] function handles user input, dispatches slash commands,
//! calls the LLM provider, and manages the conversational workflow.

use std::borrow::Cow;
use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::Result;

use crate::chat::{ChatSession, RunMode};
use crate::cli::AppConfig;
use crate::commands::{
    auto_write_files, handle_apply, handle_clear, handle_compact, handle_config, handle_config_set, handle_copy,
    handle_diff, handle_dir, handle_explain, handle_find, handle_goal, handle_init, handle_lint_with_fallback,
    handle_list_files, handle_memory, handle_memory_set, handle_mode, handle_model, handle_plan, handle_provider,
    handle_reasoning, handle_refactor, handle_reset, handle_resume_session, handle_review, handle_save_session,
    handle_search, handle_test, handle_theme, handle_tokens, handle_undo, handle_vision, handle_watch, handle_why,
    handle_write, print_chat_help, print_last_files, prompt_for_path,
};
use crate::config::first_run_setup;
use crate::constants::{CHAT_SYSTEM_PROMPT_CODE, CHAT_SYSTEM_PROMPT_CONVERSATIONAL, CHAT_SYSTEM_PROMPT_PLAN};
use crate::intent::{classify_intent, has_file_path, intent_instruction, TaskIntent};
use crate::pager::maybe_page;
use crate::parsing::extract_code_block;
use crate::provider::Provider;
use crate::session_io::{build_prompt, language_specific_guidance, print_welcome, validate_chat_response};
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
            '}' => {
                if opens.last() == Some(&'{') {
                    opens.pop();
                }
            }
            ')' => {
                if opens.last() == Some(&'(') {
                    opens.pop();
                }
            }
            ']' => {
                if opens.last() == Some(&'[') {
                    opens.pop();
                }
            }
            _ => {}
        }
    }
    !opens.is_empty() || in_backtick
}

/// Reads user input with multi-line support.
fn read_user_input(session: &mut ChatSession, prompt: &str, t: &crate::ui::theme::Theme) -> Option<String> {
    let mut error_count = 0u8;
    let mut lines: Vec<String> = Vec::new();

    loop {
        let current_prompt = if lines.is_empty() {
            prompt.to_string()
        } else {
            format!("\x01{}\x02...> \x01\x1b[0m\x02", ui::theme::paint_dim(t, "\u{2502}"))
        };

        let line = session.readline(&current_prompt);
        match line {
            Ok(s) => {
                let trimmed = s.trim_end().to_string();

                // Slash commands don't get multi-line treatment
                if lines.is_empty() && (trimmed.starts_with('/') || trimmed.is_empty()) {
                    return Some(trimmed);
                }

                lines.push(trimmed);

                // Check if we need more input
                let combined = lines.join("\n");
                if needs_continuation(&combined) {
                    continue;
                }

                return Some(combined);
            }
            Err(e) => {
                if e.kind() == io::ErrorKind::Interrupted || e.kind() == io::ErrorKind::UnexpectedEof {
                    let count = crate::CTRL_C_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    if count >= 2 || crate::SHOULD_EXIT.load(std::sync::atomic::Ordering::SeqCst) {
                        println!("  {} Ctrl+C pressed twice -- bye!", ui::theme::paint_dim(t, "!"));
                        session.feedback.flush();
                        session.save_history();
                        session.auto_save_session();
                        return None;
                    }
                    crate::provider::STREAM_CANCELLED.store(true, std::sync::atomic::Ordering::SeqCst);

                    if lines.is_empty() {
                        println!("  {} press Ctrl+C again to exit", ui::theme::paint_dim(t, "!"));
                        continue;
                    }
                    // Cancel multi-line input on first Ctrl+C during continuation
                    lines.clear();
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
        session.feedback.flush();
        session.save_history();
        session.auto_save_session();
        return true;
    }

    // Sync commands
    match name {
        "/help" | "help" => {
            print_chat_help();
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
        "/watch" => {
            handle_watch(session);
            return false;
        }
        "/resume" => {
            handle_resume_session(session);
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
        "/goal" => {
            handle_goal(client, session, args).await;
            false
        }
        "/vision" => {
            handle_vision(client, session, args).await;
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
        let mut p = instruction.to_string();
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
    let has_trigger = mod_triggers.iter().any(|t| words.contains(t));
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
    } else {
        display_text_output(&cleaned, t);
    }

    display_performance_stats(client, session, elapsed, t);

    if !cleaned.is_empty() {
        session.history_mgr.push_turn(trimmed.to_string(), cleaned);
        if session.history_mgr.history.len() > crate::constants::MAX_HISTORY_TURNS {
            session.history_mgr.history.remove(0);
        }
        session.history_mgr.messages_since_save += 1;
        if session.history_mgr.messages_since_save >= crate::constants::AUTO_SAVE_INTERVAL {
            session.auto_save_session();
            session.history_mgr.messages_since_save = 0;
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

    // Pre-warm the HTTP client so the first API call doesn't pay lazy-init cost
    let _ = crate::provider::HTTP_CLIENT.clone();

    loop {
        let prompt = build_prompt(&session, client);

        let line = match read_user_input(&mut session, &prompt, &t) {
            Some(l) => l,
            None => {
                session.feedback.flush();
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
                println!(
                    "  {} unknown command: {}",
                    ui::theme::paint_warning(&t, "!"),
                    ui::theme::paint_dim(&t, trimmed)
                );
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

        let system_prompt: Cow<'static, str> = if !lang_guidance.is_empty() {
            format!("{}{}", system_prompt, lang_guidance).into()
        } else {
            system_prompt.into()
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

        // Reset cancellation flag right before the LLM call
        crate::provider::STREAM_CANCELLED.store(false, std::sync::atomic::Ordering::SeqCst);

        // Frame-based spinner during LLM call (writes to stderr; tokens stream to stdout, no conflict)
        let spinner_frames_raw = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let spinner_frames: Vec<String> = spinner_frames_raw
            .iter()
            .map(|c| ui::theme::paint(&t, "accent_dim", c, true))
            .collect();
        let thinking_label = ui::theme::paint_dim(&t, "thinking...");
        let spinner_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sd = spinner_done.clone();
        let spinner_handle = tokio::spawn(async move {
            let mut i = 0usize;
            while !sd.load(std::sync::atomic::Ordering::SeqCst) {
                eprint!("\r  {} {}", spinner_frames[i], thinking_label);
                let _ = std::io::stderr().flush();
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                i = (i + 1) % spinner_frames.len();
            }
        });

        crate::provider::STREAM_TOKENS.store(true, std::sync::atomic::Ordering::SeqCst);
        let result = client
            .complete_chat_stream(&full_prompt, &system_prompt, &history_ctx)
            .await;
        crate::provider::STREAM_TOKENS.store(false, std::sync::atomic::Ordering::SeqCst);

        // Stop and clear the spinner
        spinner_done.store(true, std::sync::atomic::Ordering::SeqCst);
        spinner_handle.abort();
        eprint!("\r{}\r", " ".repeat(35));
        let _ = std::io::stderr().flush();
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
    session.feedback.flush();
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
            if f.path.is_empty() {
                println!("{}   {} unnamed ({} bytes)", rail_chr(), icon, f.content.len());
            } else {
                let path = ui::theme::paint_bright(t, &f.path);
                println!("{}   {} {} ({} bytes)", rail_chr(), icon, path, f.content.len());
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

/// Prints plain text output line by line, using pager for long output.
fn display_text_output(cleaned: &str, t: &crate::ui::theme::Theme) {
    let rail_chr = || ui::theme::paint(t, "accent", "\u{258C}", true);
    let line_count = cleaned.lines().count();
    if line_count > 50 {
        let mut buf = String::new();
        for line in cleaned.lines() {
            buf.push_str(&format!("{} {}\n", rail_chr(), line));
        }
        maybe_page(&buf);
        return;
    }
    for line in cleaned.lines() {
        println!("{} {}", rail_chr(), line);
    }
}

/// Prints provider, elapsed time, and tokens-per-second stats.
fn display_performance_stats(
    client: &Provider,
    session: &ChatSession,
    elapsed: std::time::Duration,
    t: &crate::ui::theme::Theme,
) {
    let tps = if elapsed.as_secs_f64() > 0.0 {
        session.last_tokens as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };
    let rail = ui::theme::paint_rail_empty(t);
    let provider_tag = ui::theme::paint_chip(t, client.kind.as_str());
    let dur = ui::theme::paint_dim(t, &format!("\u{23f1} {:.1}s", elapsed.as_secs_f64()));
    let speed = ui::theme::paint_dim(t, &format!("{:.0} tok/s", tps));
    let dot = ui::theme::paint_dim(t, "\u{00B7}");
    println!("{rail}");
    println!("{rail} {provider_tag} {dot} {dur} {dot} {speed}");
    println!("{rail}");
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
}
