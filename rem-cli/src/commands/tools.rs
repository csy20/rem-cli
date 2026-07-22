//! Tool commands (`/search`, `/explain`, `/test`, `/refactor`, `/lint`, `/find`, `/observe`).
//! Handlers for commands that invoke external tools or LLM-powered analysis.

use crate::agentic::{format_tool_output, run_lint};
use crate::chat::ChatSession;
use crate::cli::AppConfig;
use crate::find::{find_matches, FindOptions};
use crate::mcp::signoz::{SignozClient, SignozConfig};
use crate::pager::maybe_page;
use crate::parsing::extract_code_block;
use crate::provider::Provider;
use crate::search::{perform_web_search, print_search_results, provider_from_config};
use crate::truncate_to_lines;
use crate::ui;
use std::fs;

/// Query SigNoz via MCP and answer with the active LLM (`/observe` command).
pub(crate) async fn handle_observe(client: &Provider, session: &mut ChatSession, cfg: &AppConfig, query: &str) {
    let t = ui::theme::active();
    if query.trim().is_empty() {
        println!("{} usage: /observe <query>", ui::theme::paint_warning(&t, "│"));
        println!("{} examples:", ui::theme::paint_rail_empty(&t));
        println!(
            "{}   /observe which tasks used fireworks and why",
            ui::theme::paint_rail_empty(&t)
        );
        println!(
            "{}   /observe show me the slowest task in the last run",
            ui::theme::paint_rail_empty(&t)
        );
        return;
    }

    println!(
        "{} {} querying SigNoz MCP...",
        ui::theme::paint_rail_empty(&t),
        ui::theme::paint(&t, "accent", "📡", true)
    );

    let signoz_cfg = SignozConfig::from_app(
        Some(cfg.signoz_mcp_url.as_str()),
        cfg.signoz_api_key.as_deref(),
        cfg.signoz_url.as_deref(),
        Some(cfg.signoz_service.as_str()),
    );

    let mut mcp = match SignozClient::new(signoz_cfg) {
        Ok(c) => c,
        Err(e) => {
            println!("{} {}", ui::theme::paint_error_label(&t, "│  observe init failed:"), e);
            return;
        }
    };

    let context = match mcp.observe_context(query).await {
        Ok(ctx) => ctx,
        Err(e) => {
            println!("{} {}", ui::theme::paint_error_label(&t, "│  SigNoz MCP failed:"), e);
            println!(
                "{} tip: set signoz_mcp_url / SIGNOZ_MCP_URL (default http://localhost:8000/mcp)",
                ui::theme::paint_warning(&t, "│")
            );
            println!(
                "{}      and ensure the MCP server is running (Foundry mcp.enabled=true)",
                ui::theme::paint_warning(&t, "│")
            );
            return;
        }
    };

    // Show a short preview of raw context (truncated)
    let preview: String = context.chars().take(600).collect();
    println!(
        "{} {} fetched {} chars of trace context",
        ui::theme::paint_rail_empty(&t),
        ui::theme::paint(&t, "accent", "✓", true),
        context.len()
    );
    if !preview.is_empty() {
        println!(
            "{} {}",
            ui::theme::paint_dim(&t, "│"),
            ui::theme::paint_dim(&t, &preview.replace('\n', " · "))
        );
    }

    println!(
        "{} {} analyzing with {}...",
        ui::theme::paint_rail_empty(&t),
        ui::theme::paint(&t, "accent", "🤖", true),
        client.provider_label()
    );

    let prompt = format!(
        "You are an SRE sidekick debugging the router-agent service using REAL OpenTelemetry \
         span data from SigNoz (via MCP). Answer the user's question using ONLY the provided \
         context. Cite concrete span attributes (task_id, category, stage, accepted, confidence, \
         tokens_prompt, tokens_completion, latency_ms, model). If the context is insufficient, \
         say what is missing — do NOT invent traces.\n\n\
         USER QUESTION:\n{query}\n\n\
         SIGNOZ CONTEXT:\n{context}"
    );

    match client
        .complete_chat_stream(
            &prompt,
            "[MODE: CHAT] You are an observability SRE assistant. Cite real span attributes. \
             No code generation. Be concise and structured.",
            "",
        )
        .await
    {
        Ok(response) => {
            println!("\n{}", response);
            session.add_history(&format!("/observe {}", query));
            session.history_mgr.push_turn(format!("/observe {}", query), response);
        }
        Err(e) => {
            println!("\n{} observe LLM failed: {}", ui::theme::paint_error_label(&t, "│"), e);
            // Still dump raw context so the user can debug without the model
            println!(
                "\n{} raw SigNoz context follows:\n{}",
                ui::theme::paint_warning(&t, "│"),
                context
            );
        }
    }
}

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
    match perform_web_search(&crate::provider::HTTP_CLIENT, query, search_provider.as_ref()).await {
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
    let base = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let file_path = match crate::types::resolve_safe_path(&base, path.trim()) {
        Some(p) => p,
        None => {
            println!("{} invalid path: {}", ui::theme::paint_warning(&t, "│"), path);
            return;
        }
    };
    if !file_path.exists() {
        println!("{} file not found: {}", ui::theme::paint_warning(&t, "│"), path);
        return;
    }
    let content = match fs::read_to_string(&file_path) {
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
    let base = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let file_path = match crate::types::resolve_safe_path(&base, path.trim()) {
        Some(p) => p,
        None => {
            println!("{} invalid path: {}", ui::theme::paint_warning(&t, "│"), path);
            return;
        }
    };
    if !file_path.exists() {
        println!("{} file not found: {}", ui::theme::paint_warning(&t, "│"), path);
        return;
    }
    let content = match fs::read_to_string(&file_path) {
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
pub(crate) async fn handle_lint(session: &ChatSession, path: &str) {
    let t = ui::theme::active();
    let base = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let safe_path = match crate::types::resolve_safe_path(&base, path) {
        Some(p) => p,
        None => return,
    };
    if !safe_path.exists() {
        println!("{} file not found: {}", ui::theme::paint_warning(&t, "\u{258C}"), path);
        return;
    }
    println!(
        "{} linting {}...",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        path
    );
    let result = run_lint(safe_path.to_str().unwrap_or(path)).await;
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
                    .filter(|p| p.path.exists())
                    .map(|p| p.path.display().to_string())
                    .collect()
            } else {
                session
                    .code_out
                    .last_files
                    .iter()
                    .filter(|f| !f.path.is_empty())
                    .map(|f| {
                        let base = session
                            .ctx
                            .project_dir
                            .clone()
                            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                        crate::types::resolve_safe_path(&base, &f.path)
                            .filter(|p| p.exists())
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| f.path.clone())
                    })
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

    let find_body = format_find_matches(&t, &root, &report.matches, show_limit);
    if shown > 40 {
        maybe_page(&find_body);
    } else {
        print!("{}", find_body);
    }

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

/// Formats find match results into a display string (deduplicates paths, formats line/col).
fn format_find_matches(
    t: &ui::theme::Theme,
    root: &std::path::Path,
    matches: &[crate::find::Match],
    show_limit: usize,
) -> String {
    let mut buf = String::new();
    let mut last_path: Option<String> = None;
    for m in matches.iter().take(show_limit) {
        let rel = m
            .path
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| m.path.display().to_string());
        if last_path.as_deref() != Some(rel.as_str()) {
            if last_path.is_some() {
                buf.push_str(&format!("{}\n", ui::theme::paint_rail_empty(t)));
            }
            buf.push_str(&format!(
                "{} {}\n",
                ui::theme::paint(t, "accent", "\u{258C}", true),
                ui::theme::paint(
                    t,
                    "accent_info",
                    &format!("\u{2500}\u{2500} {} \u{2500}\u{2500}", rel),
                    true
                ),
            ));
            last_path = Some(rel);
        }
        let line_no_w = 4usize;
        let col_w = 3usize;
        buf.push_str(&format!(
            "{} {}   {:>lw$}:{:<cw$}  {}\n",
            ui::theme::paint(t, "accent", "\u{258C}", true),
            ui::theme::paint_dim(t, "\u{251c}\u{2500}\u{2500}"),
            m.line_no,
            m.column,
            ui::theme::paint_bright(t, &trim_for_display(&m.line, 120)),
            lw = line_no_w,
            cw = col_w
        ));
    }
    buf.push_str(&format!("{}\n", ui::theme::paint_rail_empty(t)));
    buf
}

/// Performs semantic code search (`/semantic` command).
/// Uses BM25 retrieval against the codebase index.
pub(crate) fn handle_semantic(session: &ChatSession, query: &str) {
    let t = ui::theme::active();
    if query.is_empty() {
        println!("{}", ui::theme::paint_rail_empty(&t));
        println!(
            "{} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "usage: /semantic <natural language query>")
        );
        println!(
            "{}  {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_dim(&t, "search the codebase index using BM25 retrieval")
        );
        println!("{}", ui::theme::paint_rail_empty(&t));
        return;
    }

    let root = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let index = match crate::indexer::load_codebase_index(&root) {
        Some(idx) => idx,
        None => {
            println!("{}", ui::theme::paint_rail_empty(&t));
            println!(
                "{} {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_warning(&t, "no codebase index found")
            );
            println!(
                "{} {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_dim(&t, "run `rem index` first to build the search index")
            );
            println!("{}", ui::theme::paint_rail_empty(&t));
            return;
        }
    };

    let hits = crate::indexer::retrieve_relevant_chunks(&index, query, 10, 6000);

    println!("{}", ui::theme::paint_rail_empty(&t));
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, &format!("\u{203a} SEMANTIC  {}", query)),
    );
    println!(
        "{} {} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_dim(&t, "in"),
        ui::theme::paint_bright(&t, &format!("{}", root.display()))
    );
    println!("{}", ui::theme::paint_rail_empty(&t));

    if hits.is_empty() {
        println!(
            "{} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_warning(&t, "(no relevant chunks found)")
        );
        println!("{}", ui::theme::paint_rail_empty(&t));
        return;
    }

    let mut buf = String::new();
    for (i, c) in hits.iter().enumerate() {
        let loc = if c.start_line > 0 && c.end_line > 0 {
            format!("{}:{}-{}", c.path, c.start_line, c.end_line)
        } else {
            c.path.clone()
        };
        let type_tag = if !c.chunk_type.is_empty() && c.chunk_type != "file" {
            format!(" ({})", c.chunk_type)
        } else {
            String::new()
        };
        buf.push_str(&format!(
            "{} {}  {}{}\n",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_dim(&t, &format!("#{}", i + 1)),
            ui::theme::paint_bright(&t, &loc),
            ui::theme::paint_dim(&t, &type_tag),
        ));
        for line in c.content.lines().take(5) {
            let trimmed = if line.len() > 120 {
                format!("{}…", &line[..120])
            } else {
                line.to_string()
            };
            buf.push_str(&format!(
                "{} {}   {}\n",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_dim(&t, "\u{251c}\u{2500}\u{2500}"),
                ui::theme::paint(&t, "text_faint", &trimmed, false),
            ));
        }
        if c.content.lines().count() > 5 {
            buf.push_str(&format!(
                "{} {}   {} more lines\n",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_dim(&t, "\u{251c}\u{2500}\u{2500}"),
                c.content.lines().count() - 5,
            ));
        }
        buf.push('\n');
    }
    buf.push_str(&format!("{}\n", ui::theme::paint_rail_empty(&t)));
    let summary = format!(
        "  {} chunk{} in {} · {} chunks indexed",
        hits.len(),
        if hits.len() == 1 { "" } else { "s" },
        {
            let mut files: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
            for c in &hits {
                files.insert(&c.path);
            }
            files.len()
        },
        index.num_chunks,
    );
    buf.push_str(&ui::theme::paint_success_label(&t, &summary));
    buf.push('\n');
    buf.push_str(&format!("{}\n", ui::theme::paint_rail_empty(&t)));

    if hits.len() > 5 {
        crate::pager::maybe_page(&buf);
    } else {
        print!("{}", buf);
    }
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
