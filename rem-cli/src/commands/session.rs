//! Session and workspace commands (`/dir`, `/files`, `/config`, `/memory`, `/init`, etc.).
//! Handlers for commands that manage the workspace directory, configuration,
//! project memory, token stats, and session save/resume.

use crate::chat::ChatSession;
use crate::config::persist_workspace;
use crate::memory::ProjectMemory;
use crate::pager::maybe_page;
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
use std::path::{Path, PathBuf};
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
                        if safe
                            .canonicalize()
                            .map(|c| c.starts_with(&canon_parent))
                            .unwrap_or(false)
                        {
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
        session.ctx.invalidate_caches();
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
        session.ctx.invalidate_caches();
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

    let mut entries: Vec<(String, bool, u64)> = Vec::new();
    for entry in WalkDir::new(&dir)
        .max_depth(4)
        .follow_links(false)
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

    if entries.len() > 46 {
        let mut buf = String::new();
        buf.push_str(&format!("{}\n", ui::theme::paint_rail_empty(&t)));
        buf.push_str(&format!(
            "{} {}\n",
            ui::theme::paint_rail_empty(&t),
            ui::theme::paint_bright(&t, &format!("\u{1f4c2} project ({})", dir.display()))
        ));
        buf.push_str(&format!("{}\n", ui::theme::paint_rail_empty(&t)));
        for (path, is_dir, size) in &entries {
            let depth = path.chars().filter(|&c| c == '/').count();
            let indent = "  ".repeat(depth);
            let name = if let Some(pos) = path.rfind('/') {
                &path[pos + 1..]
            } else {
                path
            };
            if *is_dir {
                buf.push_str(&format!(
                    "{} {} {} {} \n",
                    ui::theme::paint_rail_empty(&t),
                    indent,
                    ui::theme::paint_dim(&t, "\u{251c}\u{2500}\u{2500}"),
                    ui::theme::paint(&t, "accent_info", &format!("\u{1f4c1} {}/", name), true)
                ));
            } else {
                let icon = file_icon(name);
                let hs = human_size(*size);
                buf.push_str(&format!(
                    "{} {} {} {} {} {}\n",
                    ui::theme::paint_rail_empty(&t),
                    indent,
                    ui::theme::paint_dim(&t, "\u{251c}\u{2500}\u{2500}"),
                    icon,
                    ui::theme::paint_bright(&t, name),
                    ui::theme::paint_dim(&t, &format!("({})", hs))
                ));
            }
        }
        buf.push_str(&format!("{}\n", ui::theme::paint_rail_empty(&t)));
        maybe_page(&buf);
        return;
    }

    println!("{}", ui::theme::paint_rail_empty(&t));
    println!(
        "{} {}",
        ui::theme::paint_rail_empty(&t),
        ui::theme::paint_bright(&t, &format!("\u{1f4c2} project ({})", dir.display()))
    );
    println!("{}", ui::theme::paint_rail_empty(&t));

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
        "edit" => {
            let config_path = crate::config::config_dir().unwrap_or_default().join("config.toml");
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
            println!(
                "{} {} opening {} with {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_dim(&t, "\u{270E}"),
                ui::theme::paint_bright(&t, &config_path.display().to_string()),
                editor
            );
            match std::process::Command::new(&editor).arg(&config_path).status() {
                Ok(status) if status.success() => {
                    crate::config::invalidate_config_cache();
                    println!(
                        "{} {} config reloaded",
                        ui::theme::paint(&t, "accent", "\u{258C}", true),
                        ui::theme::paint_success_label(&t, "\u{2713}")
                    );
                }
                Ok(_) => println!(
                    "{} {} editor exited with error",
                    ui::theme::paint(&t, "accent", "\u{258C}", true),
                    ui::theme::paint_warning(&t, "\u{2717}")
                ),
                Err(e) => println!(
                    "{} {} failed to launch {}: {}",
                    ui::theme::paint(&t, "accent", "\u{258C}", true),
                    ui::theme::paint_error_label(&t, "\u{2717}"),
                    editor,
                    e
                ),
            }
        }
        other => {
            println!(
                "{} unknown config key: {}",
                ui::theme::paint_warning(&t, "\u{258C}"),
                other
            );
            println!(
                "{} usage: /config workspace <path> | /config edit",
                ui::theme::paint_rail_empty(&t)
            );
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

/// Reloads config from disk and re-scans project context (`/reload` command).
pub(crate) fn handle_reload(session: &mut ChatSession, cfg: &mut crate::cli::AppConfig) {
    use crate::config::{invalidate_config_cache, load_config};
    let t = ui::theme::active();
    invalidate_config_cache();
    match load_config() {
        Ok(new_cfg) => {
            let old_workspace = cfg.workspace_dir.clone();
            let old_theme = cfg.theme.clone();
            *cfg = new_cfg;
            // Preserve runtime state that shouldn't be overwritten
            if cfg.workspace_dir.is_none() {
                cfg.workspace_dir = old_workspace;
            }
            if old_theme != cfg.theme {
                ui::theme::set_active(&cfg.theme);
            }
            crate::pager::init_page_threshold(cfg.page_threshold);
            // Re-scan project context
            session.ctx.invalidate_caches();
            if let Some(ref dir) = cfg.workspace_dir {
                let path = std::path::PathBuf::from(dir);
                if path.exists() {
                    session.ctx.project_dir = Some(path);
                    session.ctx.project_memory = crate::memory::ProjectMemory::load(
                        session
                            .ctx
                            .project_dir
                            .as_deref()
                            .unwrap_or_else(|| std::path::Path::new(".")),
                    );
                }
            }
            println!(
                "{} {}",
                ui::theme::paint_success_label(&t, "\u{2713}"),
                ui::theme::paint_dim(&t, "config reloaded from disk")
            );
        }
        Err(e) => {
            println!(
                "{} {}",
                ui::theme::paint_error_label(&t, "\u{2717}"),
                ui::theme::paint(&t, "error", &format!("failed to reload config: {e}"), false)
            );
        }
    }
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
    let ptype_label = if ptype.is_empty() { "unknown" } else { &ptype };
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
    let starter = ProjectMemory::generate_starter(&dir, &ptype);
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

/// Dry-run preview of compaction (`/compact-dry-run`).
/// Shows what would be summarized without calling the LLM.
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

/// Opens $VISUAL or $EDITOR to write multi-line input (/edit command).
/// Returns the editor content as a String, or an error message.
pub(crate) fn handle_edit() -> Option<String> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let tmp_path = std::env::temp_dir().join(format!("rem-edit-{}.md", std::process::id()));
    match std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{} \"$1\"", editor))
        .arg("rem-edit")
        .arg(&tmp_path)
        .status()
    {
        Ok(status) if status.success() => {
            let content = std::fs::read_to_string(&tmp_path).unwrap_or_default();
            let _ = std::fs::remove_file(&tmp_path);
            let trimmed = content.trim().to_string();
            if trimmed.is_empty() {
                let t = crate::ui::theme::active();
                println!(
                    "{} empty input, cancelled",
                    crate::ui::theme::paint_warning(&t, "\u{258C}")
                );
                None
            } else {
                Some(trimmed)
            }
        }
        Ok(_) => {
            let t = crate::ui::theme::active();
            println!(
                "{} editor exited with error",
                crate::ui::theme::paint_error_label(&t, "\u{258C}")
            );
            let _ = std::fs::remove_file(&tmp_path);
            None
        }
        Err(e) => {
            let t = crate::ui::theme::active();
            eprintln!(
                "{} failed to launch editor '{}': {}",
                crate::ui::theme::paint_error_label(&t, "err:"),
                editor,
                e
            );
            None
        }
    }
}

/// Shows debug context being sent to the model (`/context` command).
/// Displays the assembled prompt with character/token counts and session analytics.
pub(crate) fn handle_context(session: &ChatSession, client: &Provider) {
    let t = ui::theme::active();
    let history = session.history_mgr.build_chat_history();
    let turn_count = session.history_mgr.history.len();
    let elapsed = session.session_start.elapsed();

    println!("{}", ui::theme::paint_rail_header(&t, "CONTEXT DEBUG"));
    println!(
        "{} {} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "Provider:"),
        ui::theme::paint_dim(&t, client.kind.as_str()),
    );
    println!(
        "{} {} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "Model:"),
        ui::theme::paint_dim(&t, &client.ctx.model),
    );
    println!(
        "{} {} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "Mode:"),
        ui::theme::paint_dim(&t, session.mode.label()),
    );
    println!(
        "{} {} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "Turns:"),
        ui::theme::paint_dim(&t, &turn_count.to_string()),
    );
    println!(
        "{} {} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "Total Tokens:"),
        ui::theme::paint_dim(&t, &session.total_tokens.to_string()),
    );
    println!(
        "{} {} {:.1}s",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "Duration:"),
        elapsed.as_secs_f64(),
    );
    println!("{}", ui::theme::paint_rail_empty(&t));

    println!(
        "{} Chat history: {} chars, ~{} tokens",
        ui::theme::paint_bright(&t, "\u{258C}"),
        history.len(),
        crate::token_count::estimate_tokens(&history),
    );
    if !history.is_empty() {
        println!("{}", ui::theme::paint_rail_header(&t, "HISTORY"));
        println!("{}", history);
    }
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
    // Confirm before destructive compact
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
    let saved_history = session.history_mgr.history.clone();
    // Persist backup before compacting (for /compact --undo)
    let dir = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let backup_path = dir.join(".rem/compact_backup.json.gz");
    let _ = std::fs::create_dir_all(dir.join(".rem"));
    if let Ok(json) = serde_json::to_string_pretty(&saved_history) {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        if encoder.write_all(json.as_bytes()).is_ok() {
            if let Ok(compressed) = encoder.finish() {
                let _ = std::fs::write(&backup_path, &compressed);
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
                let _ = std::fs::remove_file(&backup_path);
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
fn read_maybe_gzip(path: &std::path::Path) -> std::io::Result<String> {
    let raw = fs::read(path)?;
    // Check gzip magic number
    if raw.first() == Some(&0x1f) && raw.get(1) == Some(&0x8b) {
        let mut decoder = GzDecoder::new(&raw[..]);
        let mut out = String::new();
        decoder.read_to_string(&mut out).map_err(|e| {
            if out.len() > 100_000_000 {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "decompressed data exceeds size limit")
            } else {
                e
            }
        })?;
        Ok(out)
    } else {
        String::from_utf8(raw).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

/// Saves the current session to `.rem/session.json.gz` (`/save`).
pub(crate) fn handle_save_session(session: &ChatSession) {
    let t = ui::theme::active();
    let dir = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let session_file = dir.join(".rem/session.json.gz");
    let _ = fs::create_dir_all(dir.join(".rem"));
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
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    if let Err(e) = encoder.write_all(json.as_bytes()) {
        println!(
            "{} compression failed: {}",
            ui::theme::paint_error_label(&t, "\u{2717}"),
            e
        );
        return;
    }
    let compressed = match encoder.finish() {
        Ok(c) => c,
        Err(e) => {
            println!(
                "{} compression failed: {}",
                ui::theme::paint_error_label(&t, "\u{2717}"),
                e
            );
            return;
        }
    };
    match fs::write(&out_path, &compressed) {
        Ok(()) => println!(
            "{} session exported to {} ({} bytes)",
            ui::theme::paint_success_label(&t, "\u{2713}"),
            out_path.display(),
            compressed.len(),
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

/// Lists saved session files in `.rem/` (`/session list`).
pub(crate) fn handle_list_sessions(session: &ChatSession) {
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
    let mut sessions: Vec<(String, u64)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&rem_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("gz")
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.ends_with(".json"))
            {
                if let Ok(meta) = entry.metadata() {
                    let size = meta.len();
                    let name = entry.file_name().to_string_lossy().to_string();
                    sessions.push((name, size));
                }
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
    sessions.sort_by_key(|b| std::cmp::Reverse(b.1));
    println!("{}", ui::theme::paint_rail_header(&t, "SAVED SESSIONS"));
    for (name, size) in &sessions {
        let icon = file_icon(name);
        println!(
            "{} {} {} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            icon,
            ui::theme::paint_bright(&t, name),
            ui::theme::paint_dim(&t, &format!("({})", human_size(*size)))
        );
    }
    println!("{}", ui::theme::paint_rail_empty(&t));
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
    match read_maybe_gzip(&in_path) {
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

    #[test]
    fn handle_export_session_blocks_path_traversal() {
        let tmp = std::env::temp_dir().join(format!("rem-test-export-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let session = crate::chat::ChatSession::new("test", Some(tmp.clone())).unwrap();

        // Attempt to export with path traversal — should not crash
        handle_export_session(&session, "../escape.gz");

        // Verify the escape file was NOT created outside the project dir
        let parent = tmp.parent().unwrap();
        assert!(!parent.join("escape.gz").exists(), "path traversal should be blocked");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn handle_import_session_blocks_path_traversal() {
        let tmp = std::env::temp_dir().join(format!("rem-test-import-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut session = crate::chat::ChatSession::new("test", Some(tmp.clone())).unwrap();

        // Attempt to import with path traversal — should not crash
        handle_import_session(&mut session, "../escape.gz");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn handle_compact_undo_no_backup_does_not_panic() {
        let tmp = std::env::temp_dir().join(format!("rem-test-compact-undo-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut session = crate::chat::ChatSession::new("test", Some(tmp.clone())).unwrap();

        // No backup exists — should print message but not panic
        handle_compact_undo(&mut session);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
