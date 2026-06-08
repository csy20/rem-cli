// ── ui/header.rs ──
//
// Single-line meta header. Renders the same content as the old boxed panel,
// but with no border / no extra lines. Called once at session start, and
// again after `/mode`, `/model`, `/theme`.
#![allow(dead_code)]

use crate::ui::theme;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const RAM_WARNING: parking_lot_unused::OnceCell<String> = parking_lot_unused::OnceCell::new();

mod parking_lot_unused {
    pub struct OnceCell<T>(std::sync::OnceLock<T>);
    impl<T> OnceCell<T> {
        pub const fn new() -> Self {
            Self(std::sync::OnceLock::new())
        }
        pub fn set(&self, val: T) -> Result<(), T> {
            self.0.set(val)
        }
        pub fn get(&self) -> Option<&T> {
            self.0.get()
        }
    }
}

/// Optional message to print *above* the header (e.g. low-RAM warning).
pub fn set_ram_warning(message: Option<String>) {
    if let Some(msg) = message {
        let _ = RAM_WARNING.set(msg);
    }
}

/// Build the single-line meta header as a string. Used by `render` and the
/// tests. Layout:
///
/// `> rem v0.1.0  ·  model <name>  ·  [ <mode> ]  ·  /  for commands   ?  for help`
pub fn build(model: &str, mode: &str) -> String {
    let t = theme::active();
    let arrow = theme::paint(&t, "accent", ">", true);
    let word = theme::paint(&t, "accent", &format!("rem v{VERSION}"), true);
    let dot = theme::paint(&t, "text_faint", "\u{00B7}", false);

    let model_lbl = theme::paint(&t, "text_faint", "model", false);
    let model_val = theme::paint(&t, "text_muted", model, false);

    let chip = theme::paint_chip(&t, mode);

    let slash_hint = theme::paint(&t, "text_faint", "/", false);
    let slash_help = theme::paint(&t, "text_muted", "commands", false);
    let q_hint = theme::paint(&t, "text_faint", "?", false);
    let q_help = theme::paint(&t, "text_muted", "help", false);

    let left = format!("  {arrow} {word}  {dot}  {model_lbl} {model_val}  {dot}  {chip}");
    let right = format!("{slash_hint}  {slash_help}    {q_hint}  {q_help}");

    let width = theme::visible_width();
    let left_len = theme::visible_len(&left);
    let right_len = theme::visible_len(&right);
    let pad = if width > left_len + right_len + 4 {
        " ".repeat(width - left_len - right_len - 2)
    } else {
        "    ".to_string()
    };

    format!("{left}{pad}{right}")
}

/// Print the header using the active theme.
pub fn render(model: &str, mode: &str) {
    if let Some(warn) = RAM_WARNING.get() {
        let t = theme::active();
        theme::println(&theme::paint(&t, "sys_color", &format!("  ! {warn}"), true));
    }
    theme::println(&build(model, mode));
}

/// Variant of `render` that builds the header for an explicit theme name.
/// Used by the `/theme` picker preview so each row can show what the user's
/// current header would look like under the new theme.
pub fn render_with_theme(model: &str, mode: &str, theme_name: &str) {
    // We swap the active theme transiently is not safe (other readers); we
    // instead rebuild the line by hand using the named theme.
    let t = theme::by_name(theme_name);
    let arrow = theme::paint(&t, "accent", ">", true);
    let word = theme::paint(&t, "accent", &format!("rem v{VERSION}"), true);
    let dot = theme::paint(&t, "text_faint", "\u{00B7}", false);
    let model_lbl = theme::paint(&t, "text_faint", "model", false);
    let model_val = theme::paint(&t, "text_muted", model, false);
    let chip = theme::paint_chip(&t, mode);
    let slash_hint = theme::paint(&t, "text_faint", "/", false);
    let slash_help = theme::paint(&t, "text_muted", "commands", false);
    let q_hint = theme::paint(&t, "text_faint", "?", false);
    let q_help = theme::paint(&t, "text_muted", "help", false);
    let line = format!(
        "  {arrow} {word}  {dot}  {model_lbl} {model_val}  {dot}  {chip}    {slash_hint}  {slash_help}    {q_hint}  {q_help}"
    );
    theme::println(&line);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_is_single_line() {
        let s = build("rem-coder", "CHAT");
        assert!(!s.contains('\n'), "header must be a single line: {s:?}");
        assert!(s.contains("rem v0.1.0"));
        assert!(s.contains("rem-coder"));
        assert!(s.contains("CHAT"));
    }

    #[test]
    fn no_box_drawing_in_header() {
        let s = build("m", "CODE");
        for ch in [
            '\u{256D}', '\u{2500}', '\u{256E}', '\u{2502}', '\u{2570}', '\u{256F}',
        ] {
            assert!(!s.contains(ch), "header should not contain {ch:?}: {s:?}");
        }
    }

    #[test]
    fn render_each_theme_does_not_panic() {
        for name in theme::list_names() {
            render_with_theme("rem-coder", "CHAT", &name);
        }
    }
}
