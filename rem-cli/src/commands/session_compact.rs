use crate::chat::ChatSession;
use crate::provider::Provider;
use crate::ui;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::{Read, Write};

/// Dry-run preview of compaction (`/compact-dry-run`).
pub(crate) fn handle_compact_dry_run(session: &ChatSession) {
    let t = ui::theme::active();
    if session.history_mgr.history.is_empty() {
        println!(
            "{} nothing to compact — history is empty",
            ui::theme::paint_warning(&t, "\u{258C}")
        );
        return;
    }
    println!("{}", ui::theme::paint_rail_header(&t, "COMPACT DRY-RUN"));
    let turn_count = session.history_mgr.history.len();
    let total_chars: usize = session.history_mgr.history.iter().map(|(u, a)| u.len() + a.len()).sum();
    println!(
        "{}   turns: {} | total size: {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, &turn_count.to_string()),
        ui::theme::paint_dim(&t, &crate::text_util::human_size(total_chars as u64))
    );
    println!("{}", ui::theme::paint_rail_empty(&t));
    for (i, (user, assistant)) in session.history_mgr.history.iter().enumerate() {
        let user_preview = user.lines().next().unwrap_or(user);
        let user_preview = if user_preview.len() > 60 {
            format!("{}...", &user_preview[..57])
        } else {
            user_preview.to_string()
        };
        let assistant_preview = assistant.lines().next().unwrap_or(assistant);
        let assistant_preview = if assistant_preview.len() > 60 {
            format!("{}...", &assistant_preview[..57])
        } else {
            assistant_preview.to_string()
        };
        println!(
            "{}   {} user: {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_dim(&t, &format!("#{}", i + 1)),
            ui::theme::paint_bright(&t, &user_preview)
        );
        println!(
            "{}   {} assistant: {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_dim(&t, "   "),
            ui::theme::paint_dim(&t, &assistant_preview)
        );
        println!("{}", ui::theme::paint_rail_empty(&t));
    }
    println!(
        "{}   {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_dim(&t, "LLM was NOT called — run /compact to actually compact")
    );
    println!("{}", ui::theme::paint_rail_empty(&t));
}

/// Compacts conversation history via LLM summarization (`/compact`).
pub(crate) async fn handle_compact(client: &Provider, session: &mut ChatSession) {
    let t = ui::theme::active();
    if session.history_mgr.history.is_empty() {
        println!(
            "{} nothing to compact — history is empty",
            ui::theme::paint_warning(&t, "\u{258C}")
        );
        return;
    }
    match session.readline(&format!(
        "{} Are you sure? [y/N] ",
        ui::theme::paint_warning(&t, "\u{258C}")
    )) {
        Ok(line) => {
            let trimmed = line.trim().to_lowercase();
            if trimmed != "y" && trimmed != "yes" {
                println!(
                    "{} {}",
                    ui::theme::paint(&t, "accent", "\u{258C}", true),
                    ui::theme::paint_dim(&t, "compact cancelled")
                );
                return;
            }
        }
        Err(_) => {
            println!(
                "{} {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_dim(&t, "compact cancelled")
            );
            return;
        }
    }
    let history_text = session.build_chat_history();
    let compact_prompt = format!(
        "[SYSTEM] Summarize this conversation in 3-5 bullet points covering key decisions, code generated, and next actions. Be concise.\n\n{}",
        history_text
    );
    println!(
        "{} compacting {} turns...",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        session.history_mgr.history.len()
    );
    let saved_history = std::mem::take(&mut session.history_mgr.history);
    let dir = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let backup_path = dir.join(".rem/compact_backup.json.gz");
    if let Err(e) = std::fs::create_dir_all(dir.join(".rem")) {
        tracing::warn!("failed to create .rem directory for compact backup: {}", e);
    }
    if let Ok(json) = serde_json::to_string_pretty(&saved_history) {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        if encoder.write_all(json.as_bytes()).is_ok() {
            if let Ok(compressed) = encoder.finish() {
                if let Err(e) = std::fs::write(&backup_path, &compressed) {
                    tracing::warn!("failed to write compact backup: {}", e);
                }
            }
        }
    }
    match client
        .complete_chat_stream(
            &compact_prompt,
            "You are a summarizer. Output only bullet-point summary. No preamble, no code.",
            "",
        )
        .await
    {
        Ok(summary) => {
            let old_count = session.history_mgr.history.len();
            session.history_mgr.history.clear();
            session
                .history_mgr
                .history
                .push(("[compacted summary]".to_string(), summary.trim().to_string()));
            println!(
                "{} {} {} → {} turns",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_success_label(&t, "✓ compacted:"),
                old_count,
                session.history_mgr.history.len()
            );
            println!(
                "{} {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_dim(&t, "use /session compact-undo to restore original history")
            );
        }
        Err(e) => {
            session.history_mgr.history = saved_history;
            println!(
                "{} {} compact failed: {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_error_label(&t, "✗"),
                e
            );
        }
    }
}

/// Restores history from the compact backup (`/session compact-undo`).
pub(crate) fn handle_compact_undo(session: &mut ChatSession) {
    let t = ui::theme::active();
    let dir = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let backup_path = dir.join(".rem/compact_backup.json.gz");
    if !backup_path.exists() {
        println!(
            "{} {}",
            ui::theme::paint_warning(&t, "\u{258C}"),
            ui::theme::paint_dim(&t, "no compact backup found")
        );
        return;
    }
    match read_maybe_gzip(&backup_path) {
        Ok(content) => {
            if let Ok(history) = serde_json::from_str::<Vec<(String, String)>>(&content) {
                session.history_mgr.history = history;
                println!(
                    "{} {}",
                    ui::theme::paint_success_label(&t, "\u{2713}"),
                    ui::theme::paint_dim(&t, "history restored from compact backup")
                );
                if let Err(e) = std::fs::remove_file(&backup_path) {
                    tracing::warn!("failed to remove compact backup after undo: {}", e);
                }
            } else {
                println!(
                    "{} {}",
                    ui::theme::paint_error_label(&t, "\u{2717}"),
                    ui::theme::paint(&t, "error", "invalid backup format", false)
                );
            }
        }
        Err(e) => {
            println!(
                "{} {}",
                ui::theme::paint_error_label(&t, "\u{2717}"),
                ui::theme::paint(&t, "error", &format!("failed to read backup: {e}"), false)
            );
        }
    }
}

/// Reads a file that may be gzip-compressed or plain text.
pub(crate) fn read_maybe_gzip(path: &std::path::Path) -> std::io::Result<String> {
    let raw = std::fs::read(path)?;
    if raw.first() == Some(&0x1f) && raw.get(1) == Some(&0x8b) {
        let mut decoder = GzDecoder::new(&raw[..]);
        let mut out = String::new();
        decoder.read_to_string(&mut out)?;
        if out.len() > 100_000_000 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "decompressed data exceeds size limit",
            ));
        }
        Ok(out)
    } else {
        String::from_utf8(raw).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}
