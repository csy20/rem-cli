#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::LazyLock;
use std::sync::Mutex;

use crate::config;

const DEFAULT_THEME_NAME: &str = "GHOST";

static THEMES: LazyLock<BTreeMap<&'static str, Theme>> = LazyLock::new(|| {
    let mut t = BTreeMap::new();
    t.insert(
        "GHOST",
        Theme::build(
            "GHOST", "#030303", "#0d0d0d", "#181818", "#e8e8e8", "#888888", "#7a8aa0", "#444444",
            "#222222", "#1a1a1a", "#2a2a2a", "#e8e8e8", "#111111", "#1e1e1e", "#333333", "#1e1e1e",
            "#141414", "#e8e8e8", "#888888", "#d87070", "#7ac890",
        ),
    );
    t.insert(
        "PHOSPHOR",
        Theme::build(
            "PHOSPHOR", "#030a04", "#050e06", "#0d2010", "#3aff5a", "#2a8040", "#3acfa0",
            "#1a4020", "#0d2010", "#061409", "#0d2a10", "#3aff5a", "#061409", "#0d2010", "#1a4020",
            "#0d2010", "#0a1e0c", "#3aff5a", "#3aff5a", "#ff6a6a", "#3aff5a",
        ),
    );
    t.insert(
        "MIST",
        Theme::build(
            "MIST", "#0c0f14", "#0f1420", "#1a2538", "#7ba8d4", "#4a6a90", "#8aa0c8", "#2a3a55",
            "#1a2538", "#102040", "#1a3060", "#7ba8d4", "#0f1420", "#1a2538", "#2a3a55", "#1a2538",
            "#102040", "#7ba8d4", "#7ba8d4", "#d49a9a", "#9ac8a8",
        ),
    );
    t.insert(
        "EMBER",
        Theme::build(
            "EMBER", "#0f0b06", "#161008", "#251a08", "#f0a030", "#7a5520", "#d08850", "#2a2010",
            "#1e1508", "#1e1408", "#302010", "#f0a030", "#161008", "#251a08", "#2a2010", "#251a08",
            "#1e1408", "#f0a030", "#f0a030", "#e08070", "#c8a868",
        ),
    );
    t.insert(
        "SAKURA",
        Theme::build(
            "SAKURA", "#080610", "#0c0a1a", "#1a1438", "#d46fa0", "#5a4888", "#9a7ad4", "#2a2048",
            "#130e22", "#14103a", "#201850", "#d46fa0", "#0c0a1a", "#1a1438", "#2a2048", "#1a1438",
            "#14103a", "#d46fa0", "#d46fa0", "#e08aa0", "#8ac8a8",
        ),
    );
    t.insert(
        "PAPER",
        Theme::build(
            "PAPER", "#f5f2eb", "#ede8df", "#d0cabb", "#3a3228", "#5a5248", "#6a6048", "#a09888",
            "#c8c0b0", "#e0d8cc", "#c8c0b0", "#3a3228", "#e5e0d5", "#d0cabb", "#a09888", "#c8c0b0",
            "#ddd8cc", "#3a3228", "#3a3228", "#a04848", "#4a7858",
        ),
    );
    t
});

static ACTIVE_THEME_CACHE: LazyLock<Mutex<ActiveThemeCache>> = LazyLock::new(|| {
    Mutex::new(ActiveThemeCache {
        name: String::new(),
        theme: None,
    })
});

struct ActiveThemeCache {
    name: String,
    theme: Option<Theme>,
}

fn hex_to_ansi_fg(hex: &str) -> String {
    debug_assert!(
        hex.as_bytes().get(0) == Some(&b'#') && hex.len() == 7,
        "bad hex: {}",
        hex
    );
    let r = u8::from_str_radix(&hex[1..3], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[3..5], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[5..7], 16).unwrap_or(0);
    let mut buf = String::with_capacity(20);
    use std::fmt::Write;
    let _ = write!(buf, "\x1b[38;2;{r};{g};{b}m");
    buf
}

fn hex_to_ansi_bg(hex: &str) -> String {
    debug_assert!(
        hex.as_bytes().get(0) == Some(&b'#') && hex.len() == 7,
        "bad hex: {}",
        hex
    );
    let r = u8::from_str_radix(&hex[1..3], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[3..5], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[5..7], 16).unwrap_or(0);
    let mut buf = String::with_capacity(20);
    use std::fmt::Write;
    let _ = write!(buf, "\x1b[48;2;{r};{g};{b}m");
    buf
}

#[derive(Clone)]
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
    fg_cache: BTreeMap<&'static str, String>,
    bg_cache: BTreeMap<&'static str, String>,
}

impl Theme {
    fn build(
        name: &str,
        bg: &str,
        surface: &str,
        border: &str,
        accent: &str,
        accent_dim: &str,
        accent_info: &str,
        text_muted: &str,
        text_faint: &str,
        pill_bg: &str,
        pill_border: &str,
        pill_text: &str,
        kbd_bg: &str,
        kbd_border: &str,
        kbd_text: &str,
        sys_color: &str,
        sel_bg: &str,
        sel_left: &str,
        cursor: &str,
        error: &str,
        success: &str,
    ) -> Self {
        let fields: [(&str, &str); 20] = [
            ("bg", bg),
            ("surface", surface),
            ("border", border),
            ("accent", accent),
            ("accent_dim", accent_dim),
            ("accent_info", accent_info),
            ("text_muted", text_muted),
            ("text_faint", text_faint),
            ("pill_bg", pill_bg),
            ("pill_border", pill_border),
            ("pill_text", pill_text),
            ("kbd_bg", kbd_bg),
            ("kbd_border", kbd_border),
            ("kbd_text", kbd_text),
            ("sys_color", sys_color),
            ("sel_bg", sel_bg),
            ("sel_left", sel_left),
            ("cursor", cursor),
            ("error", error),
            ("success", success),
        ];
        let mut fg_cache = BTreeMap::new();
        let mut bg_cache = BTreeMap::new();
        for (key, hex) in &fields {
            fg_cache.insert(*key, hex_to_ansi_fg(hex));
            bg_cache.insert(*key, hex_to_ansi_bg(hex));
        }
        Self {
            name: name.to_string(),
            bg: bg.to_string(),
            surface: surface.to_string(),
            border: border.to_string(),
            accent: accent.to_string(),
            accent_dim: accent_dim.to_string(),
            accent_info: accent_info.to_string(),
            text_muted: text_muted.to_string(),
            text_faint: text_faint.to_string(),
            pill_bg: pill_bg.to_string(),
            pill_border: pill_border.to_string(),
            pill_text: pill_text.to_string(),
            kbd_bg: kbd_bg.to_string(),
            kbd_border: kbd_border.to_string(),
            kbd_text: kbd_text.to_string(),
            sys_color: sys_color.to_string(),
            sel_bg: sel_bg.to_string(),
            sel_left: sel_left.to_string(),
            cursor: cursor.to_string(),
            error: error.to_string(),
            success: success.to_string(),
            fg_cache,
            bg_cache,
        }
    }

    pub fn fg(&self, field: &str) -> &str {
        self.fg_cache.get(field).map(|s| s.as_str()).unwrap_or("")
    }

    pub fn bg(&self, field: &str) -> &str {
        self.bg_cache.get(field).map(|s| s.as_str()).unwrap_or("")
    }
}

pub fn themes() -> &'static BTreeMap<&'static str, Theme> {
    &THEMES
}

pub fn by_name(name: &str) -> Theme {
    let upper = name.to_ascii_uppercase();
    THEMES.get(upper.as_str()).cloned().unwrap_or_else(|| {
        THEMES
            .get(DEFAULT_THEME_NAME)
            .cloned()
            .expect("default theme missing")
    })
}

pub fn active() -> Theme {
    let cfg = config::load_config();
    let mut cache = ACTIVE_THEME_CACHE.lock().expect("theme cache lock");
    if cache.name != cfg.theme || cache.theme.is_none() {
        cache.name = cfg.theme.clone();
        cache.theme = Some(by_name(&cache.name));
    }
    cache.theme.clone().expect("active theme resolved")
}

pub fn set_active(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if !THEMES.contains_key(upper.as_str()) {
        return false;
    }
    let mut cfg = config::load_config();
    cfg.theme = upper;
    config::save_config(&cfg).is_ok()
}

pub fn list_names() -> Vec<String> {
    THEMES.keys().map(|s| s.to_string()).collect()
}

pub fn accent_for_mode(mode: &str) -> &'static str {
    match mode {
        "CODE" => "accent",
        "PLAN" => "accent_info",
        _ => "accent_dim",
    }
}

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const REVERSE: &str = "\x1b[7m";

pub fn paint(t: &Theme, field: &str, text: &str, bold: bool) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    out.push_str(t.fg(field));
    if bold {
        out.push_str(BOLD);
    }
    out.push_str(text);
    out.push_str(RESET);
    out
}

pub fn paint_on(t: &Theme, fg_field: &str, bg_field: &str, text: &str, bold: bool) -> String {
    let mut out = String::with_capacity(text.len() + 32);
    out.push_str(t.fg(fg_field));
    out.push_str(t.bg(bg_field));
    if bold {
        out.push_str(BOLD);
    }
    out.push_str(text);
    out.push_str(RESET);
    out
}

pub fn paint_rail(t: &Theme, accent_field: &str, body_field: &str, body: &str) -> String {
    let rail = paint(t, accent_field, "\u{258C}", true);
    let text = paint(t, body_field, body, false);
    format!("{rail} {text}")
}

pub fn paint_chip(t: &Theme, label: &str) -> String {
    paint_on(t, "pill_text", "pill_bg", &format!(" {} ", label), true)
}

pub fn paint_tool_row(t: &Theme, name: &str, detail: &str) -> String {
    let dot = paint(t, "accent_dim", "\u{25CF}", true);
    let n = paint(t, "accent", name, false);
    let d = paint(t, "text_muted", detail, false);
    format!("  {dot}  {n}  {d}")
}

pub fn paint_success(t: &Theme, msg: &str) -> String {
    let mark = paint(t, "success", "\u{2713}", true);
    let text = paint(t, "accent", msg, true);
    format!("  {mark} {text}")
}

pub fn paint_error(t: &Theme, msg: &str) -> String {
    let mark = paint(t, "error", "\u{2717}", true);
    let text = paint(t, "text_muted", msg, false);
    format!("  {mark} {text}")
}

pub fn paint_hint(t: &Theme, msg: &str) -> String {
    paint(t, "text_faint", msg, false)
}

pub fn paint_status_line(left: &str, right: &str) -> String {
    let t = active();
    let left = paint(&t, "text_muted", left, false);
    let right = paint(&t, "text_faint", right, false);
    let dot = paint(&t, "text_faint", "\u{00B7}", false);
    format!("{left}  {dot}  {right}")
}

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

static SPINNER_TICK: AtomicUsize = AtomicUsize::new(0);

pub fn advance_spinner() -> usize {
    SPINNER_TICK.fetch_add(1, Ordering::Relaxed)
}

pub fn reset_spinner() {
    SPINNER_TICK.store(0, Ordering::Relaxed);
}

pub fn println(s: &str) {
    let stdout = io::stdout();
    let mut h = stdout.lock();
    let _ = writeln!(h, "{s}");
    let _ = h.flush();
}

pub fn clear_current_line() {
    let stdout = io::stdout();
    let mut h = stdout.lock();
    let _ = write!(h, "\r\x1b[2K");
    let _ = h.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses() {
        assert_eq!(hex_to_ansi_fg("#e8e8e8"), "\x1b[38;2;232;232;232m");
        assert_eq!(hex_to_ansi_bg("#000000"), "\x1b[48;2;0;0;0m");
    }

    #[test]
    fn fg_bg_emit_ansi() {
        assert_eq!(hex_to_ansi_fg("#000000"), "\x1b[38;2;0;0;0m");
        assert_eq!(hex_to_ansi_bg("#ffffff"), "\x1b[48;2;255;255;255m");
    }

    #[test]
    fn paint_resets() {
        let t = active();
        let s = paint(&t, "accent", "REM", true);
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
        assert!(set_active("GHOST"));
        let t = active();
        assert_eq!(t.name, "GHOST");
    }

    #[test]
    fn rail_uses_accent_then_body() {
        let t = active();
        let s = paint_rail(&t, "accent", "text_muted", "hi");
        assert!(s.contains("\u{258C}"));
        assert!(s.contains("hi"));
    }

    #[test]
    fn accent_for_mode_distinguishes_plan() {
        assert_eq!(accent_for_mode("CHAT"), "accent_dim");
        assert_eq!(accent_for_mode("CODE"), "accent");
        assert_eq!(accent_for_mode("PLAN"), "accent_info");
    }

    #[test]
    fn cached_fg_and_bg() {
        let t = active();
        let fg = t.fg("accent");
        let bg = t.bg("accent");
        assert!(fg.starts_with("\x1b[38;2;"));
        assert!(bg.starts_with("\x1b[48;2;"));
        assert!(!fg.is_empty());
        assert!(!bg.is_empty());
    }
}
