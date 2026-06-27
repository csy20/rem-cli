//! Session and workspace commands (`/dir`, `/files`, `/config`, `/memory`, `/init`, etc.).
//! Handlers for commands that manage the workspace directory, configuration,
//! project memory, token stats, and session save/resume.

use crate::chat::ChatSession;
use crate::config::persist_workspace;
use crate::memory::ProjectMemory;
use crate::provider::Provider;
use crate::session_io::detect_project_type;
use crate::text_util::human_size;
use crate::token_count::{context_usage_percent, estimate_tokens_batch};
use crate::types::{file_icon, BackupEntry, FileEntry};
use crate::ui;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use walkdir::WalkDir;

/// Sets the workspace directory (`/dir` command).
pub(crate) fn handle_dir(session: &mut ChatSession, path: &str) {
    let t = ui::theme::active();
    let raw = path.trim();
    let cwd = std::env::current_dir().unwrap_or_default();

    let dir = if raw == "." {
        cwd.clone()
    } else {
        let p = PathBuf::from(raw);
        if p.is_absolute() {
            p
        } else {
            cwd.join(&p)
        }
    };

    // Canonicalize to prevent path traversal (resolves .. segments)
    let resolved = match dir.canonicalize() {
        Ok(r) => r,
        Err(_) => {
            // Directory doesn't exist yet; canonicalize parent
            if let Some(parent) = dir.parent() {
                if let Ok(canon_parent) = parent.canonicalize() {
                    if let Some(name) = dir.file_name() {
                        let safe = canon_parent.join(name);
                        // Prevent traversal via parent dir
                        if safe.starts_with(&canon_parent) {
                            safe
                        } else {
                            eprintln!(
                                "  {} path traversal blocked",
                                ui::theme::paint_error_label(&t, "\u{2717}")
                            );
                            return;
                        }
                    } else {
                        canon_parent
                    }
                } else {
                    println!(
                        "  {} parent directory does not exist: {}",
                        ui::theme::paint_warning(&t, "!"),
                        parent.display()
                    );
                    return;
                }
            } else {
                println!("  {} invalid directory: {}", ui::theme::paint_warning(&t, "!"), raw);
                return;
            }
        }
    };

    if resolved.exists() {
        session.ctx.project_dir = Some(resolved.clone());
        session.ctx.workspace_dir = Some(resolved.clone());
        persist_workspace(&resolved);
        println!(
            "  {} workspace set to {}",
            ui::theme::paint_success_label(&t, "✓"),
            ui::theme::paint_bright(&t, &resolved.display().to_string())
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
        session.ctx.project_dir = Some(resolved.clone());
        session.ctx.workspace_dir = Some(resolved.clone());
        persist_workspace(&resolved);
        println!(
            "  {} workspace set to {}",
            ui::theme::paint_success_label(&t, "✓"),
            ui::theme::paint_bright(&t, &resolved.display().to_string())
        );
    }
}

/// Lists project files in a tree view (`/files` command).
pub(crate) fn handle_list_files(session: &ChatSession) {
    let dir = session
        .ctx
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
    for entry in WalkDir::new(&dir).max_depth(4).into_iter().filter_map(|e| e.ok()) {
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

/// Displays current configuration (`/config` command).
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
        ui::theme::paint_dim(&t, &client.ctx.model)
    );
    println!(
        "{}   {:<18} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "base url:"),
        ui::theme::paint_dim(&t, &client.ctx.base_url)
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
                .ctx
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
        ui::theme::paint_dim(&t, "/model <name>  /provider <name>  /config workspace <path>")
    );
    println!("{}", ui::theme::paint(&t, "accent", "\u{258C}", true));
}

/// Sets a configuration value (`/config workspace <path>`).
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
                crate::commands::handle_dir(session, parts[1]);
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
            println!("{} available: model, workspace", ui::theme::paint_rail_empty(&t));
        }
    }
}

/// Shows token usage and speed stats (`/tokens` command).
pub(crate) fn handle_tokens(session: &ChatSession, client: &Provider) {
    let tokens = session.last_tokens;
    let elapsed = session.last_elapsed.as_secs_f64();
    let history_tokens: usize = session
        .history_mgr
        .history
        .iter()
        .map(|(u, a)| estimate_tokens_batch(&[u, a]))
        .sum();
    let model_ctx = client.ctx.model_ctx;
    let pct = context_usage_percent(history_tokens, model_ctx);
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
                    session.history_mgr.history.len()
                )
            )
        );

        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "context window:"),
            ui::theme::paint_dim(&t, &format!("{:.0}% used ({} limit)", pct, model_ctx))
        );
    } else {
        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "context:"),
            ui::theme::paint_dim(&t, "empty (no history)")
        );
    }

    let usage = client.anthropic_usage();
    if usage.input_tokens > 0 || usage.output_tokens > 0 {
        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "input tokens:"),
            ui::theme::paint_dim(&t, &usage.input_tokens.to_string())
        );
        println!(
            "{}   {:<18} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, "output tokens:"),
            ui::theme::paint_dim(&t, &usage.output_tokens.to_string())
        );
        if usage.cache_creation_input_tokens > 0 {
            println!(
                "{}   {:<18} {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_bright(&t, "cache created:"),
                ui::theme::paint_dim(&t, &format!("{} tokens", usage.cache_creation_input_tokens))
            );
        }
        if usage.cache_read_input_tokens > 0 {
            println!(
                "{}   {:<18} {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_bright(&t, "cache read:"),
                ui::theme::paint_dim(&t, &format!("{} tokens", usage.cache_read_input_tokens))
            );
        }
    }

    println!("{}", ui::theme::paint_rail_empty(&t));
}

/// Displays the project memory (`/memory` command).
pub(crate) fn handle_memory(session: &ChatSession) {
    let t = ui::theme::active();
    println!("{}", ui::theme::paint_rail_header(&t, "MEMORY"));
    if session.ctx.project_memory.loaded && !session.ctx.project_memory.content.is_empty() {
        for line in session.ctx.project_memory.content.lines() {
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

/// Sets or appends to project memory (`/memory set ...`).
pub(crate) fn handle_memory_set(session: &mut ChatSession, args: &str) {
    let t = ui::theme::active();
    if args.eq_ignore_ascii_case("clear") {
        session.ctx.project_memory.content.clear();
        session.ctx.project_memory.loaded = false;
        let _ = session.ctx.project_memory.save();
        println!("{} memory cleared", ui::theme::paint_success_label(&t, "\u{2713}"));
        return;
    }
    if let Some(text) = args.strip_prefix("add ") {
        if let Err(e) = session.ctx.project_memory.append(text) {
            println!("{} failed: {}", ui::theme::paint_error_label(&t, "\u{2717}"), e);
        } else {
            println!(
                "{} appended to memory ({} bytes)",
                ui::theme::paint_success_label(&t, "\u{2713}"),
                text.len()
            );
        }
        return;
    }
    if let Err(e) = session.ctx.project_memory.set(args) {
        println!("{} failed: {}", ui::theme::paint_error_label(&t, "\u{2717}"), e);
    } else {
        println!(
            "{} memory saved ({} bytes)",
            ui::theme::paint_success_label(&t, "\u{2713}"),
            args.len()
        );
    }
}

/// Generates starter project memory (`/init` command).
pub(crate) fn handle_init(session: &mut ChatSession) {
    let dir = session
        .ctx
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
    if let Err(e) = session.ctx.project_memory.set(&starter) {
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

/// Compacts conversation history via LLM summarization (`/compact`).
pub(crate) async fn handle_compact(client: &Provider, session: &mut ChatSession) {
    let t = ui::theme::active();
    if session.history_mgr.history.is_empty() {
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
        session.history_mgr.history.len()
    );
    let saved_history = session.history_mgr.history.clone();
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

/// Reads a file that may be gzip-compressed or plain text.
fn read_maybe_gzip(path: &std::path::Path) -> std::io::Result<String> {
    let raw = fs::read(path)?;
    // Check gzip magic number
    if raw.first() == Some(&0x1f) && raw.get(1) == Some(&0x8b) {
        let mut decoder = GzDecoder::new(&raw[..]);
        let mut out = String::new();
        decoder.read_to_string(&mut out)?;
        Ok(out)
    } else {
        String::from_utf8(raw).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

/// Saves the current session to `.rem/session.json` (`/save`).
pub(crate) fn handle_save_session(session: &ChatSession) {
    let t = ui::theme::active();
    let dir = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let session_file = dir.join(".rem/session.json");
    let _ = fs::create_dir_all(dir.join(".rem"));
    let json = serde_json::to_string_pretty(&session.to_session_json()).unwrap_or_default();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    let _ = encoder.write_all(json.as_bytes());
    let compressed = encoder.finish().unwrap_or_default();
    match fs::write(&session_file, &compressed) {
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

/// Restores a saved session from `.rem/session.json` (`/resume`).
pub(crate) fn handle_resume_session(session: &mut ChatSession) {
    let t = ui::theme::active();
    let dir = session
        .ctx
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
    match read_maybe_gzip(&session_file) {
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
                    println!(
                        "{} {} {}",
                        ui::theme::paint_dim(&t, "\u{258C}"),
                        ui::theme::paint_dim(&t, "saved mode:"),
                        ui::theme::paint_bright(&t, m)
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
                    let written: Vec<BackupEntry> = paths
                        .iter()
                        .filter_map(|p| {
                            p.as_str().map(|s| BackupEntry {
                                path: PathBuf::from(s),
                                original: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_dir_resolves_absolute_path() {
        let tmp = std::env::temp_dir().join(format!("rem-test-dir-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut session = crate::chat::ChatSession::new("test", Some(tmp.clone())).unwrap();

        handle_dir(&mut session, tmp.to_str().unwrap());
        assert_eq!(session.ctx.project_dir, Some(tmp.clone()));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
