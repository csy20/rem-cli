//! Review and diff commands (`/diff`, `/review`).
//! Handlers for comparing generated code against existing files and
//! requesting AI-powered code reviews.

use crate::chat::ChatSession;
use crate::provider::Provider;
use crate::text_util::truncate_bytes;
use crate::types::{file_icon, BackupEntry};
use crate::ui;
use similar::{ChangeTag, TextDiff};
use std::fs;
use std::path::PathBuf;

pub(crate) fn handle_diff(session: &ChatSession) {
    let t = ui::theme::active();
    if session.code_out.last_files.is_empty() {
        println!(
            "{} No generated files to compare.",
            ui::theme::paint_warning(&t, "\u{2502}")
        );
        return;
    }

    let base_dir = session
        .ctx
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

    for f in &session.code_out.last_files {
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
                let added = f.content.lines().count().saturating_sub(existing.lines().count());
                let removed = existing.lines().count().saturating_sub(f.content.lines().count());
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
                let diff = TextDiff::from_lines(&existing, &f.content);
                let mut diff_printed = 0;
                let total = diff.iter_all_changes().count();
                for change in diff.iter_all_changes() {
                    if diff_printed >= 8 {
                        break;
                    }
                    match change.tag() {
                        ChangeTag::Delete => {
                            println!(
                                "{}     {} {}",
                                ui::theme::paint_dim(&t, "\u{2502}"),
                                ui::theme::paint_error_label(&t, "-"),
                                ui::theme::paint_error_label(&t, change.value().trim_end())
                            );
                            diff_printed += 1;
                        }
                        ChangeTag::Insert => {
                            println!(
                                "{}     {} {}",
                                ui::theme::paint_dim(&t, "\u{2502}"),
                                ui::theme::paint_success_label(&t, "+"),
                                ui::theme::paint_success_label(&t, change.value().trim_end())
                            );
                            diff_printed += 1;
                        }
                        ChangeTag::Equal => {}
                    }
                }
                if total > 8 && diff_printed > 0 {
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
                ui::theme::paint_success_label(&t, &format!("(new file) {} bytes", f.content.len()))
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

/// Applies the last diff (writes all changed files) with automatic backup for undo.
pub(crate) fn handle_apply(session: &mut ChatSession) {
    let t = ui::theme::active();
    if session.code_out.last_files.is_empty() {
        println!(
            "{} No generated files to apply.",
            ui::theme::paint_warning(&t, "\u{2502}")
        );
        return;
    }

    let base_dir = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let mut backup_entries: Vec<BackupEntry> = Vec::new();
    let mut applied = 0u32;

    for f in &session.code_out.last_files {
        if f.path.is_empty() {
            continue;
        }
        let rel_path = PathBuf::from(&f.path);
        let abs_path = if rel_path.is_relative() {
            base_dir.join(&rel_path)
        } else {
            rel_path
        };

        // Capture original content for undo
        let original = fs::read_to_string(&abs_path).ok();
        backup_entries.push(BackupEntry {
            path: abs_path.clone(),
            original,
        });

        // Write the file
        if let Some(parent) = abs_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match fs::write(&abs_path, &f.content) {
            Ok(()) => {
                let icon = file_icon(&f.path);
                let path_display = ui::theme::paint_bright(&t, &f.path);
                println!(
                    "  {} {} {} applied ({} bytes)",
                    icon,
                    path_display,
                    ui::theme::paint_success_label(&t, "\u{2713}"),
                    f.content.len()
                );
                applied += 1;
            }
            Err(e) => {
                println!(
                    "  {} {} {} failed: {}",
                    ui::theme::paint_warning(&t, "\u{2717}"),
                    ui::theme::paint_bright(&t, &f.path),
                    ui::theme::paint_error_label(&t, "error"),
                    e
                );
            }
        }
    }

    if !backup_entries.is_empty() {
        session.code_out.push_undo(backup_entries);
    }

    println!("{}", ui::theme::paint_rail_empty(&t));
    if applied > 0 {
        println!(
            "  {} {} file(s) applied \u{2014} use /undo to revert",
            ui::theme::paint_success_label(&t, "\u{2713}"),
            applied
        );
    }
    println!("{}", ui::theme::paint_rail_empty(&t));
}

pub(crate) async fn handle_review(client: &Provider, session: &mut ChatSession) {
    let t = ui::theme::active();
    if session.code_out.last_files.is_empty() {
        println!("{} no generated code to review", ui::theme::paint_warning(&t, "│"));
        return;
    }

    let mut code_for_review = String::new();
    for f in &session.code_out.last_files {
        if f.path.is_empty() {
            continue;
        }
        code_for_review.push_str(&format!(
            "\n### {}\n```\n{}\n```\n",
            f.path,
            truncate_bytes(&f.content, 3000)
        ));
    }
    if code_for_review.is_empty() && !session.code_out.last_code.is_empty() {
        code_for_review = format!("```\n{}\n```", truncate_bytes(&session.code_out.last_code, 3000));
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
        session.code_out.last_files.len()
    );
    match client
        .complete_chat_stream(
            &review_prompt,
            "[MODE: CHAT] You are a senior code reviewer. Review the code critically. Use clear markdown. Be specific.",
            "",
        )
        .await
    {
        Ok(response) => {
            println!();
            println!("{}", response);
            session.history_mgr.history.push(("/review".to_string(), response));
        }
        Err(e) => {
            println!("\n{} review failed: {}", ui::theme::paint_error_label(&t, "│"), e);
        }
    }
}
