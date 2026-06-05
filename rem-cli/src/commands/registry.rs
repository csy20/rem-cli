// ── commands/registry.rs ── cli was written in rust so dont write in py same design but redesign the current cli
//
// Slash command registry: every `/...` command that the REPL recognizes.
// Handlers are plain functions; state is held in module-level `RefCell`
// only for the conversation history. The header, theme, and config are
// always re-read fresh from disk inside each handler so live edits from
// other processes are picked up.
#![allow(dead_code)]

use std::cell::RefCell;
use std::io::{self, Write};
use std::process::Command as StdCommand;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config;
use crate::ui::theme;

#[derive(Debug, Clone)]
pub struct Command {
    pub name: &'static str,
    pub description: &'static str,
    pub shortcut: &'static str,
    pub handler: fn(),
}

pub const REGISTRY: &[Command] = &[
    Command {
        name: "/help",
        description: "show all commands and keybindings",
        shortcut: "?",
        handler: cmd_help,
    },
    Command {
        name: "/mode",
        description: "switch CHAT <-> CODE",
        shortcut: "m",
        handler: cmd_mode,
    },
    Command {
        name: "/model",
        description: "change active model",
        shortcut: "M",
        handler: cmd_model,
    },
    Command {
        name: "/theme",
        description: "change color theme",
        shortcut: "t",
        handler: cmd_theme,
    },
    Command {
        name: "/clear",
        description: "clear conversation history",
        shortcut: "c",
        handler: cmd_clear,
    },
    Command {
        name: "/save",
        description: "save session to file",
        shortcut: "s",
        handler: cmd_save,
    },
    Command {
        name: "/exit",
        description: "quit rem",
        shortcut: "q",
        handler: cmd_exit,
    },
];

/// Lookup by exact name (e.g. "/help"). Returns `None` for unknown.
pub fn find(name: &str) -> Option<&'static Command> {
    REGISTRY.iter().find(|c| c.name == name)
}

thread_local! {
    static HISTORY: RefCell<Vec<(String, String)>> = RefCell::new(Vec::new());
}

pub fn history() -> Vec<(String, String)> {
    HISTORY.with(|h| h.borrow().clone())
}

pub fn push_history(user: String, assistant: String) {
    HISTORY.with(|h| h.borrow_mut().push((user, assistant)));
}

pub fn clear_history() {
    HISTORY.with(|h| h.borrow_mut().clear());
}

// ── Handlers ──────────────────────────────────────────────────────────────

fn cmd_help() {
    let t = theme::active();
    let border = theme::paint(&t.border, "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}", false);
    theme::println(&format!(
        "  {} {}",
        theme::paint(&t.accent, "COMMANDS", true),
        border
    ));

    let name_w = REGISTRY.iter().map(|c| c.name.len()).max().unwrap_or(8);
    let desc_w = REGISTRY
        .iter()
        .map(|c| c.description.len())
        .max()
        .unwrap_or(20);
    for cmd in REGISTRY.iter() {
        let name = theme::paint(
            &t.accent,
            &format!("  {:<width$}", cmd.name, width = name_w),
            true,
        );
        let desc = theme::paint(
            &t.text_muted,
            &format!("{:<width$}", cmd.description, width = desc_w),
            false,
        );
        let sc = if cmd.shortcut.is_empty() {
            "".to_string()
        } else {
            theme::paint_on(
                &t.kbd_text,
                &t.kbd_bg,
                &format!(" {} ", cmd.shortcut),
                false,
            )
        };
        theme::println(&format!("{name}  {desc}  {sc}"));
    }
    theme::println("");
    theme::println(&format!(
        "  {}",
        theme::paint(
            &t.text_faint,
            "tip: press / on an empty prompt to open the palette",
            false
        )
    ));
}

fn cmd_mode() {
    let t = theme::active();
    let mut cfg = config::load_config();
    cfg.mode = if cfg.mode == "CHAT" {
        "CODE".to_string()
    } else {
        "CHAT".to_string()
    };
    let _ = config::save_config(&cfg);
    theme::println(&format!(
        "  {} {}",
        theme::paint(&t.accent, "▌ mode switched to", true),
        theme::paint(&t.accent, &cfg.mode, true),
    ));
    crate::ui::header::render(&cfg.model, &cfg.mode);
}

fn cmd_theme() {
    let names = theme::list_names();
    render_theme_picker(&names);
    match theme_picker_read(&names) {
        Some(idx) => {
            let chosen = names[idx].clone();
            if theme::set_active(&chosen) {
                let t = theme::active();
                theme::println(&format!(
                    "  {} {}",
                    theme::paint(&t.accent, "▌ theme switched to", true),
                    theme::paint(&t.accent, &chosen, true),
                ));
                let cfg = config::load_config();
                crate::ui::header::render(&cfg.model, &cfg.mode);
            }
        }
        None => {
            let t = theme::active();
            theme::println(&format!(
                "  {}",
                theme::paint(&t.text_muted, "Cancelled.", false)
            ));
        }
    }
}

fn cmd_model() {
    let models = fetch_ollama_models();
    if models.is_empty() {
        let t = theme::active();
        theme::println(&format!(
            "  {}",
            theme::paint(&t.text_muted, "(no models found via `ollama list`)", false)
        ));
        return;
    }
    render_model_picker(&models);
    match theme_picker_read(&models) {
        Some(idx) => {
            let mut cfg = config::load_config();
            cfg.model = models[idx].clone();
            let _ = config::save_config(&cfg);
            let t = theme::active();
            theme::println(&format!(
                "  {} {}",
                theme::paint(&t.accent, "▌ model set to", true),
                theme::paint(&t.accent, &cfg.model, true),
            ));
            crate::ui::header::render(&cfg.model, &cfg.mode);
        }
        None => {
            let t = theme::active();
            theme::println(&format!(
                "  {}",
                theme::paint(&t.text_muted, "Cancelled.", false)
            ));
        }
    }
}

fn cmd_clear() {
    clear_history();
    let t = theme::active();
    theme::println(&format!(
        "  {}",
        theme::paint(&t.accent, "History cleared.", true)
    ));
}

fn cmd_save() {
    let t = theme::active();
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = format!("rem_session_{ts}.json");
    let payload = serde_json::json!({
        "timestamp": ts,
        "history": history(),
        "config": config::load_config(),
    });
    let serialized = match serde_json::to_string_pretty(&payload) {
        Ok(s) => s,
        Err(e) => {
            theme::println(&format!(
                "  {}",
                theme::paint(&t.sys_color, &format!("save failed: {e}"), false)
            ));
            return;
        }
    };
    match std::fs::write(&path, serialized) {
        Ok(()) => theme::println(&format!(
            "  {} {}",
            theme::paint(&t.accent, "▌ saved to", true),
            theme::paint(&t.accent_dim, &path, false),
        )),
        Err(e) => theme::println(&format!(
            "  {}",
            theme::paint(&t.sys_color, &format!("save failed: {e}"), false)
        )),
    }
}

fn cmd_exit() {
    let t = theme::active();
    theme::println(&format!(
        "  {}",
        theme::paint(&t.text_muted, "Goodbye.", false)
    ));
    std::process::exit(0);
}

// ── Pickers (theme + model) ───────────────────────────────────────────────

fn render_theme_picker(names: &[String]) {
    let t = theme::active();
    let cfg = config::load_config();
    theme::println(&format!(
        "  {}",
        theme::paint(&t.accent, "SELECT THEME", true)
    ));
    let w = names.iter().map(|s| s.len()).max().unwrap_or(6);
    for (i, name) in names.iter().enumerate() {
        let marker = if name.eq_ignore_ascii_case(&cfg.theme) {
            "●"
        } else {
            " "
        };
        let num = format!("  {} ", i + 1);
        let label = theme::paint(&t.accent, &num, true);
        let key = theme::paint(&t.accent, &format!("{:<w$}", name, w = w), true);
        let mut desc = String::new();
        desc.push_str(theme_short_desc(name));
        let desc_colored = theme::paint(&t.text_muted, &desc, false);
        let marker_colored = theme::paint(&t.accent_dim, marker, false);
        theme::println(&format!("{label} {key}  {desc_colored}  {marker_colored}"));
        // Live preview line
        crate::ui::header::render_with_theme(&cfg.model, &cfg.mode, name);
    }
    theme::println("");
    theme::println(&format!(
        "  {}",
        theme::paint(&t.text_faint, "press 1-6 to switch, ESC to cancel", false)
    ));
    let _ = io::stdout().flush();
}

fn theme_short_desc(name: &str) -> &'static str {
    match name {
        "GHOST" => "pure black · ghost white accents",
        "PHOSPHOR" => "CRT green · retro terminal",
        "MIST" => "slate gray · cool blue tones",
        "EMBER" => "warm amber · charcoal dark",
        "SAKURA" => "deep navy · pink accents",
        "PAPER" => "off-white · ink on vellum",
        _ => "",
    }
}

fn render_model_picker(models: &[String]) {
    let t = theme::active();
    let cfg = config::load_config();
    theme::println(&format!(
        "  {}",
        theme::paint(&t.accent, "SELECT MODEL", true)
    ));
    let w = models.iter().map(|s| s.len()).max().unwrap_or(6);
    for (i, name) in models.iter().enumerate() {
        let marker = if name == &cfg.model { "●" } else { " " };
        let num = format!("  {} ", i + 1);
        let label = theme::paint(&t.accent, &num, true);
        let key = theme::paint(&t.accent_dim, &format!("{:<w$}", name, w = w), false);
        let marker_colored = theme::paint(&t.accent, marker, true);
        theme::println(&format!("{label} {key}  {marker_colored}"));
    }
    theme::println("");
    theme::println(&format!(
        "  {}",
        theme::paint(&t.text_faint, "press a number, or ESC to cancel", false)
    ));
    let _ = io::stdout().flush();
}

fn theme_picker_read(options: &[String]) -> Option<usize> {
    use std::io::Read;
    let stdin = io::stdin();
    let mut handle = stdin.lock();
    let mut buf = [0u8; 1];
    let mut collected = String::new();
    loop {
        let n = match handle.read(&mut buf) {
            Ok(n) => n,
            Err(_) => return None,
        };
        if n == 0 {
            return None;
        }
        match buf[0] {
            0x1b => return None,
            b'\r' | b'\n' => {
                if let Ok(idx) = collected.trim().parse::<usize>() {
                    if idx >= 1 && idx <= options.len() {
                        return Some(idx - 1);
                    }
                }
                return None;
            }
            0x7f | 0x08 => {
                collected.pop();
            }
            c if c.is_ascii_digit() => collected.push(c as char),
            _ => {}
        }
    }
}

fn fetch_ollama_models() -> Vec<String> {
    let output = StdCommand::new("ollama").arg("list").output();
    let output = match output {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    if !output.status.success() {
        return Vec::new();
    }
    let text = match std::str::from_utf8(&output.stdout) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("NAME") {
            continue;
        }
        let first = line.split_whitespace().next().unwrap_or("");
        if !first.is_empty() {
            out.push(first.to_string());
        }
    }
    out
}

/// Render a themed "section header" used by subcommands (`new`, `ask`,
/// `explain`, `patch`). Two lines: a bold title bar in the active accent,
/// then a faint subtitle line.
pub fn print_section(title: &str, subtitle: &str) {
    let t = theme::active();
    let bar = "\u{2500}".repeat(48usize.saturating_sub(title.chars().count()));
    theme::println(&format!(
        "  {} {}",
        theme::paint(&t.accent, title, true),
        theme::paint(&t.border, &bar, false),
    ));
    if !subtitle.is_empty() {
        theme::println(&format!(
            "  {}",
            theme::paint(&t.text_muted, subtitle, false),
        ));
    }
}

/// Print a themed success message. Equivalent to the old `style!(C_GREEN, "✓")` line.
pub fn print_success(msg: &str) {
    let t = theme::active();
    theme::println(&format!(
        "  {} {}",
        theme::paint(&t.accent, "\u{2713}", true),
        theme::paint(&t.accent, msg, true),
    ));
}

/// Print a themed file/entry row (icon + path + size).
pub fn print_file_row(icon: &str, path: &str, size: usize) {
    let t = theme::active();
    theme::println(&format!(
        "    {} {} {}",
        theme::paint(&t.text_faint, icon, false),
        theme::paint(&t.accent, path, false),
        theme::paint(&t.text_muted, &format!("({} bytes)", size), false),
    ));
}

/// Print a faint hint line at the end of an operation.
pub fn print_hint(msg: &str) {
    let t = theme::active();
    theme::println(&format!(
        "  {} {}",
        theme::paint(&t.text_faint, "next:", true),
        theme::paint(&t.text_muted, msg, false),
    ));
}

/// Theme-colored file icon for a given path.
pub fn file_icon_for(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".html") || lower.ends_with(".htm") {
        "\u{1F310}"
    } else if lower.ends_with(".css") {
        "\u{1F3A8}"
    } else if lower.ends_with(".js") || lower.ends_with(".mjs") || lower.ends_with(".ts") {
        "\u{26A1}"
    } else if lower.ends_with(".json") {
        "\u{1F4CB}"
    } else if lower.ends_with(".md") || lower.ends_with(".txt") {
        "\u{1F4C4}"
    } else if lower.ends_with(".py") {
        "\u{1F40D}"
    } else {
        "\u{1F4C4}"
    }
}
