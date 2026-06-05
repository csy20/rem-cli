// ── ui/palette.rs ── cli was written in rust so dont write in py same design but redesign the current cli
//
// Slash command palette: a numbered, filterable picker. Renders an inline
// panel with the command list, then reads a sub-line of text that doubles
// as a live filter and a selection. ↑↓ navigation is provided as a no-cost
// bonus (the user can just type the prefix they want).
#![allow(dead_code)]

use std::io::{self, Read, Write};

use crate::commands::registry::{Command, REGISTRY};
use crate::ui::theme;

/// Read a single raw keypress from stdin in non-canonical mode. Returns
/// the byte (or multi-byte sequence) that was read, or `None` for ESC.
/// We do not use termios directly because the existing editor still owns
/// the terminal while the palette is up; instead we just read what's
/// available with a zero timeout, leaving the line discipline alone.
pub fn show_palette() -> Option<String> {
    let mut filtered: Vec<Command> = REGISTRY.iter().cloned().collect();
    let mut selected: usize = 0;
    let mut query = String::new();

    render_palette(&filtered, selected, &query);

    let stdin = io::stdin();
    let mut handle = stdin.lock();
    let mut buf = [0u8; 1];

    loop {
        let n = match handle.read(&mut buf) {
            Ok(n) => n,
            Err(_) => return None,
        };
        if n == 0 {
            return None;
        }
        let b = buf[0];
        match b {
            0x1b => {
                // ESC — single 0x1b is the escape key. CSI sequences
                // (e.g. arrow keys) start with 0x1b 0x5b; consume the
                // bracketed part if present.
                let mut peek = [0u8; 1];
                if handle.read_exact(&mut peek).is_ok() && peek[0] == b'[' {
                    let mut tail = [0u8; 1];
                    if handle.read_exact(&mut tail).is_ok() {
                        match tail[0] {
                            b'A' => {
                                if !filtered.is_empty() {
                                    selected = (selected + filtered.len() - 1) % filtered.len();
                                    render_palette(&filtered, selected, &query);
                                }
                            }
                            b'B' => {
                                if !filtered.is_empty() {
                                    selected = (selected + 1) % filtered.len();
                                    render_palette(&filtered, selected, &query);
                                }
                            }
                            _ => {}
                        }
                    }
                } else {
                    return None;
                }
            }
            b'\r' | b'\n' => {
                if let Some(cmd) = filtered.get(selected) {
                    return Some(cmd.name.to_string());
                }
                return None;
            }
            0x7f | 0x08 => {
                query.pop();
                (filtered, selected) = apply_filter(&query);
                render_palette(&filtered, selected, &query);
            }
            0x03 => return None, // Ctrl+C cancels
            0x04 => return None, // Ctrl+D cancels
            c if c.is_ascii_graphic() || c == b' ' => {
                query.push(c as char);
                (filtered, selected) = apply_filter(&query);
                render_palette(&filtered, selected, &query);
            }
            _ => {}
        }
    }
}

fn apply_filter(query: &str) -> (Vec<Command>, usize) {
    let q = query.trim_start_matches('/').to_ascii_lowercase();
    if q.is_empty() {
        return (REGISTRY.to_vec(), 0);
    }
    let filtered: Vec<Command> = REGISTRY
        .iter()
        .filter(|c| {
            c.name
                .trim_start_matches('/')
                .to_ascii_lowercase()
                .starts_with(&q)
        })
        .cloned()
        .collect();
    (filtered, 0)
}

fn render_palette(filtered: &[Command], selected: usize, query: &str) {
    let t = theme::active();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let _ = write!(out, "\x1b[2J\x1b[H");

    let _ = writeln!(out, "{}", theme::paint(&t.accent, "  COMMANDS", true));
    let _ = writeln!(
        out,
        "{}",
        theme::paint(&t.border, "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}", false)
    );

    for (i, cmd) in filtered.iter().enumerate() {
        let is_sel = i == selected;
        let marker = if is_sel {
            theme::paint(&t.sel_left, "▸", true)
        } else {
            " ".to_string()
        };
        let row_bg = if is_sel { &t.sel_bg } else { &t.surface };
        let name = theme::paint_on(&t.accent, row_bg, &format!(" {}", cmd.name), true);
        let desc = theme::paint_on(
            &t.text_muted,
            row_bg,
            &format!(" {}", cmd.description),
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
        let _ = writeln!(out, " {marker}  {name}  {desc}    {sc}");
    }

    if filtered.is_empty() {
        let _ = writeln!(
            out,
            "  {}",
            theme::paint(&t.text_muted, "(no commands match)", false)
        );
    }

    let sub_prompt = format!(
        "  {} {}{}",
        theme::paint(&t.text_muted, "/", false),
        theme::paint(&t.accent, query, true),
        theme::paint(&t.cursor, "_", false)
    );
    let _ = writeln!(out);
    let _ = write!(out, "{sub_prompt}");
    let _ = out.flush();
}
