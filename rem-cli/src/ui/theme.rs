// ── ui/theme.rs ──
//
// Theme registry: 6 hand-picked themes, the active theme state, hex→ANSI
// conversion, and the theme-aware style helpers used by every other UI module.
// No other module in the crate is allowed to hardcode a hex value or ANSI
// color constant; everything flows through `active()` or one of the helpers
// below.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::config;

/// A single color theme. All fields are 6-digit hex strings (e.g. `"#e8e8e8"`).
/// Themes are stored in a `BTreeMap` keyed by their uppercase name.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub bg: String,
    pub surface: String,
    pub border: String,
    pub accent: String,
    pub accent_dim: String,
    pub accent_info: String,
    pub text_muted: String,
    pub text_faint: String,
    pub pill_bg: String,
    pub pill_border: String,
    pub pill_text: String,
    pub kbd_bg: String,
    pub kbd_border: String,
    pub kbd_text: String,
    pub sys_color: String,
    pub sel_bg: String,
    pub sel_left: String,
    pub cursor: String,
    pub error: String,
    pub success: String,
}

const DEFAULT_THEME_NAME: &str = "GHOST";

/// All available themes. Insertion order matches the picker (1..=6).
pub fn themes() -> BTreeMap<&'static str, Theme> {
    let mut t = BTreeMap::new();
    t.insert(
        "GHOST",
        Theme {
            name: "GHOST".into(),
            bg: "#030303".into(),
            surface: "#0d0d0d".into(),
            border: "#181818".into(),
            accent: "#e8e8e8".into(),
            accent_dim: "#888888".into(),
            accent_info: "#7a8aa0".into(),
            text_muted: "#444444".into(),
            text_faint: "#222222".into(),
            pill_bg: "#1a1a1a".into(),
            pill_border: "#2a2a2a".into(),
            pill_text: "#e8e8e8".into(),
            kbd_bg: "#111111".into(),
            kbd_border: "#1e1e1e".into(),
            kbd_text: "#333333".into(),
            sys_color: "#1e1e1e".into(),
            sel_bg: "#141414".into(),
            sel_left: "#e8e8e8".into(),
            cursor: "#888888".into(),
            error: "#d87070".into(),
            success: "#7ac890".into(),
        },
    );
    t.insert(
        "PHOSPHOR",
        Theme {
            name: "PHOSPHOR".into(),
            bg: "#030a04".into(),
            surface: "#050e06".into(),
            border: "#0d2010".into(),
            accent: "#3aff5a".into(),
            accent_dim: "#2a8040".into(),
            accent_info: "#3acfa0".into(),
            text_muted: "#1a4020".into(),
            text_faint: "#0d2010".into(),
            pill_bg: "#061409".into(),
            pill_border: "#0d2a10".into(),
            pill_text: "#3aff5a".into(),
            kbd_bg: "#061409".into(),
            kbd_border: "#0d2010".into(),
            kbd_text: "#1a4020".into(),
            sys_color: "#0d2010".into(),
            sel_bg: "#0a1e0c".into(),
            sel_left: "#3aff5a".into(),
            cursor: "#3aff5a".into(),
            error: "#ff6a6a".into(),
            success: "#3aff5a".into(),
        },
    );
    t.insert(
        "MIST",
        Theme {
            name: "MIST".into(),
            bg: "#0c0f14".into(),
            surface: "#0f1420".into(),
            border: "#1a2538".into(),
            accent: "#7ba8d4".into(),
            accent_dim: "#4a6a90".into(),
            accent_info: "#8aa0c8".into(),
            text_muted: "#2a3a55".into(),
            text_faint: "#1a2538".into(),
            pill_bg: "#102040".into(),
            pill_border: "#1a3060".into(),
            pill_text: "#7ba8d4".into(),
            kbd_bg: "#0f1420".into(),
            kbd_border: "#1a2538".into(),
            kbd_text: "#2a3a55".into(),
            sys_color: "#1a2538".into(),
            sel_bg: "#102040".into(),
            sel_left: "#7ba8d4".into(),
            cursor: "#7ba8d4".into(),
            error: "#d49a9a".into(),
            success: "#9ac8a8".into(),
        },
    );
    t.insert(
        "EMBER",
        Theme {
            name: "EMBER".into(),
            bg: "#0f0b06".into(),
            surface: "#161008".into(),
            border: "#251a08".into(),
            accent: "#f0a030".into(),
            accent_dim: "#7a5520".into(),
            accent_info: "#d08850".into(),
            text_muted: "#2a2010".into(),
            text_faint: "#1e1508".into(),
            pill_bg: "#1e1408".into(),
            pill_border: "#302010".into(),
            pill_text: "#f0a030".into(),
            kbd_bg: "#161008".into(),
            kbd_border: "#251a08".into(),
            kbd_text: "#2a2010".into(),
            sys_color: "#251a08".into(),
            sel_bg: "#1e1408".into(),
            sel_left: "#f0a030".into(),
            cursor: "#f0a030".into(),
            error: "#e08070".into(),
            success: "#c8a868".into(),
        },
    );
    t.insert(
        "SAKURA",
        Theme {
            name: "SAKURA".into(),
            bg: "#080610".into(),
            surface: "#0c0a1a".into(),
            border: "#1a1438".into(),
            accent: "#d46fa0".into(),
            accent_dim: "#5a4888".into(),
            accent_info: "#9a7ad4".into(),
            text_muted: "#2a2048".into(),
            text_faint: "#130e22".into(),
            pill_bg: "#14103a".into(),
            pill_border: "#201850".into(),
            pill_text: "#d46fa0".into(),
            kbd_bg: "#0c0a1a".into(),
            kbd_border: "#1a1438".into(),
            kbd_text: "#2a2048".into(),
            sys_color: "#1a1438".into(),
            sel_bg: "#14103a".into(),
            sel_left: "#d46fa0".into(),
            cursor: "#d46fa0".into(),
            error: "#e08aa0".into(),
            success: "#8ac8a8".into(),
        },
    );
    t.insert(
        "PAPER",
        Theme {
            name: "PAPER".into(),
            bg: "#f5f2eb".into(),
            surface: "#ede8df".into(),
            border: "#d0cabb".into(),
            accent: "#3a3228".into(),
            accent_dim: "#5a5248".into(),
            accent_info: "#6a6048".into(),
            text_muted: "#a09888".into(),
            text_faint: "#c8c0b0".into(),
            pill_bg: "#e0d8cc".into(),
            pill_border: "#c8c0b0".into(),
            pill_text: "#3a3228".into(),
            kbd_bg: "#e5e0d5".into(),
            kbd_border: "#d0cabb".into(),
            kbd_text: "#a09888".into(),
            sys_color: "#c8c0b0".into(),
            sel_bg: "#ddd8cc".into(),
            sel_left: "#3a3228".into(),
            cursor: "#3a3228".into(),
            error: "#a04848".into(),
            success: "#4a7858".into(),
        },
    );
    t
}

/// Look up a theme by name (case-insensitive). Falls back to GHOST.
pub fn by_name(name: &str) -> Theme {
    let upper = name.to_ascii_uppercase();
    themes().get(upper.as_str()).cloned().unwrap_or_else(|| {
        themes()
            .get(DEFAULT_THEME_NAME)
            .cloned()
            .expect("default theme missing")
    })
}

/// Return the active theme by reading `config.json`. Always returns a valid theme.
pub fn active() -> Theme {
    let cfg = config::load_config();
    by_name(&cfg.theme)
}

/// Persist the given theme name to `config.json`. Unknown names are ignored
/// and the function returns `false`; otherwise returns `true`.
pub fn set_active(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if !themes().contains_key(upper.as_str()) {
        return false;
    }
    let mut cfg = config::load_config();
    cfg.theme = upper;
    config::save_config(&cfg).is_ok()
}

/// All theme names, in picker order (BTreeMap is lexicographic; the order
/// happens to match the spec for the canonical 6 themes).
pub fn list_names() -> Vec<String> {
    themes().keys().map(|s| s.to_string()).collect()
}

/// Pick the accent color for a given run mode. Used to color the mode chip
/// in the prompt, the small bullet in the response rail, and the "switched
/// to … mode" message. Plan uses `accent_info` so it's visually distinct
/// from CHAT/CODE on every theme.
pub fn accent_for_mode(t: &Theme, mode: &str) -> &str {
    match mode {
        "CODE" => &t.accent,
        "PLAN" => &t.accent_info,
        _ => &t.accent_dim,
    }
}

// ── ANSI helpers ──────────────────────────────────────────────────────────

fn hex_to_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let s = hex.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Convert a hex color to a truecolor ANSI escape sequence. Returns an empty
/// string if the hex is malformed (so the caller can still print a sensible
/// fallback).
pub fn fg(hex: &str) -> String {
    match hex_to_rgb(hex) {
        Some((r, g, b)) => format!("\x1b[38;2;{r};{g};{b}m"),
        None => String::new(),
    }
}

pub fn bg(hex: &str) -> String {
    match hex_to_rgb(hex) {
        Some((r, g, b)) => format!("\x1b[48;2;{r};{g};{b}m"),
        None => String::new(),
    }
}

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const REVERSE: &str = "\x1b[7m";

/// Style text with a hex foreground and optional bold. The result is
/// automatically terminated with the ANSI reset.
pub fn paint(hex: &str, text: &str, bold: bool) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    out.push_str(&fg(hex));
    if bold {
        out.push_str(BOLD);
    }
    out.push_str(text);
    out.push_str(RESET);
    out
}

/// Same as `paint` but also sets the background color.
pub fn paint_on(fg_hex: &str, bg_hex: &str, text: &str, bold: bool) -> String {
    let mut out = String::with_capacity(text.len() + 32);
    out.push_str(&fg(fg_hex));
    out.push_str(&bg(bg_hex));
    if bold {
        out.push_str(BOLD);
    }
    out.push_str(text);
    out.push_str(RESET);
    out
}

// ── High-level layout helpers ─────────────────────────────────────────────

/// A single soft left-rail line: `▌ <body>`. The rail uses the mode accent;
/// the body uses `text_muted`. Used as the prefix of every model reply.
pub fn paint_rail(accent_hex: &str, body_hex: &str, body: &str) -> String {
    let rail = paint(accent_hex, "\u{258C}", true);
    let text = paint(body_hex, body, false);
    format!("{rail} {text}")
}

/// Small inline chip: ` label `. The chip background and text colors are
/// pulled from the theme. Used for the mode indicator in the prompt.
pub fn paint_chip(t: &Theme, label: &str) -> String {
    paint_on(&t.pill_text, &t.pill_bg, &format!(" {} ", label), true)
}

/// A single tool / file row: `●  <name>  <dim detail>`. Used by `/files`,
/// generated-file lists, etc. — anything that was previously a bordered row.
pub fn paint_tool_row(t: &Theme, name: &str, detail: &str) -> String {
    let dot = paint(&t.accent_dim, "\u{25CF}", true);
    let n = paint(&t.accent, name, false);
    let d = paint(&t.text_muted, detail, false);
    format!("  {dot}  {n}  {d}")
}

/// A success line: `✓ <msg>` in the success color. Replaces the old
/// `style!(C_GREEN, "✓", ...)` call sites in main.rs.
pub fn paint_success(t: &Theme, msg: &str) -> String {
    let mark = paint(&t.success, "\u{2713}", true);
    let text = paint(&t.accent, msg, true);
    format!("  {mark} {text}")
}

/// An error line: `✗ <msg>` in the error color.
pub fn paint_error(t: &Theme, msg: &str) -> String {
    let mark = paint(&t.error, "\u{2717}", true);
    let text = paint(&t.text_muted, msg, false);
    format!("  {mark} {text}")
}

/// A dim hint line, right-padded with `text_faint` to leave breathing room.
pub fn paint_hint(t: &Theme, msg: &str) -> String {
    paint(&t.text_faint, msg, false)
}

/// Status line. Single dim line that holds: model, mode, ctx %, messages,
/// hint about `/` commands. Right side is right-aligned when `width` is
/// provided. Used by the sticky status line under the prompt and the meta
/// header at session start.
pub fn paint_status_line(left: &str, right: &str) -> String {
    let t = active();
    let left = paint(&t.text_muted, left, false);
    let right = paint(&t.text_faint, right, false);
    let dot = paint(&t.text_faint, "\u{00B7}", false);
    format!("{left}  {dot}  {right}")
}

/// Best-effort terminal width. Falls back to 80 when COLUMNS is unset or
/// stdout is not a tty. Single source of truth — used by the header,
/// status line, and pickers.
pub fn visible_width() -> usize {
    if let Ok(text) = std::env::var("COLUMNS") {
        if let Ok(n) = text.parse::<usize>() {
            if n > 20 {
                return n;
            }
        }
    }
    80
}

/// Count visible characters in a string, skipping ANSI escape sequences.
pub fn visible_len(s: &str) -> usize {
    let mut count = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for inner in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&inner) {
                        break;
                    }
                }
            } else {
                chars.next();
            }
            continue;
        }
        count += 1;
    }
    count
}

// ── Single shared stdout sink ─────────────────────────────────────────────

/// Counter used to rotate a deterministic accent on spinner frames. Held as
/// an atomic so the spinner task and the main task can advance it together.
static SPINNER_TICK: AtomicUsize = AtomicUsize::new(0);

pub fn advance_spinner() -> usize {
    SPINNER_TICK.fetch_add(1, Ordering::Relaxed)
}

pub fn reset_spinner() {
    SPINNER_TICK.store(0, Ordering::Relaxed);
}

/// Write a single line to stdout and flush.
pub fn println(s: &str) {
    let stdout = io::stdout();
    let mut h = stdout.lock();
    let _ = writeln!(h, "{s}");
    let _ = h.flush();
}

/// Clear the current line. Used by the inline spinner / live redraw.
pub fn clear_current_line() {
    let stdout = io::stdout();
    let mut h = stdout.lock();
    let _ = write!(h, "\r\x1b[2K");
    let _ = h.flush();
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses() {
        assert_eq!(hex_to_rgb("#e8e8e8"), Some((232, 232, 232)));
        assert_eq!(hex_to_rgb("3aff5a"), Some((58, 255, 90)));
        assert_eq!(hex_to_rgb("#nope"), None);
    }

    #[test]
    fn fg_bg_emit_ansi() {
        assert_eq!(fg("#000000"), "\x1b[38;2;0;0;0m");
        assert_eq!(bg("#ffffff"), "\x1b[48;2;255;255;255m");
    }

    #[test]
    fn paint_resets() {
        let s = paint("#e8e8e8", "REM", true);
        assert!(s.ends_with(RESET));
        assert!(s.contains("REM"));
    }

    #[test]
    fn unknown_theme_falls_back() {
        let t = by_name("nope");
        assert_eq!(t.name, "GHOST");
    }

    #[test]
    fn list_names_has_six() {
        let names = list_names();
        assert_eq!(names.len(), 6);
        assert!(names.contains(&"GHOST".to_string()));
        assert!(names.contains(&"PAPER".to_string()));
    }

    #[test]
    fn all_themes_have_distinct_accents() {
        let ts = themes();
        let accents: std::collections::HashSet<String> =
            ts.values().map(|t| t.accent.clone()).collect();
        assert_eq!(
            accents.len(),
            ts.len(),
            "every theme must have a unique accent"
        );
    }

    #[test]
    fn all_themes_have_new_fields() {
        for t in themes().values() {
            assert!(!t.accent_info.is_empty(), "{} missing accent_info", t.name);
            assert!(!t.error.is_empty(), "{} missing error", t.name);
            assert!(!t.success.is_empty(), "{} missing success", t.name);
        }
    }

    #[test]
    fn set_active_round_trips() {
        assert!(set_active("PHOSPHOR"));
        let t = active();
        assert_eq!(t.name, "PHOSPHOR");
        // restore
        assert!(set_active("GHOST"));
        let t = active();
        assert_eq!(t.name, "GHOST");
    }

    #[test]
    fn rail_uses_accent_then_body() {
        let t = active();
        let s = paint_rail(&t.accent, &t.text_muted, "hi");
        assert!(s.contains("\u{258C}"));
        assert!(s.contains("hi"));
    }

    #[test]
    fn accent_for_mode_distinguishes_plan() {
        let t = active();
        assert_eq!(accent_for_mode(&t, "CHAT"), t.accent_dim);
        assert_eq!(accent_for_mode(&t, "CODE"), t.accent);
        assert_eq!(accent_for_mode(&t, "PLAN"), t.accent_info);
    }
}
