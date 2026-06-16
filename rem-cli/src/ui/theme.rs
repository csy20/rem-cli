//! Color theme system.
//! Defines built-in themes (GHOST, PHOSPHOR, MIST, PAPER, SAKURA, EMBER)
//! and provides paint helper functions for ANSI-colored terminal output.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::RwLock;

const DEFAULT_THEME_NAME: &str = "GHOST";

/// Returns true if NO_COLOR or CLICOLOR=0 is set (disables ANSI output).
fn no_color() -> bool {
    std::env::var("NO_COLOR").is_ok()
        || std::env::var("CLICOLOR").map(|v| v == "0").unwrap_or(false)
}

/// Custom theme definition for loading from TOML files.
#[derive(Debug, serde::Deserialize)]
struct CustomThemeDef {
    name: String,
    bg: String,
    surface: String,
    border: String,
    accent: String,
    accent_dim: String,
    accent_info: String,
    text_muted: String,
    text_faint: String,
    pill_bg: String,
    pill_border: String,
    pill_text: String,
    kbd_bg: String,
    kbd_border: String,
    kbd_text: String,
    sys_color: String,
    sel_bg: String,
    sel_left: String,
    cursor: String,
    error: String,
    success: String,
    code_bg: String,
}

static CUSTOM_THEMES: LazyLock<Mutex<BTreeMap<String, Theme>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static CUSTOM_THEMES_LOADED: AtomicBool = AtomicBool::new(false);

/// Loads a custom theme from a TOML file.
fn load_custom_theme(path: &PathBuf) -> Option<(String, Theme)> {
    let text = std::fs::read_to_string(path).ok()?;
    let def: CustomThemeDef = toml::from_str(&text).ok()?;
    let theme = Theme::build(
        &def.name,
        &def.bg,
        &def.surface,
        &def.border,
        &def.accent,
        &def.accent_dim,
        &def.accent_info,
        &def.text_muted,
        &def.text_faint,
        &def.pill_bg,
        &def.pill_border,
        &def.pill_text,
        &def.kbd_bg,
        &def.kbd_border,
        &def.kbd_text,
        &def.sys_color,
        &def.sel_bg,
        &def.sel_left,
        &def.cursor,
        &def.error,
        &def.success,
        &def.code_bg,
    );
    Some((def.name.to_uppercase(), theme))
}

/// Loads all custom themes from `~/.config/rem-cli/themes/`.
fn load_custom_themes() {
    if CUSTOM_THEMES_LOADED.swap(true, Ordering::SeqCst) {
        return;
    }
    let theme_dir = dirs::home_dir().map(|h| h.join(".config/rem-cli/themes"));
    let Some(dir) = theme_dir else { return };
    if !dir.exists() {
        return;
    }
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut custom = CUSTOM_THEMES.lock().unwrap_or_else(|e| e.into_inner());
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            if let Some((name, theme)) = load_custom_theme(&path) {
                custom.insert(name, theme);
            }
        }
    }
}

static THEMES: LazyLock<BTreeMap<&'static str, Theme>> = LazyLock::new(|| {
    let mut t = BTreeMap::new();
    t.insert(
        "GHOST",
        Theme::build(
            "GHOST", "#030303", "#0d0d0d", "#181818", "#e8e8e8", "#888888", "#7a8aa0", "#444444",
            "#222222", "#1a1a1a", "#2a2a2a", "#e8e8e8", "#111111", "#1e1e1e", "#333333", "#1e1e1e",
            "#141414", "#e8e8e8", "#888888", "#d87070", "#7ac890", "#0a0a0a",
        ),
    );
    t.insert(
        "PHOSPHOR",
        Theme::build(
            "PHOSPHOR", "#030a04", "#050e06", "#0d2010", "#3aff5a", "#2a8040", "#3acfa0",
            "#1a4020", "#0d2010", "#061409", "#0d2a10", "#3aff5a", "#061409", "#0d2010", "#1a4020",
            "#0d2010", "#0a1e0c", "#3aff5a", "#3aff5a", "#ff6a6a", "#3aff5a", "#050e06",
        ),
    );
    t.insert(
        "MIST",
        Theme::build(
            "MIST", "#0c0f14", "#0f1420", "#1a2538", "#7ba8d4", "#4a6a90", "#8aa0c8", "#2a3a55",
            "#1a2538", "#102040", "#1a3060", "#7ba8d4", "#0f1420", "#1a2538", "#2a3a55", "#1a2538",
            "#102040", "#7ba8d4", "#7ba8d4", "#d49a9a", "#9ac8a8", "#0a0f18",
        ),
    );
    t.insert(
        "PAPER",
        Theme::build(
            "PAPER", "#f5f2eb", "#ede8df", "#d0cabb", "#3a3228", "#5a5248", "#6a6048", "#a09888",
            "#c8c0b0", "#e0d8cc", "#c8c0b0", "#3a3228", "#e5e0d5", "#d0cabb", "#a09888", "#c8c0b0",
            "#ddd8cc", "#3a3228", "#3a3228", "#a04848", "#4a7858", "#e8e4d8",
        ),
    );
    t.insert(
        "SAKURA",
        Theme::build(
            "SAKURA", "#1a1018", "#24151e", "#3a1e30", "#ff8cbc", "#c86090", "#e8a0c0", "#805870",
            "#503848", "#3a1e30", "#4a2840", "#ffb0d0", "#24151e", "#3a1e30", "#805870", "#3a1e30",
            "#2a1420", "#ff8cbc", "#ff8cbc", "#d84860", "#70b890", "#1a0e14",
        ),
    );
    t.insert(
        "EMBER",
        Theme::build(
            "EMBER", "#14100a", "#1e1810", "#302818", "#ff8833", "#cc6620", "#eeaa44", "#887050",
            "#504030", "#302818", "#403020", "#ffbb88", "#1e1810", "#302818", "#887050", "#302818",
            "#201810", "#ff8833", "#ff8833", "#cc3333", "#66aa44", "#0e0c08",
        ),
    );
    t
});

static ACTIVE_THEME: LazyLock<RwLock<Arc<Theme>>> = LazyLock::new(|| {
    let theme = by_name(DEFAULT_THEME_NAME);
    RwLock::new(Arc::new(theme))
});

/// Converts a hex color string to an ANSI foreground escape code.
fn hex_to_ansi_fg(hex: &str) -> String {
    debug_assert!(
        hex.as_bytes().first() == Some(&b'#') && hex.len() == 7,
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

/// Converts a hex color string to an ANSI background escape code.
fn hex_to_ansi_bg(hex: &str) -> String {
    debug_assert!(
        hex.as_bytes().first() == Some(&b'#') && hex.len() == 7,
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

/// A color theme defining ANSI color codes for terminal UI elements.
#[derive(Clone)]
#[allow(dead_code)]
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
    pub code_bg: String,
    fg_cache: BTreeMap<&'static str, String>,
    bg_cache: BTreeMap<&'static str, String>,
}

impl Theme {
    /// Builds a new theme from 21 hex color values.
    #[allow(clippy::too_many_arguments)]
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
        code_bg: &str,
    ) -> Self {
        let field_names: [&str; 21] = [
            "bg",
            "surface",
            "border",
            "accent",
            "accent_dim",
            "accent_info",
            "text_muted",
            "text_faint",
            "pill_bg",
            "pill_border",
            "pill_text",
            "kbd_bg",
            "kbd_border",
            "kbd_text",
            "sys_color",
            "sel_bg",
            "sel_left",
            "cursor",
            "error",
            "success",
            "code_bg",
        ];
        let field_values = [
            bg,
            surface,
            border,
            accent,
            accent_dim,
            accent_info,
            text_muted,
            text_faint,
            pill_bg,
            pill_border,
            pill_text,
            kbd_bg,
            kbd_border,
            kbd_text,
            sys_color,
            sel_bg,
            sel_left,
            cursor,
            error,
            success,
            code_bg,
        ];
        let mut fg_cache = BTreeMap::new();
        let mut bg_cache = BTreeMap::new();
        for (key, hex) in field_names.iter().zip(field_values.iter()) {
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
            code_bg: code_bg.to_string(),
            fg_cache,
            bg_cache,
        }
    }

    /// Returns the ANSI foreground escape code for a named color field.
    /// Returns empty string if NO_COLOR or CLICOLOR=0 is set.
    pub fn fg(&self, field: &str) -> &str {
        if no_color() {
            return "";
        }
        self.fg_cache.get(field).map(|s| s.as_str()).unwrap_or("")
    }

    /// Returns the ANSI background escape code for a named color field.
    /// Returns empty string if NO_COLOR or CLICOLOR=0 is set.
    pub fn bg(&self, field: &str) -> &str {
        if no_color() {
            return "";
        }
        self.bg_cache.get(field).map(|s| s.as_str()).unwrap_or("")
    }
}

/// Looks up a theme by name (case-insensitive), falling back to default.
/// Checks built-in themes first, then custom themes from `~/.config/rem-cli/themes/`.
pub fn by_name(name: &str) -> Theme {
    let upper = name.to_ascii_uppercase();
    if let Some(t) = THEMES.get(upper.as_str()) {
        return t.clone();
    }
    load_custom_themes();
    let custom = CUSTOM_THEMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(t) = custom.get(&upper) {
        return t.clone();
    }
    THEMES
        .get(DEFAULT_THEME_NAME)
        .cloned()
        .expect("default theme missing")
}

/// Returns the currently active theme.
pub fn active() -> Arc<Theme> {
    ACTIVE_THEME
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Sets the active theme by name. Returns false if the name is unknown.
/// Checks built-in themes first, then custom themes.
pub fn set_active(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if THEMES.contains_key(upper.as_str()) {
        let theme = Arc::new(by_name(&upper));
        *ACTIVE_THEME.write().unwrap_or_else(|e| e.into_inner()) = theme;
        return true;
    }
    load_custom_themes();
    let custom = CUSTOM_THEMES.lock().unwrap_or_else(|e| e.into_inner());
    if custom.contains_key(&upper) {
        let theme = Arc::new(by_name(&upper));
        *ACTIVE_THEME.write().unwrap_or_else(|e| e.into_inner()) = theme;
        return true;
    }
    false
}

/// Returns a list of available theme names (built-in + custom).
pub fn list_names() -> Vec<String> {
    let mut names: Vec<String> = THEMES.keys().map(|s| s.to_string()).collect();
    load_custom_themes();
    let custom = CUSTOM_THEMES.lock().unwrap_or_else(|e| e.into_inner());
    names.extend(custom.keys().cloned());
    names.sort();
    names
}

/// Returns the theme accent field name for a given chat mode.
pub fn accent_for_mode(mode: &str) -> &'static str {
    match mode {
        "CODE" => "accent",
        "PLAN" => "accent_info",
        _ => "accent_dim",
    }
}

/// ANSI escape code to reset formatting.
pub const RESET: &str = "\x1b[0m";
/// ANSI escape code for bold text.
pub const BOLD: &str = "\x1b[1m";

/// Paints text in a named theme color, optionally bold.
/// Returns plain text when NO_COLOR or CLICOLOR=0 is set.
pub fn paint(t: &Theme, field: &str, text: &str, bold: bool) -> String {
    if no_color() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 16);
    out.push_str(t.fg(field));
    if bold {
        out.push_str(BOLD);
    }
    out.push_str(text);
    out.push_str(RESET);
    out
}

/// Paints text with foreground and background colors.
/// Returns plain text when NO_COLOR or CLICOLOR=0 is set.
pub fn paint_on(t: &Theme, fg_field: &str, bg_field: &str, text: &str, bold: bool) -> String {
    if no_color() {
        return text.to_string();
    }
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

/// Paints a chip/badge with pill styling.
pub fn paint_chip(t: &Theme, label: &str) -> String {
    paint_on(t, "pill_text", "pill_bg", &format!(" {} ", label), true)
}

/// Paints a success message with checkmark.
pub fn paint_success(t: &Theme, msg: &str) -> String {
    let mark = paint(t, "success", "\u{2713}", true);
    let text = paint(t, "accent", msg, true);
    format!("  {mark} {text}")
}

/// Paints an error message with X mark.
pub fn paint_error(t: &Theme, msg: &str) -> String {
    let mark = paint(t, "error", "\u{2717}", true);
    let text = paint(t, "text_muted", msg, false);
    format!("  {mark} {text}")
}

/// Paints text in the accent color with bold.
pub fn paint_bright(t: &Theme, text: &str) -> String {
    paint(t, "accent", text, true)
}

/// Paints a rail-style line with accent bar and body text.
pub fn paint_rail(t: &Theme, accent_field: &str, body_field: &str, body: &str) -> String {
    let rail = paint(t, accent_field, "\u{258C}", true);
    let text = paint(t, body_field, body, false);
    format!("{rail} {text}")
}

/// Paints an empty rail line (faint bar only).
pub fn paint_rail_empty(t: &Theme) -> String {
    paint(t, "text_faint", "\u{258C}", true)
}

/// Paints a section header with rail styling.
pub fn paint_rail_header(t: &Theme, title: &str) -> String {
    let rail = paint(t, "accent", "\u{258C}", true);
    let title_text = paint(
        t,
        "accent",
        &format!("\u{2500}\u{2500} {title} \u{2500}\u{2500}"),
        true,
    );
    format!("{rail}  {title_text}")
}

/// Paints a help line with command and description.
pub fn paint_help_line(t: &Theme, cmd: &str, desc: &str) -> String {
    let rail = paint(t, "accent", "\u{258C}", true);
    let cmd_text = paint(t, "accent", cmd, true);
    let desc_text = paint(t, "text_faint", desc, false);
    format!("{rail}   {cmd_text:<18} {desc_text}")
}

/// Paints a bullet point with rail.
pub fn paint_rail_bullet(t: &Theme, text: &str) -> String {
    let rail = paint(t, "accent", "\u{258C}", true);
    let dot = paint(t, "text_faint", "\u{2022}", false);
    let body = paint(t, "text_faint", text, false);
    format!("{rail}   {dot} {body}")
}

/// Paints a bullet line with multiple styled segments.
pub fn paint_bullet_line(t: &Theme, parts: &[(&str, &str, bool)]) -> String {
    let rail = paint(t, "accent", "\u{258C}", true);
    let dot = paint(t, "text_faint", "\u{2022}", false);
    let mut out = format!("{rail}   {dot}");
    for (field, text, bold) in parts {
        out.push(' ');
        out.push_str(&paint(t, field, text, *bold));
    }
    out
}

/// Paints text in a dim/faint color.
pub fn paint_dim(t: &Theme, text: &str) -> String {
    paint(t, "text_faint", text, false)
}

/// Paints text as a warning (system color).
pub fn paint_warning(t: &Theme, text: &str) -> String {
    paint(t, "sys_color", text, false)
}

/// Paints an error label in the error color (bold).
pub fn paint_error_label(t: &Theme, text: &str) -> String {
    paint(t, "error", text, true)
}

/// Paints a success label in the success color (bold).
pub fn paint_success_label(t: &Theme, text: &str) -> String {
    paint(t, "success", text, true)
}

/// Prints a string to stdout with newline and flush.
pub fn println(s: &str) {
    let stdout = io::stdout();
    let mut h = stdout.lock();
    let _ = writeln!(h, "{s}");
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
        assert!(names.contains(&"SAKURA".to_string()));
        assert!(names.contains(&"EMBER".to_string()));
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
}
