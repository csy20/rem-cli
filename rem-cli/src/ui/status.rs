// ── ui/status.rs ──
//
// Sticky status line. One dim line, always rendered immediately above the
// prompt, that surfaces: active model, mode, token estimate, context %,
// message count, and the `/?` hint. Re-rendered each loop cycle so the
// token / context numbers stay current.
#![allow(dead_code)]

use crate::ui::theme;

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub tokens: u32,
    pub context_pct: u8,
    pub messages: u32,
    pub last_duration_s: f64,
}

impl Status {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_tokens(mut self, tokens: u32) -> Self {
        self.tokens = tokens;
        self
    }

    pub fn with_context_pct(mut self, pct: u8) -> Self {
        self.context_pct = pct.min(100);
        self
    }

    pub fn with_messages(mut self, n: u32) -> Self {
        self.messages = n;
        self
    }

    pub fn with_duration(mut self, secs: f64) -> Self {
        self.last_duration_s = secs;
        self
    }
}

/// Build the dim status line as a single string. Does not print. The REPL
/// loop calls this just before it draws the prompt so the user always sees
/// the most recent token / context numbers.
pub fn build(model: &str, mode: &str, status: &Status) -> String {
    let t = theme::active();
    let arrow = theme::paint(&t.accent, ">", true);
    let word = theme::paint(&t.accent, "rem", true);
    let dot = theme::paint(&t.text_faint, "\u{00B7}", false);

    let model_lbl = theme::paint(&t.text_faint, "model", false);
    let model_val = theme::paint(&t.text_muted, model, false);

    let mode_chip = theme::paint_chip(&t, &mode.to_ascii_lowercase());

    let tokens = if status.tokens == 0 {
        "0".to_string()
    } else if status.tokens < 1000 {
        format!("{}", status.tokens)
    } else {
        format!("{:.1}K", status.tokens as f64 / 1000.0)
    };
    let ctx_str = format!("{}/8K ctx ({}%)", tokens, status.context_pct);
    let ctx_lbl = theme::paint(&t.text_faint, "ctx", false);
    let ctx_val = theme::paint(&t.text_muted, &ctx_str, false);

    let msgs = format!(
        "{} msg{}",
        status.messages,
        if status.messages == 1 { "" } else { "s" }
    );
    let msgs = theme::paint(&t.text_muted, &msgs, false);

    let slash = theme::paint(&t.text_faint, "/", false);
    let help = theme::paint(&t.text_faint, "commands", false);

    format!(
        "  {arrow} {word}  {dot}  {model_lbl} {model_val}  {dot}  {mode_chip}  {dot}  {msgs}  {dot}  {ctx_lbl} {ctx_val}  {dot}  {slash}  {help}"
    )
}

/// Print the status line directly to stdout.
pub fn render(model: &str, mode: &str, status: &Status) {
    theme::println(&build(model, mode, status));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_includes_model_mode_and_ctx() {
        let s = build(
            "rem-coder",
            "CHAT",
            &Status::new()
                .with_tokens(1200)
                .with_context_pct(15)
                .with_messages(4),
        );
        assert!(s.contains("rem-coder"), "missing model in status: {s}");
        assert!(s.contains("chat"), "missing mode chip in status: {s}");
        assert!(s.contains("1.2K"), "missing token count: {s}");
        assert!(s.contains("15%"), "missing context %: {s}");
        assert!(s.contains("4 msgs"), "missing message count: {s}");
    }

    #[test]
    fn token_scaling() {
        let s = build("m", "CHAT", &Status::new().with_tokens(500));
        assert!(s.contains("500"));
        let s = build("m", "CHAT", &Status::new().with_tokens(2500));
        assert!(s.contains("2.5K"));
    }
}
