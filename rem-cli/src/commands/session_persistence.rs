use crate::chat::ChatSession;
use crate::provider::Provider;
use crate::text_util::human_size;
use crate::types::{file_icon, BackupEntry, FileEntry};
use crate::ui;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::{Read, Write};
use std::path::Path;

/// Saves the current session to `.rem/session.json.gz` (`/save` command).
pub(crate) fn handle_save_session(session: &ChatSession) {
    let t = ui::theme::active();
    let dir = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let session_file = dir.join(".rem/session.json.gz");
    if let Err(e) = std::fs::create_dir_all(dir.join(".rem")) {
        tracing::warn!("failed to create .rem dir for session save: {}", e);
    }
    let json = match serde_json::to_string_pretty(&session.to_session_json()) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("session serialization failed: {}", e);
            println!(
                "{} {}",
                ui::theme::paint_error_label(&t, "\u{2717}"),
                ui::theme::paint(&t, "error", "failed to serialize session", false)
            );
            return;
        }
    };
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    if let Err(e) = encoder.write_all(json.as_bytes()) {
        tracing::warn!("gzip write failed: {}", e);
    }
    let compressed = match encoder.finish() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("gzip finish failed: {}", e);
            return;
        }
    };
    match std::fs::write(&session_file, &compressed) {
        Ok(()) => println!(
            "{} session saved to {} ({} bytes)",
            ui::theme::paint_success_label(&t, "\u{2713}"),
            session_file.display(),
            compressed.len(),
        ),
        Err(e) => println!(
            "{} failed to save session: {}",
            ui::theme::paint_error_label(&t, "\u{2717}"),
            e
        ),
    }
}

/// Restores a saved session from `.rem/session.json.gz` (`/resume`).
pub(crate) fn handle_resume_session(session: &mut ChatSession) {
    let t = ui::theme::active();
    let dir = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let session_file = dir.join(".rem/session.json.gz");
    if !session_file.exists() {
        println!(
            "{} no saved session found at {}",
            ui::theme::paint_warning(&t, "\u{258C}"),
            session_file.display()
        );
        return;
    }
    match super::session_compact::read_maybe_gzip(&session_file) {
        Ok(content) => {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(history) = data["history"].as_array() {
                    let mut restored = 0;
                    for entry in history {
                        if let (Some(u), Some(a)) = (entry["user"].as_str(), entry["assistant"].as_str()) {
                            session.history_mgr.push_turn(u.to_string(), a.to_string());
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
                    session.mode = match m.to_uppercase().as_str() {
                        "CODE" => crate::chat::RunMode::Code,
                        "PLAN" => crate::chat::RunMode::Plan,
                        _ => crate::chat::RunMode::Chat,
                    };
                    println!(
                        "{} {} {}",
                        ui::theme::paint_dim(&t, "\u{258C}"),
                        ui::theme::paint_dim(&t, "restored mode:"),
                        ui::theme::paint_bright(&t, session.mode.label())
                    );
                }
                if let Some(code) = data["last_code"].as_str() {
                    if !code.is_empty() {
                        session.code_out.last_code = code.to_string();
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
                        session.code_out.last_files = restored_files;
                    }
                }
                if let Some(paths) = data["last_files_written"].as_array() {
                    let base = session.ctx.project_dir.as_deref().unwrap_or_else(|| Path::new("."));
                    let written: Vec<BackupEntry> = paths
                        .iter()
                        .filter_map(|p| {
                            p.as_str().and_then(|s| {
                                crate::types::resolve_safe_path(base, s).map(|path| {
                                    let original = std::fs::read_to_string(&path).ok();
                                    BackupEntry { path, original }
                                })
                            })
                        })
                        .collect();
                    if !written.is_empty() {
                        session.code_out.last_files_written = written;
                    }
                }
            } else {
                println!("{} invalid session file", ui::theme::paint_error_label(&t, "\u{258C}"));
            }
        }
        Err(e) => println!(
            "{} failed to read session: {}",
            ui::theme::paint_error_label(&t, "\u{258C}"),
            e
        ),
    }
}

/// Exports the current session to a file (`/session export <path>`).
pub(crate) fn handle_export_session(session: &ChatSession, path: &str) {
    let t = ui::theme::active();
    let base = session
        .ctx
        .project_dir
        .as_deref()
        .unwrap_or_else(|| std::path::Path::new("."));
    let out_path = match crate::types::resolve_safe_path(base, path) {
        Some(p) => p,
        None => {
            println!(
                "  {} path traversal blocked",
                ui::theme::paint_error_label(&t, "\u{2717}")
            );
            return;
        }
    };
    let json = match serde_json::to_string_pretty(&session.to_session_json()) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("session serialization failed: {}", e);
            println!(
                "{} {}",
                ui::theme::paint_error_label(&t, "\u{2717}"),
                ui::theme::paint(&t, "error", "failed to serialize session", false)
            );
            return;
        }
    };
    let is_gzip = out_path.extension().map(|e| e == "gz").unwrap_or(false);
    let data: Vec<u8> = if is_gzip {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        if let Err(e) = encoder.write_all(json.as_bytes()) {
            println!(
                "{} compression failed: {}",
                ui::theme::paint_error_label(&t, "\u{2717}"),
                e
            );
            return;
        }
        match encoder.finish() {
            Ok(c) => c,
            Err(e) => {
                println!(
                    "{} compression failed: {}",
                    ui::theme::paint_error_label(&t, "\u{2717}"),
                    e
                );
                return;
            }
        }
    } else {
        json.into_bytes()
    };
    match std::fs::write(&out_path, &data) {
        Ok(()) => println!(
            "{} session exported to {} ({} bytes)",
            ui::theme::paint_success_label(&t, "\u{2713}"),
            out_path.display(),
            data.len(),
        ),
        Err(e) => println!(
            "{} failed to export session: {}",
            ui::theme::paint_error_label(&t, "\u{2717}"),
            e
        ),
    }
}

/// Generates and displays a session summary via the LLM (`/summary` command).
pub(crate) async fn handle_summary(client: &Provider, session: &mut ChatSession, save_path: Option<&str>) {
    let t = ui::theme::active();
    if session.history_mgr.history.is_empty() {
        println!(
            "{} {}",
            ui::theme::paint_warning(&t, "\u{258C}"),
            ui::theme::paint_dim(&t, "nothing to summarize \u{2014} history is empty")
        );
        return;
    }
    let history_text = session.build_chat_history();
    let summary_prompt = format!(
        "[SYSTEM] Summarize this conversation concisely covering: key decisions made, \
         code/files generated, bugs fixed, commands used, and next actions. \
         Use bullet points. Be specific about file paths and changes.\n\n{}",
        history_text
    );
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_dim(&t, "generating session summary...")
    );
    match client
        .complete_chat_stream(
            &summary_prompt,
            "You are a summarizer. Output only a concise bullet-point summary. No preamble, no code fences.",
            "",
        )
        .await
    {
        Ok(summary) => {
            let summary = summary.trim().to_string();
            println!("{}", ui::theme::paint_rail_header(&t, "SESSION SUMMARY"));
            for line in summary.lines() {
                println!(
                    "{} {}",
                    ui::theme::paint(&t, "accent", "\u{258C}", true),
                    ui::theme::paint_dim(&t, line)
                );
            }
            println!("{}", ui::theme::paint_rail_empty(&t));

            if let Some(path) = save_path {
                let base = session
                    .ctx
                    .project_dir
                    .clone()
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                let out_path = match crate::types::resolve_safe_path(&base, path.trim()) {
                    Some(p) => p,
                    None => {
                        println!(
                            "{} {}",
                            ui::theme::paint_error_label(&t, "\u{2717}"),
                            ui::theme::paint(&t, "error", "invalid path", false)
                        );
                        println!("{}", ui::theme::paint_rail_empty(&t));
                        return;
                    }
                };
                match std::fs::write(&out_path, &summary) {
                    Ok(()) => println!(
                        "{} {} {}",
                        ui::theme::paint_success_label(&t, "\u{2713}"),
                        ui::theme::paint_dim(&t, "summary saved to"),
                        ui::theme::paint_bright(&t, &out_path.display().to_string())
                    ),
                    Err(e) => println!(
                        "{} {}",
                        ui::theme::paint_error_label(&t, "\u{2717}"),
                        ui::theme::paint(&t, "error", &format!("failed to save summary: {e}"), false)
                    ),
                }
                println!("{}", ui::theme::paint_rail_empty(&t));
            } else {
                println!(
                    "{} {}",
                    ui::theme::paint(&t, "accent", "\u{258C}", true),
                    ui::theme::paint_dim(&t, "use /summary <path> to save to a file")
                );
                println!("{}", ui::theme::paint(&t, "accent", "\u{258C}", true));
            }
        }
        Err(e) => {
            println!(
                "{} {}",
                ui::theme::paint_error_label(&t, "\u{2717}"),
                ui::theme::paint(&t, "error", &format!("summary failed: {e}"), false)
            );
            println!("{}", ui::theme::paint_rail_empty(&t));
        }
    }
}

/// Removes session files older than MAX_SESSION_AGE_DAYS.
fn cleanup_stale_sessions(session: &ChatSession) {
    let dir = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let rem_dir = dir.join(".rem");
    if !rem_dir.exists() {
        return;
    }
    let max_age = std::time::Duration::from_secs(crate::constants::MAX_SESSION_AGE_DAYS * 86400);
    let now = std::time::SystemTime::now();
    let mut removed = 0usize;
    if let Ok(entries) = std::fs::read_dir(&rem_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("gz") {
                continue;
            }
            if !path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.ends_with(".json"))
            {
                continue;
            }
            if let Ok(metadata) = entry.metadata() {
                if let Ok(modified) = metadata.modified() {
                    if now.duration_since(modified).unwrap_or_default() > max_age {
                        if let Err(e) = std::fs::remove_file(&path) {
                            tracing::warn!("failed to remove stale session {}: {}", path.display(), e);
                        }
                        removed += 1;
                    }
                }
            }
        }
    }
    if removed > 0 {
        let t = crate::ui::theme::active();
        eprintln!(
            "  {} auto-cleaned {} stale session file(s)",
            crate::ui::theme::paint_dim(&t, "\u{258C}"),
            removed
        );
    }
}

/// Lists saved session files in `.rem/` (`/session list`).
pub(crate) fn handle_list_sessions(session: &ChatSession) {
    cleanup_stale_sessions(session);
    let t = ui::theme::active();
    let dir = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let rem_dir = dir.join(".rem");
    if !rem_dir.exists() {
        println!(
            "{} {}",
            ui::theme::paint_warning(&t, "\u{258C}"),
            ui::theme::paint_dim(&t, "no .rem directory found — no saved sessions")
        );
        return;
    }
    #[derive(Default)]
    struct SessionInfo {
        name: String,
        size: u64,
        saved_at: Option<String>,
        turn_count: Option<usize>,
        total_tokens: Option<u64>,
        duration_secs: Option<f64>,
    }
    let mut sessions: Vec<SessionInfo> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&rem_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("gz")
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.ends_with(".json"))
            {
                let name = entry.file_name().to_string_lossy().to_string();
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let mut info = SessionInfo {
                    name,
                    size,
                    ..Default::default()
                };
                if let Ok(raw) = std::fs::read(&path) {
                    if raw.first() == Some(&0x1f) && raw.get(1) == Some(&0x8b) {
                        let mut decoder = GzDecoder::new(&raw[..]);
                        let mut out = String::new();
                        if decoder.read_to_string(&mut out).is_ok() {
                            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&out) {
                                info.saved_at = data["saved_at"].as_str().map(String::from);
                                info.turn_count = data["history"].as_array().map(|a| a.len());
                                info.total_tokens = data["total_tokens"].as_u64();
                                info.duration_secs = data["session_duration_secs"].as_f64();
                            }
                        }
                    }
                }
                sessions.push(info);
            }
        }
    }
    if sessions.is_empty() {
        println!(
            "{} {}",
            ui::theme::paint_warning(&t, "\u{258C}"),
            ui::theme::paint_dim(&t, "no session files found in .rem/")
        );
        return;
    }
    sessions.sort_by(|a, b| b.size.cmp(&a.size));
    println!("{}", ui::theme::paint_rail_header(&t, "SAVED SESSIONS"));
    for info in &sessions {
        let icon = file_icon(&info.name);
        print!(
            "{} {} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            icon,
            ui::theme::paint_bright(&t, &info.name),
        );
        if let Some(turns) = info.turn_count {
            print!(" {}", ui::theme::paint_dim(&t, &format!("{} turns", turns)));
        }
        if let Some(tokens) = info.total_tokens {
            print!(" {}", ui::theme::paint_dim(&t, &format!("{} tok", tokens)));
        }
        if let Some(dur) = info.duration_secs {
            if dur > 60.0 {
                print!(" {}", ui::theme::paint_dim(&t, &format!("{:.0}m", dur / 60.0)));
            } else {
                print!(" {}", ui::theme::paint_dim(&t, &format!("{:.0}s", dur)));
            }
        }
        if let Some(ref ts) = info.saved_at {
            print!(" {}", ui::theme::paint_dim(&t, &format!("@{}", ts)));
        }
        println!(" {}", ui::theme::paint_dim(&t, &format!("({})", human_size(info.size))));
    }
    println!("{}", ui::theme::paint_rail_empty(&t));
}

/// Exports session analytics as JSON (`/session analytics [path]`).
pub(crate) fn handle_session_analytics(session: &ChatSession, client: &Provider, path: Option<&str>) {
    let t = ui::theme::active();
    let analytics = serde_json::json!({
        "provider": client.kind.as_str(),
        "model": session.model,
        "turn_count": session.history_mgr.history.len(),
        "total_tokens": session.total_tokens,
        "duration_secs": session.session_start.elapsed().as_secs_f64(),
        "mode": session.mode.label(),
        "tokens_per_turn": if session.history_mgr.history.is_empty() {
            0.0
        } else {
            session.total_tokens as f64 / session.history_mgr.history.len() as f64
        },
    });
    let json = match serde_json::to_string_pretty(&analytics) {
        Ok(j) => j,
        Err(e) => {
            println!(
                "{} serialization failed: {}",
                ui::theme::paint_error_label(&t, "\u{2717}"),
                e
            );
            return;
        }
    };
    match path {
        Some(p) => {
            let base = session
                .ctx
                .project_dir
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new("."));
            let out_path = match crate::types::resolve_safe_path(base, p) {
                Some(p) => p,
                None => {
                    println!(
                        "  {} path traversal blocked",
                        ui::theme::paint_error_label(&t, "\u{2717}")
                    );
                    return;
                }
            };
            match std::fs::write(&out_path, &json) {
                Ok(()) => println!(
                    "{} analytics exported to {}",
                    ui::theme::paint_success_label(&t, "\u{2713}"),
                    out_path.display()
                ),
                Err(e) => println!("{} write failed: {}", ui::theme::paint_error_label(&t, "\u{2717}"), e),
            }
        }
        None => {
            println!("{}", analytics);
        }
    }
}

/// Exports the current session as human-readable Markdown (`/session export-md <path>`).
pub(crate) fn handle_export_session_md(session: &ChatSession, path: &str) {
    let t = ui::theme::active();
    let base = session
        .ctx
        .project_dir
        .as_deref()
        .unwrap_or_else(|| std::path::Path::new("."));
    let out_path = match crate::types::resolve_safe_path(base, path) {
        Some(p) => p,
        None => {
            println!(
                "  {} path traversal blocked",
                ui::theme::paint_error_label(&t, "\u{2717}")
            );
            return;
        }
    };
    let json = session.to_session_json();
    let mut md = String::new();
    md.push_str("# Session Export\n\n");
    md.push_str(&format!(
        "- **Date:** {}\n",
        json["saved_at"].as_str().unwrap_or("unknown")
    ));
    md.push_str(&format!("- **Mode:** {}\n", json["mode"].as_str().unwrap_or("CHAT")));
    if let Some(ws) = json["workspace"].as_str() {
        md.push_str(&format!("- **Workspace:** `{}`\n", ws));
    }
    md.push_str("\n---\n\n");
    if let Some(history) = json["history"].as_array() {
        for (i, entry) in history.iter().enumerate() {
            if let (Some(u), Some(a)) = (entry["user"].as_str(), entry["assistant"].as_str()) {
                md.push_str(&format!("## Turn {}\n\n", i + 1));
                md.push_str("### User\n\n");
                md.push_str(&format!("```\n{}\n```\n\n", u));
                md.push_str("### Assistant\n\n");
                if a.contains("```") {
                    md.push_str(&format!("{}\n\n", a));
                } else {
                    md.push_str(&format!("```\n{}\n```\n\n", a));
                }
            }
        }
    }
    if let Some(code) = json["last_code"].as_str() {
        if !code.is_empty() {
            md.push_str("## Last Code\n\n");
            md.push_str(&format!("```\n{}\n```\n\n", code));
        }
    }
    match std::fs::write(&out_path, &md) {
        Ok(()) => println!(
            "{} session exported to {} ({} bytes)",
            ui::theme::paint_success_label(&t, "\u{2713}"),
            out_path.display(),
            md.len(),
        ),
        Err(e) => println!(
            "{} failed to export session: {}",
            ui::theme::paint_error_label(&t, "\u{2717}"),
            e
        ),
    }
}

/// Imports a session from a file and merges into the current session (`/session import <path>`).
pub(crate) fn handle_import_session(session: &mut ChatSession, path: &str) {
    let t = ui::theme::active();
    let base = session
        .ctx
        .project_dir
        .as_deref()
        .unwrap_or_else(|| std::path::Path::new("."));
    let in_path = match crate::types::resolve_safe_path(base, path) {
        Some(p) => p,
        None => {
            println!(
                "  {} path traversal blocked",
                ui::theme::paint_error_label(&t, "\u{2717}")
            );
            return;
        }
    };
    if !in_path.exists() {
        println!(
            "{} file not found: {}",
            ui::theme::paint_warning(&t, "\u{258C}"),
            in_path.display()
        );
        return;
    }
    match super::session_compact::read_maybe_gzip(&in_path) {
        Ok(content) => {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
                let mut restored = 0;
                if let Some(history) = data["history"].as_array() {
                    for entry in history {
                        if let (Some(u), Some(a)) = (entry["user"].as_str(), entry["assistant"].as_str()) {
                            session.history_mgr.push_turn(u.to_string(), a.to_string());
                            restored += 1;
                        }
                    }
                }
                if let Some(m) = data["mode"].as_str() {
                    session.mode = match m {
                        "Code" => crate::chat::RunMode::Code,
                        "Plan" => crate::chat::RunMode::Plan,
                        _ => crate::chat::RunMode::Chat,
                    };
                }
                if let Some(code) = data["last_code"].as_str() {
                    if !code.is_empty() {
                        session.code_out.last_code = code.to_string();
                    }
                }
                if let Some(files) = data["last_files"].as_array() {
                    let imported_files: Vec<FileEntry> = files
                        .iter()
                        .filter_map(|f| {
                            Some(FileEntry {
                                path: f["path"].as_str()?.to_string(),
                                content: f["content"].as_str()?.to_string(),
                            })
                        })
                        .collect();
                    if !imported_files.is_empty() {
                        session.code_out.last_files = imported_files;
                    }
                }
                println!(
                    "{} session imported from {} ({} turns restored)",
                    ui::theme::paint_success_label(&t, "\u{2713}"),
                    in_path.display(),
                    restored,
                );
            } else {
                println!(
                    "{} invalid session file: {}",
                    ui::theme::paint_error_label(&t, "\u{2717}"),
                    in_path.display()
                );
            }
        }
        Err(e) => println!(
            "{} failed to read session: {}",
            ui::theme::paint_error_label(&t, "\u{2717}"),
            e
        ),
    }
}
