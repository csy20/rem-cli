//! Goal-driven autonomous loop (`/goal`).
//! Iteratively generates code, runs lint/tests, and feeds results back to
//! the LLM until the goal is achieved or max iterations are reached.
//! Supports checkpointing for multi-turn resume after interruption.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::agentic::{
    build_agentic_prompt, build_tool_context, extract_goal_signal, format_tool_output, run_lint, run_test, ToolOutput,
};
use crate::chat::ChatSession;
use crate::constants::CHAT_SYSTEM_PROMPT_CODE;
use crate::parsing::extract_code_block;
use crate::provider::Provider;
use crate::tool_executor;
use crate::types::{extract_code_blocks_with_names, FileEntry};
use crate::ui;

const MAX_TOOL_OUTPUT_LEN: usize = crate::constants::GOAL_TOOL_OUTPUT_MAX_CHARS;
const ITERATION_TIMEOUT: Duration = crate::constants::GOAL_ITERATION_TIMEOUT;

/// Checkpoint state for multi-turn goal resume.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalCheckpoint {
    condition: String,
    iteration: usize,
    last_tool_output: String,
    last_response_hash: u64,
    last_tool_hash: u64,
    last_written_files: Vec<String>,
}

fn checkpoint_path(session: &ChatSession) -> PathBuf {
    let dir = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    dir.join(".rem/goal_checkpoint.json")
}

fn save_checkpoint(session: &ChatSession, cp: &GoalCheckpoint) {
    let path = checkpoint_path(session);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(cp) {
        if let Err(e) = std::fs::write(&path, &json) {
            tracing::warn!("failed to save goal checkpoint: {}", e);
        }
    }
}

fn load_checkpoint(session: &ChatSession) -> Option<GoalCheckpoint> {
    let path = checkpoint_path(session);
    if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<GoalCheckpoint>(&s).ok())
    } else {
        None
    }
}

fn clear_checkpoint(session: &ChatSession) {
    let path = checkpoint_path(session);
    let _ = std::fs::remove_file(&path);
}

fn circuit_breaker_hash(output: &str, iteration: usize) -> u64 {
    let mut hasher = DefaultHasher::new();
    iteration.hash(&mut hasher);
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
        let project_path = session.ctx.project_dir.clone().unwrap_or_else(|| PathBuf::from("."));
        struct GoalUserInteraction<'a> {
            session: &'a mut ChatSession,
        }
        impl tool_executor::UserInteraction for GoalUserInteraction<'_> {
            fn approve_command(&mut self, _cmd: &str) -> bool {
                match self.session.readline("(y/N) ") {
                    Ok(line) => {
                        let trimmed = line.trim().to_lowercase();
                        trimmed == "y" || trimmed == "yes"
                    }
                    Err(_) => false,
                }
            }
            fn ask_question(&mut self, question: &str) -> Option<String> {
                match self.session.readline(&format!("REM asks: {} ", question)) {
                    Ok(line) => Some(line.trim().to_string()),
                    Err(_) => None,
                }
            }
        }
        let mut user = GoalUserInteraction { session };
        let project_dir: &Path = &project_path;
        let result = tool_executor::run_tool_loop(
            client,
            condition,
            "[MODE: CODE] Use tools to achieve the goal. When done, explain what was accomplished.",
            "",
            &mut user,
            project_dir,
        )
        .await;
        match result {
            Ok((text, writes)) => {
                clear_checkpoint(session);
                println!("\n{}", text);
                println!("{}", ui::theme::paint_rail_empty(&t));
                println!(
                    "{} {} goal completed",
                    ui::theme::paint_success_label(&t, "\u{258C}"),
                    ui::theme::paint_success_label(&t, "\u{2713}")
                );

                let files: Vec<FileEntry> = writes
                    .into_iter()
                    .map(|p| {
                        let content = std::fs::read_to_string(&p).unwrap_or_default();
                        FileEntry { path: p, content }
                    })
                    .collect();
                if !files.is_empty() {
                    session.code_out.last_files = files.clone();
                    crate::commands::auto_write_files(session, &files);
                }
                session.history_mgr.push_turn(format!("/goal {}", condition), text);
            }
            Err(e) => {
                clear_checkpoint(session);
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

    let max_iter = crate::constants::GOAL_MAX_ITERATIONS;
    let mut last_tool_output = String::new();
    let mut last_tool_hash: u64 = 0;
    let mut last_response_hash: u64 = 0;
    let mut last_written_files: Vec<String> = Vec::new();
    let mut final_iteration_text: Option<String> = None;
    let mut consecutive_empty_plans: u32 = 0;
    let mut start_iteration = 0usize;

    // Check for existing checkpoint and offer resume
    if let Some(cp) = load_checkpoint(session) {
        if cp.condition == condition {
            println!(
                "{} {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_bright(&t, "found checkpoint from previous /goal session")
            );
            println!(
                "{} {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_dim(&t, "resume from where we left off? [Y/n]")
            );
            let should_resume =
                match session.readline(&format!("{}   ", ui::theme::paint(&t, "accent", "\u{258C}", true))) {
                    Ok(line) => {
                        let trimmed = line.trim().to_lowercase();
                        trimmed.is_empty() || trimmed == "y" || trimmed == "yes"
                    }
                    Err(_) => true,
                };
            if should_resume {
                start_iteration = cp.iteration;
                last_tool_output = cp.last_tool_output;
                last_response_hash = cp.last_response_hash;
                last_tool_hash = cp.last_tool_hash;
                last_written_files = cp.last_written_files;
                println!(
                    "{} {} iteration {}",
                    ui::theme::paint_success_label(&t, "\u{2713}"),
                    ui::theme::paint_dim(&t, "resuming from"),
                    start_iteration + 1
                );
            } else {
                clear_checkpoint(session);
                println!(
                    "{} {}",
                    ui::theme::paint(&t, "accent", "\u{258C}", true),
                    ui::theme::paint_dim(&t, "starting fresh")
                );
            }
        } else {
            println!(
                "{} {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_dim(&t, "checkpoint has different goal \u{2014} starting fresh")
            );
            clear_checkpoint(session);
        }
        println!("{}", ui::theme::paint_rail_empty(&t));
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

    for i in start_iteration..max_iter {
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
            build_agentic_prompt(&goal_prompt_text, &last_tool_output, i + 1, max_iter)
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
            clear_checkpoint(session);
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
        let response_hash = circuit_breaker_hash(&cleaned, i);
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
                let p = std::path::Path::new(file_path);
                let stem = p.file_stem().map(|s| s.to_string_lossy()).unwrap_or_default();
                let is_test_file =
                    stem.ends_with("_test") || stem.ends_with("_spec") || stem == "test" || stem == "spec";

                let lint_result = run_lint(file_path).await;
                println!("{}", format_tool_output(&lint_result));

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
            tool_results.push_str("[No files written in this iteration]");
        }

        // Tool-level circuit breaker: detect repeated tool output
        if !tool_results.is_empty() {
            let new_hash = circuit_breaker_hash(&tool_results, i);
            if new_hash == last_tool_hash && i > 0 {
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
            let cutoff = tool_results.floor_char_boundary(MAX_TOOL_OUTPUT_LEN);
            tool_results.truncate(cutoff);
            tool_results.push_str("\n... [truncated]");
        }
        last_tool_output = tool_results;

        // Save checkpoint for multi-turn resume
        save_checkpoint(
            session,
            &GoalCheckpoint {
                condition: condition.to_string(),
                iteration: i + 1,
                last_tool_output: last_tool_output.clone(),
                last_response_hash,
                last_tool_hash,
                last_written_files: last_written_files.clone(),
            },
        );
    }
    // Clear checkpoint on any exit (completed, stuck, or max iterations)
    clear_checkpoint(session);
    if let Some(final_text) = final_iteration_text {
        session
            .history_mgr
            .push_turn(format!("/goal {}", condition), final_text);
    }
    println!("{}", ui::theme::paint_rail_empty(&t));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circuit_breaker_hash_different_iterations_differ() {
        let output = "fixed output";
        let h1 = circuit_breaker_hash(output, 1);
        let h2 = circuit_breaker_hash(output, 2);
        assert_ne!(h1, h2);
    }

    #[test]
    fn circuit_breaker_hash_different_outputs_differ() {
        let h1 = circuit_breaker_hash("output_a", 1);
        let h2 = circuit_breaker_hash("output_b", 1);
        assert_ne!(h1, h2);
    }

    #[test]
    fn circuit_breaker_hash_same_inputs_match() {
        let h1 = circuit_breaker_hash("hello world", 5);
        let h2 = circuit_breaker_hash("hello world", 5);
        assert_eq!(h1, h2);
    }

    #[test]
    fn circuit_breaker_hash_empty_output() {
        let h = circuit_breaker_hash("", 0);
        assert_ne!(h, 0);
    }
}
