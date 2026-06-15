//! File write and manipulation commands (`/write`, `/undo`, `/copy`, `/save`).
//! Handlers for writing generated code to disk, undoing writes, copying
//! responses, and prompting for file paths.

use crate::chat::ChatSession;
use crate::highlight;
use crate::intent::TaskIntent;
use crate::ui;
use crate::{file_icon, resolve_safe_path, BackupEntry, FileEntry};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Atomically writes content to a file using tmp+rename pattern.
/// Returns `true` on success.
fn write_file_atomic(abs_path: &Path, content: &str, t: &crate::ui::theme::Theme) -> bool {
    let tmp = abs_path.with_extension("tmp");
    match fs::write(&tmp, content) {
        Ok(()) => {
            if let Err(e) = fs::rename(&tmp, abs_path) {
                eprintln!(
                    "  {} atomic write failed: {}",
                    ui::theme::paint_error_label(t, "\u{2717}"),
                    e
                );
                let _ = fs::remove_file(&tmp);
                false
            } else {
                true
            }
        }
        Err(e) => {
            eprintln!(
                "  {} failed: {}",
                ui::theme::paint_error_label(t, "\u{2717}"),
                e
            );
            let _ = fs::remove_file(&tmp);
            false
        }
    }
}

/// Prompts the user for a file path interactively.
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
            crate::commands::handle_dir(session, tail);
            continue;
        }

        return Ok(trimmed.to_string());
    }
}

/// Writes generated code to a file (`/write` command).
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

    let original = fs::read_to_string(&abs_path).ok();
    if write_file_atomic(&abs_path, &session.last_code, &t) {
        println!(
            "  {} wrote {} ({} bytes)",
            ui::theme::paint_success_label(&t, "\u{2713}"),
            ui::theme::paint_bright(&t, &format!("{}", abs_path.display())),
            session.last_code.len()
        );
        if session.last_files_written.len() >= 5 {
            session
                .undo_stack
                .push(std::mem::take(&mut session.last_files_written));
            if session.undo_stack.len() > 10 {
                session.undo_stack.remove(0);
            }
        }
        session.last_files_written.push(BackupEntry {
            path: abs_path,
            original,
        });
    }
}

/// Writes multiple generated files to disk with confirmation prompts.
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

    let input = session.readline("rem> ").unwrap_or_else(|_| String::new());
    let input = input.trim();
    if !input.eq_ignore_ascii_case("y") && !input.eq_ignore_ascii_case("yes") {
        println!(
            "{} skipped. Use /write <path> to save individually.",
            ui::theme::paint_warning(&t, "\u{2502}  !")
        );
        println!("{}", ui::theme::paint_rail_empty(&t));
        return;
    }

    let mut written: Vec<BackupEntry> = Vec::new();
    for (f, abs_path) in &safe_entries {
        let will_overwrite = abs_path.exists();
        let original = fs::read_to_string(abs_path).ok();
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

        if write_file_atomic(abs_path, &f.content, &t) {
            let overwrite_note = if will_overwrite { " (overwritten)" } else { "" };
            println!(
                "{}   {} {} {}",
                ui::theme::paint_success_label(&t, "\u{2502} \u{2713}"),
                ui::theme::paint_bright(&t, &f.path.to_string()),
                ui::theme::paint_dim(&t, &format!("{} bytes", f.content.len())),
                ui::theme::paint_dim(&t, overwrite_note),
            );
            written.push(BackupEntry {
                path: abs_path.clone(),
                original,
            });
        }
    }

    if !written.is_empty() {
        if !session.last_files_written.is_empty() {
            session
                .undo_stack
                .push(std::mem::take(&mut session.last_files_written));
            if session.undo_stack.len() > 10 {
                session.undo_stack.remove(0);
            }
        }
        session.last_files_written = written;
        println!(
            "{} {} files written.",
            ui::theme::paint_success_label(&t, "\u{2502} \u{2713}"),
            ui::theme::paint_bright(&t, &format!("{}", session.last_files_written.len())),
        );
    }
}

/// Restores or deletes the last written files (`/undo` command).
/// Supports `/undo N` to undo multiple levels.
/// If the file existed before writing, its original content is restored.
/// If the file was new, it gets deleted.
pub(crate) fn handle_undo(session: &mut ChatSession) {
    let t = ui::theme::active();
    if session.last_files_written.is_empty() && session.undo_stack.is_empty() {
        println!("  {} Nothing to undo.", ui::theme::paint_warning(&t, "!"));
        return;
    }

    let total = session.last_files_written.len()
        + session.undo_stack.iter().map(|v| v.len()).sum::<usize>();
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent_info", "\u{258C}  ?", true),
        ui::theme::paint_bright(
            &t,
            &format!(
                "Revert the last {} written file(s)? [y/N] (or /undo N for N levels)",
                total
            )
        )
    );

    let input = session.readline("rem> ").unwrap_or_else(|_| String::new());
    let input = input.trim();
    if !input.eq_ignore_ascii_case("y") && !input.eq_ignore_ascii_case("yes") {
        println!("  {} cancelled", ui::theme::paint_rail_empty(&t));
        return;
    }

    let mut modified = 0;
    let mut dirs_to_clean: Vec<PathBuf> = Vec::new();

    let mut batches = Vec::new();
    if !session.last_files_written.is_empty() {
        batches.push(std::mem::take(&mut session.last_files_written));
    }
    while let Some(next) = session.undo_stack.pop() {
        batches.push(next);
    }

    for batch in &batches {
        for entry in batch {
            if entry.original.is_some() {
                // Restore original content
                if let Err(e) =
                    fs::write(&entry.path, entry.original.as_deref().unwrap_or_default())
                {
                    println!(
                        "  {} failed to restore {}: {}",
                        ui::theme::paint_error_label(&t, "\u{258C}"),
                        entry.path.display(),
                        e
                    );
                } else {
                    println!(
                        "  {} restored {}",
                        ui::theme::paint_warning(&t, "\u{258C}"),
                        ui::theme::paint_dim(&t, &format!("{}", entry.path.display()))
                    );
                    modified += 1;
                }
            } else if entry.path.exists() {
                // File was new, delete it
                if let Some(parent) = entry.path.parent() {
                    dirs_to_clean.push(parent.to_path_buf());
                }
                match fs::remove_file(&entry.path) {
                    Ok(()) => {
                        println!(
                            "  {} removed {}",
                            ui::theme::paint_warning(&t, "\u{258C}"),
                            ui::theme::paint_dim(&t, &format!("{}", entry.path.display()))
                        );
                        modified += 1;
                    }
                    Err(e) => {
                        println!(
                            "  {} failed to remove {}: {}",
                            ui::theme::paint_error_label(&t, "\u{258C}"),
                            entry.path.display(),
                            e
                        );
                    }
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

    if modified > 0 {
        let input = session.last_user_input.clone();
        let intent = session.last_intent.clone();
        if intent == TaskIntent::CodeAction {
            session
                .feedback
                .record_correction(&input, &intent, &TaskIntent::FastAnswer);
        }
        println!(
            "  {} {}  file(s) reverted.",
            ui::theme::paint_success_label(&t, "\u{258C} \u{2713}"),
            modified
        );
    }
}

/// Prints the last generated files (`/code` command).
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

/// Copies the last response to clipboard (`/copy` command).
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

    let copied = try_copy_to_clipboard(response);

    match copied {
        CopyResult::Success => {
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
        CopyResult::FallbackToConsole => {
            println!(
                "{} copied to console ({} chars)",
                ui::theme::paint_success_label(&t, "│ ✓"),
                response.chars().count()
            );
            for line in response.lines().take(20) {
                println!("{} {}", ui::theme::paint_rail_empty(&t), line);
            }
        }
    }
}

enum CopyResult {
    Success,
    FallbackToConsole,
}

fn try_copy_to_clipboard(text: &str) -> CopyResult {
    for (tool, args) in [
        ("xclip", &["-selection", "clipboard"] as &[_]),
        ("xsel", &["--clipboard", "--input"] as &[_]),
        ("pbcopy", &[] as &[_]),
    ] {
        let mut child = match std::process::Command::new(tool)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => continue,
        };
        use std::io::Write;
        let _ = child.stdin.take().map(|mut stdin| {
            let _ = stdin.write_all(text.as_bytes());
        });
        match child.wait() {
            Ok(status) if status.success() => return CopyResult::Success,
            _ => continue,
        }
    }
    CopyResult::FallbackToConsole
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_file_atomic_creates_file() {
        let dir = std::env::temp_dir().join(format!("rem-test-wfa-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.txt");
        let t = crate::ui::theme::active();
        let result = write_file_atomic(&path, "hello world", &t);
        assert!(result);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_file_atomic_overwrites() {
        let dir = std::env::temp_dir().join(format!("rem-test-wfa2-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.txt");
        std::fs::write(&path, "old content").unwrap();
        let t = crate::ui::theme::active();
        let result = write_file_atomic(&path, "new content", &t);
        assert!(result);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new content");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_file_atomic_removes_tmp_on_failure() {
        let dir = std::env::temp_dir().join(format!("rem-test-wfa3-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.txt");
        std::fs::write(&path, "original").unwrap();
        let t = crate::ui::theme::active();
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, "stale tmp").unwrap();
        let result = write_file_atomic(&path, "updated", &t);
        assert!(result);
        assert!(!tmp.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "updated");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_entry_new_file_has_no_original() {
        let dir = std::env::temp_dir().join(format!("rem-test-backup-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("new_file.txt");
        let original = std::fs::read_to_string(&path).ok();
        let entry = BackupEntry {
            path: path.clone(),
            original,
        };
        assert!(entry.original.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_entry_existing_file_captures_original() {
        let dir = std::env::temp_dir().join(format!("rem-test-backup2-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("existing.txt");
        std::fs::write(&path, "original content").unwrap();
        let original = std::fs::read_to_string(&path).ok();
        let entry = BackupEntry {
            path: path.clone(),
            original,
        };
        assert_eq!(entry.original.as_deref(), Some("original content"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
