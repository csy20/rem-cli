//! Interactive REPL (read-eval-print loop) for chat mode.
//! The [`run_chat`] function handles user input, dispatches slash commands,
//! calls the LLM provider, and manages the conversational workflow.

use std::io;
use std::path::PathBuf;

use anyhow::Result;

use crate::chat::{
    build_prompt, detect_project_type, language_specific_guidance, print_welcome,
    validate_chat_response, ChatSession, RunMode,
};
use crate::cli::AppConfig;
use crate::commands::{
    auto_write_files, handle_compact, handle_config, handle_config_set, handle_copy, handle_diff,
    handle_dir, handle_explain, handle_find, handle_goal, handle_init, handle_lint,
    handle_list_files, handle_memory, handle_memory_set, handle_refactor, handle_resume_session,
    handle_review, handle_save_session, handle_search, handle_test, handle_tokens, handle_undo,
    handle_write, print_chat_help, print_last_files, prompt_for_path,
};
use crate::config::{build_provider, first_run_setup, load_system_prompt, save_config};
use crate::intent::{
    classify_intent, has_creation_intent, has_file_path, intent_instruction, TaskIntent,
};
use crate::parsing::extract_code_block;
use crate::provider::Provider;
use crate::ui;
use crate::ui::output::SpinnerGuard;
use crate::{
    extract_code_blocks_with_names, exit_requested, file_icon, reset_ctrlc_count,
    CHAT_SYSTEM_PROMPT_CODE, CHAT_SYSTEM_PROMPT_CONVERSATIONAL, CHAT_SYSTEM_PROMPT_PLAN,
};

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
    Ok(session)
}

/// Main interactive REPL loop: reads user input, dispatches slash commands,
/// calls the LLM, and manages conversation history.
pub(crate) async fn run_chat(client: &mut Provider, cfg: &mut AppConfig, verbose: bool) -> Result<()> {
    reset_ctrlc_count();
    let mut session = initialize_session(client, cfg)?;
    let t = ui::theme::active();

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
            let system_prompt = load_system_prompt(cfg.prompts_dir.as_deref());
            match build_provider(cfg, system_prompt) {
                Ok(new_client) => {
                    cfg.provider = new_provider;
                    let _ = save_config(cfg);
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

                if treat_as_code {
                    display_code_files(&mut session, &cleaned, &t);
                } else if cleaned.is_empty() {
                    println!(
                        "{} {}",
                        ui::theme::paint_warning(&t, "\u{258C}"),
                        ui::theme::paint_dim(&t, "(empty response)")
                    );
                } else {
                    display_text_output(&cleaned, &t);
                }

                display_performance_stats(client, &session, elapsed, &t);

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

        if exit_requested() {
            break;
        }
    }
    session.feedback.flush();
    Ok(())
}

/// Displays code/files from an LLM response in the REPL.
fn display_code_files(session: &mut ChatSession, cleaned: &str, t: &crate::ui::theme::Theme) {
    let rail_chr = || ui::theme::paint(t, "accent", "\u{258C}", true);
    let code = extract_code_block(cleaned);
    let files = extract_code_blocks_with_names(cleaned);

    if !files.is_empty() {
        session.last_files = files.clone();
        session.last_code = if code.is_empty() { String::new() } else { code };
        let gen_label = ui::theme::paint_success_label(t, "generated:");
        let gen_count = ui::theme::paint_bright(t, &format!("{} file(s)", files.len()));
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
                let path = ui::theme::paint_bright(t, &f.path);
                println!("{}   {} {} ({} bytes)", rail_chr(), icon, path, f.content.len());
            }
        }
        println!("{}", rail_chr());
        auto_write_files(session, &files);
    } else if !code.is_empty() {
        session.last_code = code;
        session.last_files.clear();
        let msg = ui::theme::paint_success_label(
            t,
            "detected code block \u{2014} use /write <path> to save",
        );
        println!("{}", rail_chr());
        println!("{} {}", rail_chr(), msg);
        println!("{}", rail_chr());
    } else {
        display_text_output(cleaned, t);
    }
}

/// Prints plain text output line by line.
fn display_text_output(cleaned: &str, t: &crate::ui::theme::Theme) {
    let rail_chr = || ui::theme::paint(t, "accent", "\u{258C}", true);
    for line in cleaned.lines() {
        println!("{} {}", rail_chr(), line);
    }
}

/// Prints provider, elapsed time, and tokens-per-second stats.
fn display_performance_stats(client: &Provider, session: &ChatSession, elapsed: std::time::Duration, t: &crate::ui::theme::Theme) {
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
