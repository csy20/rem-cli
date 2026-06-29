//! Tool commands (`/search`, `/explain`, `/test`, `/refactor`, `/lint`, `/find`).
//! Handlers for commands that invoke external tools or LLM-powered analysis.

use crate::agentic::{format_tool_output, run_lint};
use crate::chat::ChatSession;
use crate::cli::AppConfig;
use crate::find::{find_matches, FindOptions};
use crate::parsing::extract_code_block;
use crate::provider::Provider;
use crate::search::{perform_web_search, print_search_results, provider_from_config};
use crate::truncate_to_lines;
use crate::ui;
use std::fs;
use std::path::Path;

/// Performs a web search (`/search` command).
pub(crate) async fn handle_search(_client: &Provider, session: &mut ChatSession, cfg: &AppConfig, query: &str) {
    let t = ui::theme::active();
    println!(
        "{} {} searching the web...",
        ui::theme::paint_rail_empty(&t),
        ui::theme::paint(&t, "accent", "🔍", true)
    );
    let search_provider = provider_from_config(
        &cfg.search_provider,
        cfg.search_api_key.as_deref().unwrap_or(""),
        cfg.search_cse_id.as_deref().unwrap_or(""),
    );
    match perform_web_search(&crate::provider::HTTP_CLIENT.clone(), query, search_provider.as_ref()).await {
        Ok(results) => {
            if results.is_empty() {
                println!("{} no results found for: {}", ui::theme::paint_warning(&t, "│"), query);
            } else {
                println!(
                    "{} {} results for: {}",
                    ui::theme::paint_rail_empty(&t),
                    results.len(),
                    ui::theme::paint_bright(&t, query)
                );
                print_search_results(&results);
                session.last_search = results;
            }
        }
        Err(e) => {
            println!("{} {}", ui::theme::paint_error_label(&t, "│  search failed:"), e);
        }
    }
}

/// Explains code or commands (`/explain` command).
pub(crate) async fn handle_explain(client: &Provider, session: &mut ChatSession, text: &str) {
    let t = ui::theme::active();
    if text.trim().is_empty() {
        println!("{} usage: /explain <code snippet>", ui::theme::paint_warning(&t, "│"));
        return;
    }
    println!("{} explaining...", ui::theme::paint(&t, "accent", "\u{258C}", true));
    let prompt = format!(
        "Explain what the following code does in clear, plain language. \
         Be concise but thorough. Cover: purpose, key components, control flow. \
         Do NOT generate new code. Just explain.\n\nCode:\n```\n{}\n```",
        text
    );
    match client.complete_chat_stream(
        &prompt,
        "[MODE: CHAT] You are a code explainer. Respond with plain text only — no code generation, no file format, no JSON.",
        "",
    ).await {
        Ok(response) => {
            println!("\n{}", response);
            session.add_history(&format!("/explain {}", text));
            session.history_mgr.push_turn(format!("/explain {}", text), response);
        }
        Err(e) => {
            println!("\n{} explain failed: {}", ui::theme::paint_error_label(&t, "│"), e);
        }
    }
}

/// Generates tests for a file (`/test` command).
pub(crate) async fn handle_test(client: &Provider, session: &mut ChatSession, path: &str) {
    let t = ui::theme::active();
    let file_path = Path::new(path.trim());
    if !file_path.exists() {
        println!("{} file not found: {}", ui::theme::paint_warning(&t, "│"), path);
        return;
    }
    let content = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            println!("{} cannot read file: {}", ui::theme::paint_error_label(&t, "│"), e);
            return;
        }
    };
    println!(
        "{} generating tests for {}...",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        path
    );
    let prompt = format!(
        "Generate comprehensive tests for the following code. \
         Include unit tests for all public functions/methods, edge cases, \
         and error handling. Write tests in the same language and testing \
         framework conventions.\n\nSource code:\n```\n{}\n```",
        truncate_to_lines(&content, 200)
    );
    match client.complete_chat_stream(
        &prompt,
        "[MODE: CODE] Generate test code for the given source file. Respond with the test code in a fenced code block.",
        "",
    ).await {
        Ok(response) => {
            println!();
            println!("{}", response);
            session.code_out.last_code = extract_code_block(&response);
            session.add_history(&format!("/test {}", path));
            session.history_mgr.push_turn(format!("/test {}", path), response);
            if !session.code_out.last_code.is_empty() {
                println!("{} tests ready — use {} to save",
                    ui::theme::paint_success_label(&t, "│"),
                    ui::theme::paint_bright(&t, "/write <path>"));
            }
        }
        Err(e) => {
            println!("\n{} test generation failed: {}", ui::theme::paint_error_label(&t, "│"), e);
        }
    }
}

/// Suggests refactoring for a file (`/refactor` command).
pub(crate) async fn handle_refactor(client: &Provider, session: &mut ChatSession, path: &str) {
    let t = ui::theme::active();
    let file_path = Path::new(path.trim());
    if !file_path.exists() {
        println!("{} file not found: {}", ui::theme::paint_warning(&t, "│"), path);
        return;
    }
    let content = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            println!("{} cannot read file: {}", ui::theme::paint_error_label(&t, "│"), e);
            return;
        }
    };
    println!(
        "{} analyzing {} for refactoring...",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        path
    );
    let prompt = format!(
        "Review the following code and suggest refactoring improvements. \
         Consider: code clarity, DRY principle, performance, error handling, \
         naming, structure. Give specific recommendations with before/after \
         code examples where helpful.\n\nSource code:\n```\n{}\n```",
        truncate_to_lines(&content, 200)
    );
    match client.complete_chat_stream(
        &prompt,
        "[MODE: CHAT] You are a code reviewer. Analyze the code and provide refactoring suggestions. Use clear markdown formatting.",
        "",
    ).await {
        Ok(response) => {
            println!();
            println!("{}", response);
            session.add_history(&format!("/refactor {}", path));
            session.history_mgr.push_turn(format!("/refactor {}", path), response);
        }
        Err(e) => {
            println!("\n{} refactor analysis failed: {}", ui::theme::paint_error_label(&t, "│"), e);
        }
    }
}

/// Runs a linter on the specified file (`/lint` command).
pub(crate) async fn handle_lint(_session: &mut ChatSession, path: &str) {
    let t = ui::theme::active();
    let file_path = Path::new(path);
    if !file_path.exists() {
        println!("{} file not found: {}", ui::theme::paint_warning(&t, "\u{258C}"), path);
        return;
    }
    println!(
        "{} linting {}...",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        path
    );
    let result = run_lint(path).await;
    println!("{}", format_tool_output(&result));
}

/// Handles `/lint` with automatic fallback to the last written files when no arg is given.
pub(crate) async fn handle_lint_with_fallback(session: &mut ChatSession, args: &str) {
    let t = ui::theme::active();
    if args.is_empty() {
        if session.code_out.last_files.is_empty() && session.code_out.last_files_written.is_empty() {
            println!(
                "{} no files to lint. Generate code first.",
                ui::theme::paint_warning(&t, "\u{258C}")
            );
        } else {
            let paths: Vec<String> = if !session.code_out.last_files_written.is_empty() {
                session
                    .code_out
                    .last_files_written
                    .iter()
                    .map(|p| p.path.display().to_string())
                    .collect()
            } else {
                session
                    .code_out
                    .last_files
                    .iter()
                    .filter(|f| !f.path.is_empty())
                    .map(|f| f.path.clone())
                    .collect()
            };
            for p in paths {
                handle_lint(session, &p).await;
            }
        }
    } else {
        handle_lint(session, args).await;
    }
}

/// Searches for text in project files (`/find` command).
pub(crate) fn handle_find(session: &ChatSession, query: &str) {
    let t = ui::theme::active();
    if query.is_empty() {
        println!("{}", ui::theme::paint_rail_empty(&t));
        println!(
            "{} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "usage: /find <query>")
        );
        println!(
            "{}  {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_dim(
                &t,
                "search text inside the project (skips node_modules, target, .git, ...)"
            )
        );
        println!("{}", ui::theme::paint_rail_empty(&t));
        return;
    }

    let root = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let report = find_matches(&root, query, &FindOptions::default());

    println!("{}", ui::theme::paint_rail_empty(&t));
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, &format!("\u{203a} FIND  {}", query)),
    );
    println!(
        "{} {} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_dim(&t, "in"),
        ui::theme::paint_bright(&t, &format!("{}", root.display()))
    );
    println!("{}", ui::theme::paint_rail_empty(&t));

    if report.matches.is_empty() {
        println!(
            "{} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_warning(&t, "(no matches)")
        );
        println!("{}", ui::theme::paint_rail_empty(&t));
        return;
    }

    let show_limit = 50usize;
    let shown = report.matches.len().min(show_limit);
    let mut last_path: Option<String> = None;
    for m in report.matches.iter().take(show_limit) {
        let rel = m
            .path
            .strip_prefix(&root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| m.path.display().to_string());
        if last_path.as_deref() != Some(rel.as_str()) {
            if last_path.is_some() {
                println!("{}", ui::theme::paint_rail_empty(&t));
            }
            println!(
                "{} {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint(
                    &t,
                    "accent_info",
                    &format!("\u{2500}\u{2500} {} \u{2500}\u{2500}", rel),
                    true
                ),
            );
            last_path = Some(rel);
        }
        let line_no_w = 4usize;
        let col_w = 3usize;
        println!(
            "{} {}   {:>lw$}:{:<cw$}  {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_dim(&t, "\u{251c}\u{2500}\u{2500}"),
            m.line_no,
            m.column,
            ui::theme::paint_bright(&t, &trim_for_display(&m.line, 120)),
            lw = line_no_w,
            cw = col_w
        );
    }
    println!("{}", ui::theme::paint_rail_empty(&t));

    let unique_files: std::collections::BTreeSet<String> = report
        .matches
        .iter()
        .map(|m| {
            m.path
                .strip_prefix(&root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| m.path.display().to_string())
        })
        .collect();

    let mut summary = format!(
        "  {} match{} in {} file{} · scanned {} · skipped {} · {}ms",
        report.matches.len(),
        if report.matches.len() == 1 { "" } else { "es" },
        unique_files.len(),
        if unique_files.len() == 1 { "" } else { "s" },
        report.files_scanned,
        report.files_skipped,
        report.elapsed_ms,
    );
    if report.truncated {
        summary.push_str("  (truncated)");
    }
    if shown < report.matches.len() {
        summary.push_str(&format!("  (showing first {})", shown));
    }
    println!("{}", ui::theme::paint_rail_empty(&t));
    println!("{}", ui::theme::paint_success_label(&t, &summary));
    println!("{}", ui::theme::paint_rail_empty(&t));
}

/// Truncates a string to a max number of characters for display.
fn trim_for_display(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('\u{2026}');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_for_display_short_string() {
        assert_eq!(trim_for_display("hello", 10), "hello");
    }

    #[test]
    fn trim_for_display_exact_fit() {
        assert_eq!(trim_for_display("hello", 5), "hello");
    }

    #[test]
    fn trim_for_display_truncates_with_ellipsis() {
        let result = trim_for_display("hello world", 5);
        assert_eq!(result.chars().count(), 5);
        assert!(result.ends_with('\u{2026}'));
    }

    #[test]
    fn trim_for_display_empty_string() {
        assert_eq!(trim_for_display("", 5), "");
    }

    #[test]
    fn trim_for_display_max_zero() {
        let result = trim_for_display("hello", 0);
        assert_eq!(result, "\u{2026}");
    }

    #[test]
    fn trim_for_display_unicode_preserved() {
        let input = "héllo wörld";
        let result = trim_for_display(input, 20);
        assert_eq!(result, input);
    }

    #[test]
    fn trim_for_display_unicode_truncated() {
        let input = "héllo wörld 👍";
        let result = trim_for_display(input, 6);
        assert_eq!(result.chars().count(), 6);
        assert!(result.ends_with('\u{2026}'));
    }
}
