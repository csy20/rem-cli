use crate::chat::ChatSession;
use crate::config::persist_workspace;
use crate::session_io::detect_project_type;
use crate::text_util::human_size;
use crate::types::file_icon;
use crate::ui;
use std::fs;
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

    let resolved = match dir.canonicalize() {
        Ok(r) => r,
        Err(_) => {
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
        crate::pager::maybe_page(&buf);
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

/// Generates starter project memory (`/init` command).
pub(crate) fn handle_init(session: &mut ChatSession) {
    use crate::memory::ProjectMemory;
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

/// Opens $VISUAL or $EDITOR to write multi-line input (/edit command).
pub(crate) fn handle_edit() -> Option<String> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let tmp_path = std::env::temp_dir().join(format!("rem-edit-{}.md", std::process::id()));
    match std::process::Command::new(&editor).arg(&tmp_path).status() {
        Ok(status) if status.success() => {
            let content = match std::fs::read_to_string(&tmp_path) {
                Ok(c) => c,
                Err(e) => {
                    let t = crate::ui::theme::active();
                    eprintln!(
                        "{} failed to read editor output: {}",
                        crate::ui::theme::paint_error_label(&t, "err:"),
                        e
                    );
                    let _ = std::fs::remove_file(&tmp_path);
                    return None;
                }
            };
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
