//! Goal-driven autonomous loop (`/goal`).
//! Iteratively generates code, runs lint/tests, and feeds results back to
//! the LLM until the goal is achieved or max iterations are reached.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use crate::agentic::{
    build_agentic_prompt, build_tool_context, extract_goal_signal, format_tool_output, run_lint, run_test, ToolOutput,
};
use crate::chat::ChatSession;
use crate::constants::CHAT_SYSTEM_PROMPT_CODE;
use crate::parsing::extract_code_block;
use crate::provider::Provider;
use crate::tool_executor;
use crate::types::extract_code_blocks_with_names;
use crate::ui;

const MAX_TOOL_OUTPUT_LEN: usize = crate::constants::GOAL_TOOL_OUTPUT_MAX_CHARS;
const ITERATION_TIMEOUT: Duration = crate::constants::GOAL_ITERATION_TIMEOUT;

fn circuit_breaker_hash(output: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    output.hash(&mut hasher);
    hasher.finish()
}

pub(crate) async fn handle_goal(client: &Provider, session: &mut ChatSession, condition: &str) {
    let t = ui::theme::active();
    println!("{}", ui::theme::paint_rail_empty(&t));
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, &format!("GOAL: {}", condition)),
    );
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_dim(&t, "REM will work until goal is met. Ctrl+C to stop."),
    );
    println!("{}", ui::theme::paint_rail_empty(&t));

    // Auto-detect: use native tool calling if provider supports it
    if client.supports_tools() {
        println!(
            "{} {} using native tool calling",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_dim(&t, "\u{26A1}")
        );
        let result = tool_executor::run_tool_loop(
            client,
            condition,
            "[MODE: CODE] Use tools to achieve the goal. When done, explain what was accomplished.",
            "",
        )
        .await;
        match result {
            Ok(text) => {
                println!("\n{}", text);
                println!("{}", ui::theme::paint_rail_empty(&t));
                println!(
                    "{} {} goal completed",
                    ui::theme::paint_success_label(&t, "\u{258C}"),
                    ui::theme::paint_success_label(&t, "\u{2713}")
                );

                let files = extract_code_blocks_with_names(&text);
                if !files.is_empty() {
                    session.code_out.last_files = files.clone();
                    crate::commands::auto_write_files(session, &files);
                }
                session.history_mgr.push_turn(format!("/goal {}", condition), text);
            }
            Err(e) => {
                println!(
                    "{} {} tool loop failed: {}",
                    ui::theme::paint_error_label(&t, "\u{258C}"),
                    ui::theme::paint_error_label(&t, "\u{2717}"),
                    e
                );
            }
        }
        println!("{}", ui::theme::paint_rail_empty(&t));
        return;
    }

    let goal_prompt_text = format!(
        "GOAL: {}\n\nYour task is to achieve this goal. You may need to:\n\
         1. Plan your approach\n\
         2. Write code/files using ### path/file headings\n\
         3. We will run tests/linters and report back\n\
         4. Fix any issues based on tool output\n\n\
         When you believe the goal is achieved, say GOAL_ACHIEVED: <summary>.\n\
         If you are stuck, say GOAL_FAILED: <reason>.",
        condition
    );

    let max_iter = crate::constants::GOAL_MAX_ITERATIONS;
    let mut last_tool_output = String::new();
    let mut last_tool_hash: u64 = 0;
    let mut last_response_hash: u64 = 0;
    let mut last_written_files: Vec<String> = Vec::new();
    let mut final_iteration_text: Option<String> = None;
    let mut consecutive_empty_plans: u32 = 0;

    for i in 0..max_iter {
        if i > 0 {
            println!("{}", ui::theme::paint_rail_empty(&t));
        }
        println!(
            "{} {} {}/{}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint(&t, "accent", "iteration", true),
            i + 1,
            max_iter
        );

        let prompt = if last_tool_output.is_empty() {
            goal_prompt_text.clone()
        } else {
            build_agentic_prompt(&goal_prompt_text, &last_tool_output, i, max_iter)
        };

        let result = tokio::time::timeout(
            ITERATION_TIMEOUT,
            client.complete_chat_stream(&prompt, CHAT_SYSTEM_PROMPT_CODE, ""),
        )
        .await;

        let text = match result {
            Ok(Ok(text)) => text,
            Ok(Err(e)) => {
                println!(
                    "{} {} {}",
                    ui::theme::paint_error_label(&t, "\u{258C}"),
                    ui::theme::paint_error_label(&t, "\u{2717}"),
                    e
                );
                break;
            }
            Err(_) => {
                println!(
                    "{} {} iteration timed out after 120s",
                    ui::theme::paint_warning(&t, "\u{258C}"),
                    ui::theme::paint_warning(&t, "\u{23F3}")
                );
                break;
            }
        };

        let cleaned = text.trim().to_string();
        final_iteration_text = Some(cleaned.clone());

        let files = extract_code_blocks_with_names(&cleaned);
        let code = extract_code_block(&cleaned);
        if files.is_empty() && code.is_empty() {
            // Only count as "stuck" if the response also has no useful analysis text
            let has_analysis = cleaned.len() > 50;
            if has_analysis {
                consecutive_empty_plans = 0;
            } else {
                consecutive_empty_plans += 1;
            }
            if consecutive_empty_plans >= 3 {
                println!(
                    "{} {} no code generated for 3 iterations — goal appears stuck, stopping",
                    ui::theme::paint_warning(&t, "\u{258C}"),
                    ui::theme::paint_warning(&t, "!")
                );
                break;
            }
        } else {
            consecutive_empty_plans = 0;
        }
        if !files.is_empty() {
            session.code_out.last_files = files.clone();
            session.code_out.last_code = if code.is_empty() { String::new() } else { code };
            crate::commands::auto_write_files(session, &files);
            last_written_files = files.iter().map(|f| f.path.clone()).collect();
        } else if !code.is_empty() {
            session.code_out.last_code = code;
            session.code_out.last_files.clear();
            println!(
                "{} {} use /write <path> to save",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_dim(&t, "code detected \u{2014}")
            );
        }

        if let Some((achieved, msg)) = extract_goal_signal(&cleaned) {
            if achieved {
                println!(
                    "{} {} goal achieved! {}",
                    ui::theme::paint_success_label(&t, "\u{258C}"),
                    ui::theme::paint_success_label(&t, "\u{2713}"),
                    msg
                );
            } else {
                println!(
                    "{} {} {}",
                    ui::theme::paint_warning(&t, "\u{258C}"),
                    ui::theme::paint_warning(&t, "!"),
                    msg
                );
            }
            break;
        }

        // Response-level circuit breaker: detect repeated LLM output (stalling)
        let response_hash = circuit_breaker_hash(&cleaned);
        if response_hash == last_response_hash && i > 0 {
            println!(
                "{} {} circuit breaker: same response as previous iteration, stopping",
                ui::theme::paint_warning(&t, "\u{258C}"),
                ui::theme::paint_warning(&t, "!")
            );
            break;
        }
        last_response_hash = response_hash;

        let mut tool_results = String::new();
        if !last_written_files.is_empty() {
            for file_path in &last_written_files {
                let lint_result = run_lint(file_path).await;
                println!("{}", format_tool_output(&lint_result));

                let is_test_file = file_path.contains("test")
                    || file_path.contains("spec")
                    || file_path.ends_with("_test.rs")
                    || file_path.ends_with("_test.py")
                    || file_path.ends_with("_spec.js");
                let test_result = if is_test_file {
                    let r = run_test(file_path).await;
                    if !r.stderr.is_empty() || !r.stdout.is_empty() {
                        println!("{}", format_tool_output(&r));
                    }
                    r
                } else {
                    ToolOutput {
                        tool_name: "test".into(),
                        success: true,
                        stdout: "[skipped — not a test file]".into(),
                        stderr: String::new(),
                        duration_ms: 0,
                        action: "test".into(),
                    }
                };

                tool_results.push_str(&build_tool_context(Some(&lint_result), Some(&test_result), None));
            }
        } else {
            tool_results.push_str(&format!("[No files written in this iteration]\n{}", cleaned));
        }

        // Tool-level circuit breaker: detect repeated tool output
        if !tool_results.is_empty() {
            let new_hash = circuit_breaker_hash(&tool_results);
            if new_hash == last_tool_hash && !tool_results.is_empty() && i > 0 {
                println!(
                    "{} {} circuit breaker: same results as previous iteration, stopping",
                    ui::theme::paint_warning(&t, "\u{258C}"),
                    ui::theme::paint_warning(&t, "!")
                );
                break;
            }
            last_tool_hash = new_hash;
        }

        if tool_results.len() > MAX_TOOL_OUTPUT_LEN {
            tool_results.truncate(MAX_TOOL_OUTPUT_LEN);
            tool_results.push_str("\n... [truncated]");
        }
        last_tool_output = tool_results;
    }
    if let Some(final_text) = final_iteration_text {
        session
            .history_mgr
            .push_turn(format!("/goal {}", condition), final_text);
    }
    println!("{}", ui::theme::paint_rail_empty(&t));
}
