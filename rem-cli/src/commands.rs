use crate::agentic::{
    build_agentic_prompt, build_tool_context, extract_goal_signal, format_tool_output, run_lint,
    run_test,
};
use crate::chat::{detect_project_type, ChatSession};
use crate::config::persist_workspace;
use crate::find::{find_matches, FindOptions};
use crate::highlight;
use crate::intent::TaskIntent;
use crate::memory::ProjectMemory;
use crate::parsing::extract_code_block;
use crate::provider::Provider;
use crate::search::{perform_web_search, print_search_results};
use crate::ui;
use crate::{
    extract_code_blocks_with_names, file_icon, format_timestamp, human_size, resolve_safe_path,
    truncate_bytes, truncate_to_lines, FileEntry, CHAT_SYSTEM_PROMPT_CODE,
};
use std::fs;
use std::io::{self};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub(crate) fn prompt_for_path(session: &mut ChatSession) -> io::Result<String> {
    let t = ui::theme::active();
    let workspace_display = session
        .project_dir
        .as_ref()
        .map(|d| d.display().to_string())
        .unwrap_or_else(|| "current dir".to_string());
    println!("{}", ui::theme::paint(&t, "accent", "\u{258C}", true));
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent_info", "│  ?", true),
        ui::theme::paint_bright(
            &t,
            "Where should I create this? (e.g. ./my-site/index.html or ./project/)"
        )
    );
    println!(
        "{} workspace: {}",
        ui::theme::paint(&t, "accent_info", "│", true),
        ui::theme::paint_bright(&t, &workspace_display.to_string())
    );
    println!(
        "{} type '.' for workspace root, or /dir <path> to change",
        ui::theme::paint(&t, "accent_info", "│", true),
    );
    println!("{}", ui::theme::paint(&t, "accent", "\u{258C}", true));

    loop {
        let line = session.readline("rem> path: ");
        let line = match line {
            Ok(s) => s,
            Err(_) => return Ok(".".to_string()),
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        session.add_history(trimmed);

        if trimmed.eq_ignore_ascii_case("exit") || trimmed.eq_ignore_ascii_case("quit") {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }

        if let Some(tail) = trimmed.strip_prefix("/dir ") {
            handle_dir(session, tail);
            continue;
        }

        return Ok(trimmed.to_string());
    }
}

pub(crate) fn handle_write(session: &mut ChatSession, path: &str) {
    let t = ui::theme::active();
    let trimmed = path.trim();
    let base_dir = session
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let abs_path = match resolve_safe_path(&base_dir, trimmed) {
        Some(p) => p,
        None => return,
    };

    if session.last_code.is_empty() {
        println!(
            "  {} No code from last response. Use `/code` to view it.",
            ui::theme::paint_warning(&t, "!")
        );
        return;
    }

    if abs_path.exists() {
        let existing_size = fs::metadata(&abs_path).map(|m| m.len()).unwrap_or(0);
        println!(
            "  {} {} exists ({} bytes) — {} [y/N]",
            ui::theme::paint_warning(&t, "\u{26a0}"),
            ui::theme::paint_bright(&t, trimmed),
            existing_size,
            ui::theme::paint_dim(&t, "overwrite?")
        );
        let input = session.readline("rem> ").unwrap_or_else(|_| String::new());
        if !input.trim().eq_ignore_ascii_case("y") && !input.trim().eq_ignore_ascii_case("yes") {
            println!("  {} skipped", ui::theme::paint_rail_empty(&t));
            return;
        }
    }

    if let Some(parent) = abs_path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = fs::create_dir_all(parent) {
                eprintln!(
                    "  {} cannot create directory {}: {}",
                    ui::theme::paint_error_label(&t, "\u{2717}"),
                    parent.display(),
                    e
                );
                return;
            }
        }
    }

    let tmp = abs_path.with_extension("tmp");
    match fs::write(&tmp, &session.last_code) {
        Ok(()) => {
            if let Err(e) = fs::rename(&tmp, &abs_path) {
                eprintln!(
                    "  {} atomic write failed: {}",
                    ui::theme::paint_error_label(&t, "\u{2717}"),
                    e
                );
                let _ = fs::remove_file(&tmp);
                return;
            }
            println!(
                "  {} wrote {} ({} bytes)",
                ui::theme::paint_success_label(&t, "\u{2713}"),
                ui::theme::paint_bright(&t, &format!("{}", abs_path.display())),
                session.last_code.len()
            );
            session.last_files_written.push(abs_path);
        }
        Err(e) => {
            println!(
                "  {} failed: {}",
                ui::theme::paint_error_label(&t, "\u{2717}"),
                e
            );
            let _ = fs::remove_file(&tmp);
        }
    }
}

pub(crate) fn auto_write_files(session: &mut ChatSession, files: &[FileEntry]) {
    let t = ui::theme::active();
    if files.is_empty() || files.iter().all(|f| f.path.is_empty()) {
        println!(
            "{}  Type /write <path> to save.",
            ui::theme::paint_warning(&t, "\u{2502}  !"),
        );
        return;
    }

    let base_dir = session
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let mut safe_entries: Vec<(&FileEntry, PathBuf)> = Vec::new();
    for f in files {
        if f.path.is_empty() {
            continue;
        }
        match resolve_safe_path(&base_dir, &f.path) {
            Some(abs) => safe_entries.push((f, abs)),
            None => {
                eprintln!(
                    "{}   {} {} {}",
                    ui::theme::paint_error_label(&t, "\u{2502} \u{2717}"),
                    ui::theme::paint_bright(&t, &f.path.to_string()),
                    ui::theme::paint_dim(&t, "—"),
                    ui::theme::paint_error_label(&t, "path traversal blocked")
                );
            }
        }
    }

    if safe_entries.is_empty() {
        return;
    }

    println!("{}", ui::theme::paint_rail_empty(&t));
    println!(
        "{} {}",
        ui::theme::paint_rail_empty(&t),
        ui::theme::paint_bright(
            &t,
            &format!("Plan: creating {} file(s)", safe_entries.len())
        ),
    );
    for (f, abs_path) in &safe_entries {
        let icon = file_icon(&f.path);
        let lines = f.content.lines().count();
        let marker = if abs_path.exists() {
            ui::theme::paint_warning(&t, " [EXISTS]")
        } else {
            String::new()
        };
        println!(
            "{}   {} {} ({}, {} lines){}",
            ui::theme::paint_rail_empty(&t),
            icon,
            ui::theme::paint_bright(&t, &f.path.to_string()),
            ui::theme::paint_dim(&t, &format!("{} bytes", f.content.len())),
            ui::theme::paint_dim(&t, &format!("{}", lines)),
            marker
        );
    }
    println!("{}", ui::theme::paint_rail_empty(&t));
    println!(
        "{} {} {}",
        ui::theme::paint(&t, "accent_info", "\u{2502}  ?", true),
        ui::theme::paint_bright(
            &t,
            &format!("Write all {} files? [Y/n]", safe_entries.len())
        ),
        ui::theme::paint_dim(&t, "(press Enter to confirm)")
    );
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent_info", "\u{2502}", true),
        ui::theme::paint_dim(&t, "  Type /code to preview, 'n' to cancel")
    );
    println!("{}", ui::theme::paint_rail_empty(&t));

    let input = session
        .readline("rem> ")
        .unwrap_or_else(|_| String::from("y"));
    let input = input.trim();
    if !input.is_empty() && !input.eq_ignore_ascii_case("y") && !input.eq_ignore_ascii_case("yes") {
        println!(
            "{} skipped. Use /write <path> to save individually.",
            ui::theme::paint_warning(&t, "\u{2502}  !")
        );
        println!("{}", ui::theme::paint_rail_empty(&t));
        return;
    }

    let mut written: Vec<PathBuf> = Vec::new();
    for (f, abs_path) in &safe_entries {
        let will_overwrite = abs_path.exists();
        if will_overwrite {
            println!(
                "{}   {} {}",
                ui::theme::paint_warning(&t, "\u{2502} \u{26a0}"),
                ui::theme::paint_bright(&t, &f.path.to_string()),
                ui::theme::paint_dim(&t, "exists — overwriting"),
            );
        }

        if let Some(parent) = abs_path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = fs::create_dir_all(parent) {
                    eprintln!(
                        "{}   {} cannot create dir {}: {}",
                        ui::theme::paint_error_label(&t, "\u{2502} \u{2717}"),
                        ui::theme::paint_bright(&t, &f.path.to_string()),
                        parent.display(),
                        e
                    );
                    continue;
                }
            }
        }

        let tmp = abs_path.with_extension("tmp");
        match fs::write(&tmp, &f.content) {
            Ok(()) => {
                if let Err(e) = fs::rename(&tmp, abs_path) {
                    eprintln!(
                        "{}   {} atomic write failed: {}",
                        ui::theme::paint_error_label(&t, "\u{2502} \u{2717}"),
                        ui::theme::paint_bright(&t, &f.path.to_string()),
                        e
                    );
                    let _ = fs::remove_file(&tmp);
                    continue;
                }
                let overwrite_note = if will_overwrite { " (overwritten)" } else { "" };
                println!(
                    "{}   {} {} {}",
                    ui::theme::paint_success_label(&t, "\u{2502} \u{2713}"),
                    ui::theme::paint_bright(&t, &f.path.to_string()),
                    ui::theme::paint_dim(&t, &format!("{} bytes", f.content.len())),
                    ui::theme::paint_dim(&t, overwrite_note),
                );
                written.push(abs_path.clone());
            }
            Err(e) => {
                println!(
                    "{}   {} : {}",
                    ui::theme::paint_error_label(&t, "\u{2502} \u{2717}"),
                    ui::theme::paint_bright(&t, &f.path.to_string()),
                    e
                );
                let _ = fs::remove_file(&tmp);
            }
        }
    }

    if !written.is_empty() {
        session.last_files_written = written;
        println!(
            "{} {} files written.",
            ui::theme::paint_success_label(&t, "\u{2502} \u{2713}"),
            ui::theme::paint_bright(&t, &format!("{}", session.last_files_written.len())),
        );
    }
}

pub(crate) fn handle_undo(session: &mut ChatSession) {
    let t = ui::theme::active();
    if session.last_files_written.is_empty() {
        println!("  {} Nothing to undo.", ui::theme::paint_warning(&t, "!"));
        return;
    }
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent_info", "\u{258C}  ?", true),
        ui::theme::paint_bright(
            &t,
            &format!(
                "Delete the last {} written file(s)? [y/N]",
                session.last_files_written.len()
            )
        )
    );

    let input = session.readline("rem> ").unwrap_or_else(|_| String::new());
    let input = input.trim();
    if !input.eq_ignore_ascii_case("y") && !input.eq_ignore_ascii_case("yes") {
        println!("  {} cancelled", ui::theme::paint_rail_empty(&t));
        return;
    }

    let mut removed = 0;
    let mut dirs_to_clean: Vec<PathBuf> = Vec::new();
    for path in session.last_files_written.drain(..) {
        if path.exists() {
            if let Some(parent) = path.parent() {
                dirs_to_clean.push(parent.to_path_buf());
            }
            match fs::remove_file(&path) {
                Ok(()) => {
                    println!(
                        "  {} removed {}",
                        ui::theme::paint_warning(&t, "\u{258C}"),
                        ui::theme::paint_dim(&t, &format!("{}", path.display()))
                    );
                    removed += 1;
                }
                Err(e) => {
                    println!(
                        "  {} failed to remove {}: {}",
                        ui::theme::paint_error_label(&t, "\u{258C}"),
                        path.display(),
                        e
                    );
                }
            }
        }
    }

    dirs_to_clean.sort_by_key(|b| std::cmp::Reverse(b.as_os_str().len()));
    for dir in &dirs_to_clean {
        if dir.exists() {
            let _ = fs::remove_dir(dir);
        }
    }

    if removed > 0 {
        let input = session.last_user_input.clone();
        let intent = session.last_intent.clone();
        if intent == TaskIntent::CodeAction {
            session
                .feedback
                .record_correction(&input, &intent, &TaskIntent::FastAnswer);
        }
        println!(
            "  {} {}  file(s) removed.",
            ui::theme::paint_success_label(&t, "\u{258C} \u{2713}"),
            removed
        );
    }
}

pub(crate) fn handle_list_files(session: &ChatSession) {
    let dir = session
        .project_dir
        .as_ref()
        .cloned()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let t = ui::theme::active();

    println!("{}", ui::theme::paint_rail_empty(&t));
    println!(
        "{} {}",
        ui::theme::paint_rail_empty(&t),
        ui::theme::paint_bright(&t, &format!("\u{1f4c2} project ({})", dir.display()))
    );
    println!("{}", ui::theme::paint_rail_empty(&t));

    let mut entries: Vec<(String, bool, u64)> = Vec::new();
    for entry in WalkDir::new(&dir)
        .max_depth(4)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if p == dir {
            continue;
        }
        if let Ok(rel) = p.strip_prefix(&dir) {
            let size = if p.is_file() {
                fs::metadata(p).map(|m| m.len()).unwrap_or(0)
            } else {
                0
            };
            entries.push((rel.display().to_string(), p.is_dir(), size));
        }
    }
    entries.sort();

    if entries.is_empty() {
        println!(
            "{}   {}",
            ui::theme::paint_rail_empty(&t),
            ui::theme::paint_warning(&t, "(empty)")
        );
    } else {
        for (path, is_dir, size) in &entries {
            let depth = path.chars().filter(|&c| c == '/').count();
            let indent = "  ".repeat(depth);
            let name = if let Some(pos) = path.rfind('/') {
                &path[pos + 1..]
            } else {
                path
            };
            if *is_dir {
                println!(
                    "{} {} {} {} ",
                    ui::theme::paint_rail_empty(&t),
                    indent,
                    ui::theme::paint_dim(&t, "\u{251c}\u{2500}\u{2500}"),
                    ui::theme::paint(&t, "accent_info", &format!("\u{1f4c1} {}/", name), true)
                );
            } else {
                let icon = file_icon(name);
                let hs = human_size(*size);
                println!(
                    "{} {} {} {} {} {}",
                    ui::theme::paint_rail_empty(&t),
                    indent,
                    ui::theme::paint_dim(&t, "\u{251c}\u{2500}\u{2500}"),
                    icon,
                    ui::theme::paint_bright(&t, name),
                    ui::theme::paint_dim(&t, &format!("({})", hs))
                );
            }
        }
    }
    println!("{}", ui::theme::paint_rail_empty(&t));
}

pub(crate) fn print_last_files(session: &ChatSession) {
    let t = ui::theme::active();
    if !session.last_files.is_empty() {
        for f in &session.last_files {
            let label = if f.path.is_empty() {
                "(unnamed)".to_string()
            } else {
                f.path.clone()
            };
            let lang = highlight::detect_language_from_content(&f.content);
            let lang_display = if lang.is_empty() {
                String::new()
            } else {
                format!(" [{}]", lang)
            };
            println!(
                "{}",
                ui::theme::paint_bright(
                    &t,
                    &format!(
                        "\u{2500}\u{2500} {}{} \u{2500}\u{2500}",
                        label,
                        ui::theme::paint_dim(&t, &lang_display)
                    )
                )
            );
            let highlighted = highlight::highlight_code(&f.content, lang);
            for code_line in highlighted.lines() {
                println!("{}", code_line);
            }
            println!("{}", ui::theme::paint_dim(&t, "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"));
        }
    } else if !session.last_code.is_empty() {
        let lang = highlight::detect_language_from_content(&session.last_code);
        let lang_display = if lang.is_empty() {
            String::new()
        } else {
            format!(" [{}]", lang)
        };
        println!(
            "{}",
            ui::theme::paint_bright(
                &t,
                &format!(
                    "\u{2500}\u{2500} last code{} \u{2500}\u{2500}",
                    ui::theme::paint_dim(&t, &lang_display)
                )
            )
        );
        let highlighted = highlight::highlight_code(&session.last_code, lang);
        println!("{}", highlighted);
        println!("{}", ui::theme::paint_dim(&t, "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"));
    } else {
        println!(
            "  {} No code from last response.",
            ui::theme::paint_warning(&t, "!")
        );
    }
}

pub(crate) fn handle_dir(session: &mut ChatSession, path: &str) {
    let t = ui::theme::active();
    let dir = PathBuf::from(path.trim());
    let resolved = if path.trim() == "." {
        std::env::current_dir().unwrap_or_default()
    } else {
        dir
    };
    if resolved.exists() || path.trim() == "." {
        session.project_dir = Some(resolved.clone());
        session.workspace_dir = Some(resolved.clone());
        persist_workspace(&resolved);
        println!(
            "  {} workspace set to {}",
            ui::theme::paint_success_label(&t, "✓"),
            ui::theme::paint_bright(
                &t,
                &session.project_dir.as_ref().unwrap().display().to_string()
            )
        );
    } else {
        println!(
            "  {} directory does not exist — creating it",
            ui::theme::paint_warning(&t, "!")
        );
        if let Err(e) = fs::create_dir_all(&resolved) {
            println!("  {} failed: {}", ui::theme::paint_error_label(&t, "✗"), e);
            return;
        }
        session.project_dir = Some(resolved.clone());
        session.workspace_dir = Some(resolved.clone());
        persist_workspace(&resolved);
        println!(
            "  {} workspace set to {}",
            ui::theme::paint_success_label(&t, "✓"),
            ui::theme::paint_bright(
                &t,
                &session.project_dir.as_ref().unwrap().display().to_string()
            )
        );
    }
}

pub(crate) async fn handle_search(client: &Provider, session: &mut ChatSession, query: &str) {
    let t = ui::theme::active();
    println!(
        "{} {} searching the web...",
        ui::theme::paint_rail_empty(&t),
        ui::theme::paint(&t, "accent", "🔍", true)
    );
    match perform_web_search(&client.client, query).await {
        Ok(results) => {
            if results.is_empty() {
                println!(
                    "{} no results found for: {}",
                    ui::theme::paint_warning(&t, "│"),
                    query
                );
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
            println!(
                "{} {}",
                ui::theme::paint_error_label(&t, "│  search failed:"),
                e
            );
        }
    }
}

pub(crate) async fn handle_explain(client: &Provider, session: &mut ChatSession, text: &str) {
    let t = ui::theme::active();
    if text.trim().is_empty() {
        println!(
            "{} usage: /explain <code snippet>",
            ui::theme::paint_warning(&t, "│")
        );
        return;
    }
    println!(
        "{} explaining...",
        ui::theme::paint(&t, "accent", "\u{258C}", true)
    );
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
            session.history.push((format!("/explain {}", text), response));
        }
        Err(e) => {
            println!("\n{} explain failed: {}", ui::theme::paint_error_label(&t, "│"), e);
        }
    }
}

pub(crate) async fn handle_test(client: &Provider, session: &mut ChatSession, path: &str) {
    let t = ui::theme::active();
    let file_path = Path::new(path.trim());
    if !file_path.exists() {
        println!(
            "{} file not found: {}",
            ui::theme::paint_warning(&t, "│"),
            path
        );
        return;
    }
    let content = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            println!(
                "{} cannot read file: {}",
                ui::theme::paint_error_label(&t, "│"),
                e
            );
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
            session.last_code = extract_code_block(&response);
            session.add_history(&format!("/test {}", path));
            session.history.push((format!("/test {}", path), response));
            if !session.last_code.is_empty() {
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

pub(crate) async fn handle_refactor(client: &Provider, session: &mut ChatSession, path: &str) {
    let t = ui::theme::active();
    let file_path = Path::new(path.trim());
    if !file_path.exists() {
        println!(
            "{} file not found: {}",
            ui::theme::paint_warning(&t, "│"),
            path
        );
        return;
    }
    let content = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            println!(
                "{} cannot read file: {}",
                ui::theme::paint_error_label(&t, "│"),
                e
            );
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
            session.history.push((format!("/refactor {}", path), response));
        }
        Err(e) => {
            println!("\n{} refactor analysis failed: {}", ui::theme::paint_error_label(&t, "│"), e);
        }
    }
}

pub(crate) fn handle_config(session: &ChatSession, client: &Provider) {
    let t = ui::theme::active();
    println!("{}", ui::theme::paint_rail_header(&t, "CONFIG"));
    println!(
        "{}   {:<18} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "provider:"),
        ui::theme::paint_dim(&t, client.kind.as_str())
    );
    println!(
        "{}   {:<18} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "model:"),
        ui::theme::paint_dim(&t, &client.model)
    );
    println!(
        "{}   {:<18} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "base url:"),
        ui::theme::paint_dim(&t, &client.base_url)
    );
    println!(
        "{}   {:<18} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "mode:"),
        ui::theme::paint_dim(&t, session.mode.label())
    );
    println!(
        "{}   {:<18} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "workspace:"),
        ui::theme::paint_dim(
            &t,
            &session
                .project_dir
                .as_ref()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| "none".to_string())
        )
    );
    println!("{}", ui::theme::paint_rail_empty(&t));
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_dim(
            &t,
            "/model <name>  /provider <name>  /config workspace <path>"
        )
    );
    println!("{}", ui::theme::paint(&t, "accent", "\u{258C}", true));
}

pub(crate) fn handle_config_set(session: &mut ChatSession, client: &Provider, args: &str) {
    let t = ui::theme::active();
    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    if parts.is_empty() {
        handle_config(session, client);
        return;
    }
    match parts[0] {
        "workspace" | "dir" => {
            if parts.len() > 1 {
                handle_dir(session, parts[1]);
            } else {
                println!(
                    "{} usage: /config workspace <path>",
                    ui::theme::paint_warning(&t, "\u{258C}")
                );
            }
        }
        other => {
            println!(
                "{} unknown config key: {}",
                ui::theme::paint_warning(&t, "\u{258C}"),
                other
            );
            println!(
                "{} available: model, workspace",
                ui::theme::paint_rail_empty(&t)
            );
        }
    }
}

pub(crate) fn handle_diff(session: &ChatSession) {
    let t = ui::theme::active();
    if session.last_files.is_empty() {
        println!(
            "{} No generated files to compare.",
            ui::theme::paint_warning(&t, "\u{2502}")
        );
        return;
    }

    let base_dir = session
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    println!("{}", ui::theme::paint_dim(&t, "\u{2502}"));
    println!(
        "{} {}",
        ui::theme::paint_rail_empty(&t),
        ui::theme::paint_bright(&t, "--- DIFF ---"),
    );
    println!("{}", ui::theme::paint_dim(&t, "\u{2502}"));

    for f in &session.last_files {
        if f.path.is_empty() {
            continue;
        }
        let rel_path = PathBuf::from(&f.path);
        let abs_path = if rel_path.is_relative() {
            base_dir.join(&rel_path)
        } else {
            rel_path
        };

        let icon = file_icon(&f.path);
        if abs_path.exists() {
            let existing = fs::read_to_string(&abs_path).unwrap_or_default();
            if existing == f.content {
                println!(
                    "{} {} {} {}",
                    ui::theme::paint_rail_empty(&t),
                    icon,
                    ui::theme::paint_bright(&t, &f.path.to_string()),
                    ui::theme::paint_dim(&t, "(unchanged)")
                );
            } else {
                let added = f
                    .content
                    .lines()
                    .count()
                    .saturating_sub(existing.lines().count());
                let removed = existing
                    .lines()
                    .count()
                    .saturating_sub(f.content.lines().count());
                println!(
                    "{} {} {}",
                    ui::theme::paint_rail_empty(&t),
                    icon,
                    ui::theme::paint_bright(&t, &f.path.to_string()),
                );
                if added > 0 {
                    println!(
                        "{}   {}",
                        ui::theme::paint_rail_empty(&t),
                        ui::theme::paint_success_label(&t, &format!("+{} lines", added)),
                    );
                }
                if removed > 0 {
                    println!(
                        "{}   {}",
                        ui::theme::paint_rail_empty(&t),
                        ui::theme::paint_error_label(&t, &format!("-{} lines", removed)),
                    );
                }
                let old_lines: Vec<&str> = existing.lines().collect();
                let new_lines: Vec<&str> = f.content.lines().collect();
                let max_lines = old_lines.len().max(new_lines.len());
                let mut diff_printed = 0;
                for i in 0..max_lines {
                    let old = old_lines.get(i).copied().unwrap_or("");
                    let new = new_lines.get(i).copied().unwrap_or("");
                    if old != new && diff_printed < 8 {
                        if i < old_lines.len() && !old.is_empty() {
                            println!(
                                "{}     {} {}",
                                ui::theme::paint_dim(&t, "\u{2502}"),
                                ui::theme::paint_error_label(&t, "-"),
                                ui::theme::paint_error_label(&t, old)
                            );
                        }
                        if i < new_lines.len() && !new.is_empty() {
                            println!(
                                "{}     {} {}",
                                ui::theme::paint_dim(&t, "\u{2502}"),
                                ui::theme::paint_success_label(&t, "+"),
                                ui::theme::paint_success_label(&t, new)
                            );
                        }
                        diff_printed += 1;
                    }
                }
                if max_lines > 8 && diff_printed > 0 {
                    println!(
                        "{}     {}",
                        ui::theme::paint_dim(&t, "\u{2502}"),
                        ui::theme::paint_dim(&t, "...")
                    );
                }
            }
        } else {
            println!(
                "{} {} {} {}",
                ui::theme::paint_rail_empty(&t),
                icon,
                ui::theme::paint_bright(&t, &f.path.to_string()),
                ui::theme::paint_success_label(
                    &t,
                    &format!("(new file) {} bytes", f.content.len())
                )
            );
        }
    }

    let cmd = std::process::Command::new("git")
        .args(["diff", "--stat", "--"])
        .current_dir(&base_dir)
        .output();

    if let Ok(output) = cmd {
        if !output.stdout.is_empty() {
            println!("{}", ui::theme::paint_rail_empty(&t));
            println!(
                "{} {}",
                ui::theme::paint_rail_empty(&t),
                ui::theme::paint_dim(&t, "git diff --stat:")
            );
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                println!(
                    "{}   {}",
                    ui::theme::paint_rail_empty(&t),
                    ui::theme::paint_dim(&t, line)
                );
            }
        }
    }

    println!("{}", ui::theme::paint_rail_empty(&t));
}

pub(crate) fn handle_tokens(session: &ChatSession) {
    let tokens = session.last_tokens;
    let elapsed = session.last_elapsed.as_secs_f64();
    let history_tokens: usize = session
        .history
        .iter()
        .map(|(u, a)| (u.len() + a.len()) / 4)
        .sum();
    let t = ui::theme::active();

    println!("{}", ui::theme::paint_rail_empty(&t));
    println!(
        "{}  {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "\u{2500}\u{2500} TOKENS \u{2500}\u{2500}"),
    );
    println!(
        "{}   {:<18} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "last response:"),
        ui::theme::paint_dim(&t, &format!("~{} tokens", tokens))
    );

    if elapsed > 0.0 && tokens > 0 {
        let tps = tokens as f64 / elapsed;
        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "speed:"),
            ui::theme::paint_dim(&t, &format!("~{:.0} tok/s", tps))
        );
    }

    if session.last_elapsed.as_secs() > 0 {
        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "elapsed:"),
            ui::theme::paint_dim(&t, &format!("{:.1}s", elapsed))
        );
    }

    if history_tokens > 0 {
        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "context history:"),
            ui::theme::paint_dim(
                &t,
                &format!(
                    "~{} tokens ({} turns)",
                    history_tokens,
                    session.history.len()
                )
            )
        );

        // Display uses the new scaled default (was hardcoded 2048). In future this should come from
        // the active Provider or ChatSession (after we store model_ctx + actual prompt budget).
        let pct = (history_tokens as f64 / 4096.0 * 100.0).min(100.0);
        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "context window:"),
            ui::theme::paint_dim(&t, &format!("{:.0}% used (4096 limit)", pct))
        );
    } else {
        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "context:"),
            ui::theme::paint_dim(&t, "empty (no history)")
        );
    }
    println!("{}", ui::theme::paint_rail_empty(&t));
}

pub(crate) fn handle_memory(session: &ChatSession) {
    let t = ui::theme::active();
    println!("{}", ui::theme::paint_rail_header(&t, "MEMORY"));
    if session.project_memory.loaded && !session.project_memory.content.is_empty() {
        for line in session.project_memory.content.lines() {
            println!(
                "{} {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_dim(&t, line)
            );
        }
    } else {
        println!(
            "{} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_dim(&t, "no project memory yet.")
        );
        println!(
            "{} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_dim(&t, "use /init to generate, or /memory add <text>")
        );
    }
    println!("{}", ui::theme::paint_rail_empty(&t));
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_dim(&t, "/memory add <text>  /init  /memory clear")
    );
    println!("{}", ui::theme::paint(&t, "accent", "\u{258C}", true));
}

pub(crate) fn handle_memory_set(session: &mut ChatSession, args: &str) {
    let t = ui::theme::active();
    if args.eq_ignore_ascii_case("clear") {
        session.project_memory.content.clear();
        session.project_memory.loaded = false;
        let _ = session.project_memory.save();
        println!(
            "{} memory cleared",
            ui::theme::paint_success_label(&t, "\u{2713}")
        );
        return;
    }
    if let Some(text) = args.strip_prefix("add ") {
        if let Err(e) = session.project_memory.append(text) {
            println!(
                "{} failed: {}",
                ui::theme::paint_error_label(&t, "\u{2717}"),
                e
            );
        } else {
            println!(
                "{} appended to memory ({} bytes)",
                ui::theme::paint_success_label(&t, "\u{2713}"),
                text.len()
            );
        }
        return;
    }
    if let Err(e) = session.project_memory.set(args) {
        println!(
            "{} failed: {}",
            ui::theme::paint_error_label(&t, "\u{2717}"),
            e
        );
    } else {
        println!(
            "{} memory saved ({} bytes)",
            ui::theme::paint_success_label(&t, "\u{2713}"),
            args.len()
        );
    }
}

pub(crate) fn handle_init(session: &mut ChatSession) {
    let dir = session
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let ptype = detect_project_type(&dir);
    let ptype_label = if ptype.is_empty() { "unknown" } else { ptype };
    let t = ui::theme::active();
    println!("{}", ui::theme::paint_rail_empty(&t));
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, &format!("detected project type: {}", ptype_label))
    );
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_dim(&t, "generating .rem/memory.md...")
    );
    let starter = ProjectMemory::generate_starter(&dir, ptype);
    if let Err(e) = session.project_memory.set(&starter) {
        println!(
            "{} {} failed: {}",
            ui::theme::paint_error_label(&t, "\u{258C}"),
            ui::theme::paint_error_label(&t, "✗"),
            e
        );
    } else {
        println!(
            "{} {} {} ({} bytes)",
            ui::theme::paint_success_label(&t, "\u{258C}"),
            ui::theme::paint_success_label(&t, "✓"),
            ui::theme::paint_bright(&t, ".rem/memory.md created"),
            starter.len()
        );
        println!(
            "{}  use {} to view",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "/memory")
        );
    }
    println!("{}", ui::theme::paint_rail_empty(&t));
}

pub(crate) async fn handle_compact(client: &Provider, session: &mut ChatSession) {
    let t = ui::theme::active();
    if session.history.is_empty() {
        println!(
            "{} nothing to compact — history is empty",
            ui::theme::paint_warning(&t, "│")
        );
        return;
    }
    let history_text = session.build_chat_history();
    let compact_prompt = format!(
        "[SYSTEM] Summarize this conversation in 3-5 bullet points covering key decisions, code generated, and next actions. Be concise.\n\n{}",
        history_text
    );
    println!(
        "{} compacting {} turns...",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        session.history.len()
    );
    match client
        .complete_chat_stream(
            &compact_prompt,
            "You are a summarizer. Output only bullet-point summary. No preamble, no code.",
            "",
        )
        .await
    {
        Ok(summary) => {
            let old_count = session.history.len();
            session.history.clear();
            session.history.push((
                "[compacted summary]".to_string(),
                summary.trim().to_string(),
            ));
            println!(
                "{} {} {} → {} turns",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_success_label(&t, "✓ compacted:"),
                old_count,
                session.history.len()
            );
        }
        Err(e) => {
            println!(
                "{} {} compact failed: {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_error_label(&t, "✗"),
                e
            );
        }
    }
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

    let max_iter = 10;
    let mut last_tool_output = String::new();
    let mut last_written_files: Vec<String> = Vec::new();

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

        match client
            .complete_chat_stream(&prompt, CHAT_SYSTEM_PROMPT_CODE, "")
            .await
        {
            Ok(text) => {
                let cleaned = text.trim().to_string();
                session
                    .history
                    .push((format!("/goal {}", condition), cleaned.clone()));

                let files = extract_code_blocks_with_names(&cleaned);
                let code = extract_code_block(&cleaned);
                if !files.is_empty() {
                    session.last_files = files.clone();
                    session.last_code = if code.is_empty() { String::new() } else { code };
                    auto_write_files(session, &files);
                    last_written_files = files.iter().map(|f| f.path.clone()).collect();
                } else if !code.is_empty() {
                    session.last_code = code;
                    session.last_files.clear();
                    println!(
                        "{} {} use /write <path> to save",
                        ui::theme::paint(&t, "accent", "\u{258C}", true),
                        ui::theme::paint_dim(&t, "code detected —")
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

                if cleaned.contains("GOAL_ACHIEVED") {
                    println!(
                        "{} {} goal achieved!",
                        ui::theme::paint_success_label(&t, "\u{258C}"),
                        ui::theme::paint_success_label(&t, "\u{2713}")
                    );
                    break;
                }
                if cleaned.contains("GOAL_FAILED") {
                    println!(
                        "{} {} goal could not be achieved.",
                        ui::theme::paint_warning(&t, "\u{258C}"),
                        ui::theme::paint_warning(&t, "!")
                    );
                    break;
                }

                if !last_written_files.is_empty() {
                    let mut tool_results = String::new();
                    for file_path in &last_written_files {
                        let lint_result = run_lint(file_path);
                        println!("{}", format_tool_output(&lint_result));

                        let test_result = run_test(file_path);
                        if !test_result.stderr.is_empty() || !test_result.stdout.is_empty() {
                            println!("{}", format_tool_output(&test_result));
                        }

                        tool_results.push_str(&build_tool_context(
                            Some(&lint_result),
                            Some(&test_result),
                            None,
                        ));
                    }
                    last_tool_output = tool_results;
                }
            }
            Err(e) => {
                println!(
                    "{} {} error: {}",
                    ui::theme::paint_error_label(&t, "\u{258C}"),
                    ui::theme::paint_error_label(&t, "\u{2717}"),
                    e
                );
                break;
            }
        }
    }
    println!("{}", ui::theme::paint_rail_empty(&t));
}

pub(crate) fn handle_copy(session: &ChatSession, n: usize) {
    let t = ui::theme::active();
    let response = if n == 1 || session.history.is_empty() {
        session
            .history
            .last()
            .map(|(_, a)| a.as_str())
            .unwrap_or("")
    } else {
        let total = session.history.len();
        if n > total {
            println!(
                "{} only {} responses in history",
                ui::theme::paint_warning(&t, "\u{258C}"),
                total
            );
            return;
        }
        session
            .history
            .get(total - n)
            .map(|(_, a)| a.as_str())
            .unwrap_or("")
    };

    if response.is_empty() {
        println!(
            "{} nothing to copy",
            ui::theme::paint_warning(&t, "\u{258C}")
        );
        return;
    }

    let use_clipboard = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("printf '%s' {:?} | xclip -selection clipboard 2>/dev/null || printf '%s' {:?} | xsel --clipboard 2>/dev/null || printf '%s' {:?} | pbcopy 2>/dev/null || echo 'no-clipboard'", response, response, response))
        .output();

    match use_clipboard {
        Ok(out) if String::from_utf8_lossy(&out.stdout).contains("no-clipboard") => {
            println!(
                "{} copied to console:",
                ui::theme::paint_success_label(&t, "│ ✓")
            );
            println!("{}", ui::theme::paint_rail_empty(&t));
            for line in response.lines().take(20) {
                println!("{} {}", ui::theme::paint_rail_empty(&t), line);
            }
            if response.lines().count() > 20 {
                println!(
                    "{} ... ({} lines total)",
                    ui::theme::paint_rail_empty(&t),
                    response.lines().count()
                );
            }
        }
        Ok(_) => {
            println!(
                "{} copied to clipboard ({} chars)",
                ui::theme::paint_success_label(&t, "│ ✓"),
                response.len()
            );
        }
        Err(_) => {
            println!(
                "{} copied to console ({}) — install xclip/xsel for clipboard",
                ui::theme::paint_success_label(&t, "│ ✓"),
                response.chars().count()
            );
            for line in response.lines().take(20) {
                println!("{} {}", ui::theme::paint_rail_empty(&t), line);
            }
        }
    }
}

pub(crate) fn handle_lint(_session: &mut ChatSession, path: &str) {
    let t = ui::theme::active();
    let file_path = Path::new(path);
    if !file_path.exists() {
        println!(
            "{} file not found: {}",
            ui::theme::paint_warning(&t, "\u{258C}"),
            path
        );
        return;
    }

    println!(
        "{} linting {}...",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        path
    );
    let result = run_lint(path);
    println!("{}", format_tool_output(&result));
}

pub(crate) async fn handle_review(client: &Provider, session: &mut ChatSession) {
    let t = ui::theme::active();
    if session.last_files.is_empty() {
        println!(
            "{} no generated code to review",
            ui::theme::paint_warning(&t, "│")
        );
        return;
    }

    let mut code_for_review = String::new();
    for f in &session.last_files {
        if f.path.is_empty() {
            continue;
        }
        code_for_review.push_str(&format!(
            "\n### {}\n```\n{}\n```\n",
            f.path,
            truncate_bytes(&f.content, 3000)
        ));
    }
    if code_for_review.is_empty() && !session.last_code.is_empty() {
        code_for_review = format!("```\n{}\n```", truncate_bytes(&session.last_code, 3000));
    }
    if code_for_review.is_empty() {
        println!("{} no code to review", ui::theme::paint_warning(&t, "│"));
        return;
    }

    let review_prompt = format!(
        "Review the following code for:\n\
         1. Bugs & correctness issues\n\
         2. Code smells & anti-patterns\n\
         3. Security vulnerabilities\n\
         4. Missing error handling\n\
         5. Style & naming improvements\n\n\
         Be specific — reference line numbers where possible.\n\n{}",
        code_for_review
    );

    println!(
        "{} reviewing {} file(s)...",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        session.last_files.len()
    );
    match client.complete_chat_stream(
        &review_prompt,
        "[MODE: CHAT] You are a senior code reviewer. Review the code critically. Use clear markdown. Be specific.",
        "",
    ).await {
        Ok(response) => {
            println!();
            println!("{}", response);
            session.history.push(("/review".to_string(), response));
        }
        Err(e) => {
            println!("\n{} review failed: {}", ui::theme::paint_error_label(&t, "│"), e);
        }
    }
}

pub(crate) fn handle_find(session: &ChatSession, query: &str) {
    let t = ui::theme::active();
    if query.is_empty() {
        println!("{}", ui::theme::paint_rail_empty(&t));
        println!(
            "{} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "usage: /find <query>")
        );
        println!("{}", ui::theme::paint_rail_empty(&t));
        return;
    }

    let root = session
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

pub(crate) fn trim_for_display(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('\u{2026}');
        out
    }
}

pub(crate) fn handle_save_session(session: &ChatSession) {
    let t = ui::theme::active();
    let dir = session
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let rem_dir = dir.join(".rem");
    let _ = fs::create_dir_all(&rem_dir);
    let session_file = rem_dir.join("session.json");
    let last_files_json: Vec<serde_json::Value> = session
        .last_files
        .iter()
        .map(|f| serde_json::json!({"path": f.path, "content": f.content}))
        .collect();
    let data = serde_json::json!({
        "history": session.history.iter().map(|(u, a)| serde_json::json!({"user": u, "assistant": a})).collect::<Vec<_>>(),
        "mode": session.mode.label(),
        "workspace": session.project_dir.as_ref().map(|d| d.display().to_string()),
        "saved_at": format_timestamp(),
        "last_code": session.last_code,
        "last_files": last_files_json,
        "last_files_written": session.last_files_written.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    });
    match fs::write(
        &session_file,
        serde_json::to_string_pretty(&data).unwrap_or_default(),
    ) {
        Ok(()) => println!(
            "{} session saved to {}",
            ui::theme::paint_success_label(&t, "\u{2713}"),
            session_file.display()
        ),
        Err(e) => println!(
            "{} failed to save session: {}",
            ui::theme::paint_error_label(&t, "\u{2717}"),
            e
        ),
    }
}

pub(crate) fn handle_resume_session(session: &mut ChatSession) {
    let t = ui::theme::active();
    let dir = session
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let session_file = dir.join(".rem/session.json");
    if !session_file.exists() {
        println!(
            "{} no saved session found at {}",
            ui::theme::paint_warning(&t, "\u{258C}"),
            session_file.display()
        );
        return;
    }
    match fs::read_to_string(&session_file) {
        Ok(content) => {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(history) = data["history"].as_array() {
                    let mut restored = 0;
                    for entry in history {
                        if let (Some(u), Some(a)) =
                            (entry["user"].as_str(), entry["assistant"].as_str())
                        {
                            session.history.push((u.to_string(), a.to_string()));
                            restored += 1;
                        }
                    }
                    println!(
                        "{} restored {} turns from {}",
                        ui::theme::paint_success_label(&t, "\u{2713}"),
                        restored,
                        session_file.display()
                    );
                    println!(
                        "{} current conversation is now merged with saved session",
                        ui::theme::paint_dim(&t, "\u{258C}")
                    );
                }
                if let Some(m) = data["mode"].as_str() {
                    println!(
                        "{} {} {}",
                        ui::theme::paint_dim(&t, "\u{258C}"),
                        ui::theme::paint_dim(&t, "saved mode:"),
                        ui::theme::paint_bright(&t, m)
                    );
                }
                if let Some(code) = data["last_code"].as_str() {
                    if !code.is_empty() {
                        session.last_code = code.to_string();
                        println!(
                            "{} {} {}",
                            ui::theme::paint_dim(&t, "\u{258C}"),
                            ui::theme::paint_dim(&t, "last code:"),
                            ui::theme::paint_success_label(&t, "restored")
                        );
                    }
                }
                if let Some(files) = data["last_files"].as_array() {
                    let restored_files: Vec<FileEntry> = files
                        .iter()
                        .filter_map(|f| {
                            Some(FileEntry {
                                path: f["path"].as_str()?.to_string(),
                                content: f["content"].as_str()?.to_string(),
                            })
                        })
                        .collect();
                    if !restored_files.is_empty() {
                        println!(
                            "{} {} {} file(s) restored",
                            ui::theme::paint_dim(&t, "\u{258C}"),
                            ui::theme::paint_dim(&t, "last files:"),
                            restored_files.len()
                        );
                        session.last_files = restored_files;
                    }
                }
                if let Some(paths) = data["last_files_written"].as_array() {
                    let written: Vec<PathBuf> = paths
                        .iter()
                        .filter_map(|p| p.as_str().map(PathBuf::from))
                        .collect();
                    if !written.is_empty() {
                        session.last_files_written = written;
                    }
                }
            } else {
                println!(
                    "{} invalid session file",
                    ui::theme::paint_error_label(&t, "\u{258C}")
                );
            }
        }
        Err(e) => println!(
            "{} failed to read session: {}",
            ui::theme::paint_error_label(&t, "\u{258C}"),
            e
        ),
    }
}

pub(crate) fn print_chat_help() {
    let t = ui::theme::active();
    println!("{}", ui::theme::paint_rail_empty(&t));
    println!("{}", ui::theme::paint_rail_header(&t, "COMMANDS"));
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/help", "show this help")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/mode", "toggle CHAT → CODE → PLAN")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/plan", "switch to PLAN mode (explore & analyze)")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(
            &t,
            "/model <name>",
            "switch model (e.g. gpt-4, claude-sonnet-4)"
        )
    );
    println!(
        "{}",
        ui::theme::paint_help_line(
            &t,
            "/provider <name>",
            "switch provider: ollama, openai, gemini, anthropic"
        )
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/clear", "reset conversation history")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/explain <code>", "explain what code does")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/test <file>", "generate tests for a file")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/refactor <file>", "suggest refactoring for a file")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/write <path>", "save last code to file")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/save <path>", "same as /write")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/dir <path>", "set project root")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/search <q>", "search the web (DuckDuckGo)")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/code", "show last generated code")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/files", "list project files tree")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/undo", "delete last written files")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/diff", "compare generated vs existing files")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/tokens", "show token usage & context stats")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/config", "view current configuration")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/memory", "view/set project memory (.rem/memory.md)")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/theme [name]", "show or switch color theme")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/init", "auto-generate project memory file")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/compact", "summarize & free context window")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/goal <cond>", "autonomous loop until goal is met")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/copy [N]", "copy last response to clipboard")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/lint [file]", "run linter on generated files")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/review", "AI code review of generated code")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/find <q>", "search text inside the project")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/reset", "full reset — clear history & code cache")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/save", "save current session to .rem/session.json")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/resume", "restore saved session history")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/why", "show why last intent was chosen")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "exit / quit", "exit REM")
    );
    println!("{}", ui::theme::paint_rail_empty(&t));
    println!("{}", ui::theme::paint_rail_header(&t, "TIPS"));
    println!(
        "{}",
        ui::theme::paint_bullet_line(
            &t,
            &[
                ("text_faint", "use ", false),
                ("accent", "@<path>", true),
                (
                    "text_faint",
                    " to include file context: @src/main.rs",
                    false
                ),
            ]
        )
    );
    println!(
        "{}",
        ui::theme::paint_bullet_line(
            &t,
            &[
                ("text_faint", "use ", false),
                ("accent", "/mode", true),
                (
                    "text_faint",
                    " to toggle between chat, code, and plan modes",
                    false
                ),
            ]
        )
    );
    println!(
        "{}",
        ui::theme::paint_bullet_line(
            &t,
            &[
                ("accent", "/plan", true),
                (
                    "text_faint",
                    " for analysis first — REM explores codebase before coding",
                    false
                ),
            ]
        )
    );
    println!(
        "{}",
        ui::theme::paint_rail_bullet(&t, "describe what you want — REM detects intent")
    );
    println!(
        "{}",
        ui::theme::paint_rail_bullet(&t, "multi-file intent and auto-writes after confirmation")
    );
    println!(
        "{}",
        ui::theme::paint_bullet_line(
            &t,
            &[
                ("text_faint", "use ", false),
                ("accent", "/explain", true),
                ("text_faint", " ", false),
                ("accent", "/test", true),
                ("text_faint", " ", false),
                ("accent", "/refactor", true),
                ("text_faint", " for analysis, tests, and refactoring", false),
            ]
        )
    );
    println!(
        "{}",
        ui::theme::paint_bullet_line(
            &t,
            &[
                ("text_faint", "run ", false),
                ("accent", "/init", true),
                (
                    "text_faint",
                    " for persistent project memory across sessions",
                    false
                ),
            ]
        )
    );
    println!(
        "{}",
        ui::theme::paint_bullet_line(
            &t,
            &[
                ("text_faint", "run ", false),
                ("accent", "rem new <name>", true),
                ("text_faint", " to scaffold a new project instantly", false),
            ]
        )
    );
    println!("{}", ui::theme::paint_rail_empty(&t));
}
