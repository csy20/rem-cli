// ── ui/output.rs ──
//
// Streaming response renderer + reply panel + spinner. All framing is
// gone: model replies render as a single soft left-rail line plus an
// indented body and a one-line status footer. No top or bottom border,
// no `┐─┤` decoration.
#![allow(dead_code)]

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;

use crate::ui::theme;

const CODE_FENCE: &str = "\u{2502}";

/// Stream the model response. `tokens` is a blocking iterator over string
/// chunks (the LLM SSE payload already chunked by the caller). The
/// function blocks until the iterator is exhausted. Returns the full
/// concatenated string.
pub fn stream<S, I>(model: &str, tokens: I) -> String
where
    S: Into<String>,
    I: IntoIterator<Item = S>,
{
    let mut full = String::new();
    let mut buffer: Vec<String> = Vec::new();
    let mut first_token = true;
    let start = Instant::now();
    let mut last_redraw = Instant::now();

    for chunk in tokens {
        let chunk = chunk.into();
        full.push_str(&chunk);

        if first_token {
            theme::clear_current_line();
            first_token = false;
        }

        buffer.push(chunk);

        if last_redraw.elapsed() >= Duration::from_millis(40) {
            render_live(&buffer, model);
            last_redraw = Instant::now();
        }
    }

    if first_token {
        theme::clear_current_line();
    } else {
        render_live(&buffer, model);
    }

    let elapsed = start.elapsed();
    print_final(&full, model, elapsed);
    full
}

/// Render a "thinking..." spinner line. Returns a guard that, when
/// dropped, erases the spinner line. Use before the first token arrives.
pub struct SpinnerGuard {
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SpinnerGuard {
    pub fn new(msg: &'static str) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();
        let handle = tokio::spawn(async move {
            let chars = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut i = 0usize;
            while r.load(Ordering::Relaxed) {
                let t = theme::active();
                let glyph = theme::paint(&t, "accent", chars[i], true);
                let label = theme::paint(&t, "text_muted", msg, false);
                eprint!("\r  {glyph}  {label}");
                let _ = io::stderr().flush();
                tokio::time::sleep(Duration::from_millis(80)).await;
                i = (i + 1) % chars.len();
            }
        });
        Self {
            running,
            handle: Some(handle),
        }
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            h.abort();
        }
        eprint!("\r{}\r", " ".repeat(60));
        let _ = io::stderr().flush();
    }
}

impl Drop for SpinnerGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

fn render_live(buffer: &[String], _model: &str) {
    let t = theme::active();
    let mut out = io::stdout().lock();
    let _ = write!(out, "\r\x1b[2K");
    let rail = theme::paint_rail(&t, "accent_dim", "text_muted", &flatten_chunks(buffer));
    let _ = writeln!(out, "{rail}");
    let _ = out.flush();
}

fn flatten_chunks(buffer: &[String]) -> String {
    let mut s = String::new();
    for c in buffer {
        s.push_str(c);
    }
    s
}

fn print_final(text: &str, model: &str, elapsed: Duration) {
    let t = theme::active();
    let mode = crate::config::load_config().mode;
    let accent_field = theme::accent_for_mode(&mode);

    let mut in_fence = false;
    let mut first = true;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            if in_fence {
                let lang = trimmed.trim_start_matches('`').trim();
                let header = if lang.is_empty() {
                    "code".to_string()
                } else {
                    lang.to_ascii_lowercase()
                };
                let tag = theme::paint(&t, "text_faint", &format!("{CODE_FENCE} {header}"), false);
                theme::println(&format!("  {tag}"));
            }
            continue;
        }
        if in_fence {
            let body = theme::paint(&t, "accent_dim", line, false);
            theme::println(&format!("  {CODE_FENCE}   {body}"));
        } else if first {
            let rail = theme::paint_rail(&t, accent_field, "text_muted", line);
            theme::println(&format!("  {rail}"));
            first = false;
        } else {
            // continuation line, indented under the rail
            let body = theme::paint(&t, "text_muted", line, false);
            theme::println(&format!("    {body}"));
        }
    }
    if first {
        // model returned an empty response — still print an empty rail so
        // the footer doesn't float unattached.
        let rail = theme::paint(&t, accent_field, "\u{258C}", true);
        theme::println(&format!("  {rail}"));
    }

    // One-line status footer.
    let dot = theme::paint(&t, "text_faint", "\u{00B7}", false);
    let model_lbl = theme::paint(&t, "text_faint", "model", false);
    let model_val = theme::paint(&t, "text_muted", model, false);
    let dur = theme::paint(
        &t,
        "text_muted",
        &format!("{:.1}s", elapsed.as_secs_f64()),
        false,
    );
    theme::println(&format!("  {dot}  {model_lbl} {model_val}  {dot}  {dur}"));
}

/// Render a single completed response as a themed reply block. Used by
/// the one-shot subcommands (`ask`, `explain`, `patch`) where there is no
/// streaming — just a final block of text from the model.
pub fn print_reply_panel(text: &str, model: &str) {
    let t = theme::active();
    let mode = crate::config::load_config().mode;
    let accent_field = theme::accent_for_mode(&mode);

    let dot = theme::paint(&t, "text_faint", "\u{00B7}", false);
    let model_lbl = theme::paint(&t, "text_faint", "model", false);
    let model_val = theme::paint(&t, "text_muted", model, false);
    theme::println(&format!("  {dot}  {model_lbl} {model_val}"));

    let mut in_fence = false;
    let mut first = true;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            if in_fence {
                let lang = trimmed.trim_start_matches('`').trim();
                let header = if lang.is_empty() {
                    "code".to_string()
                } else {
                    lang.to_ascii_lowercase()
                };
                let tag = theme::paint(&t, "text_faint", &format!("{CODE_FENCE} {header}"), false);
                theme::println(&format!("  {tag}"));
            }
            continue;
        }
        if in_fence {
            let body = theme::paint(&t, "accent_dim", line, false);
            theme::println(&format!("  {CODE_FENCE}   {body}"));
        } else if first {
            let rail = theme::paint_rail(&t, accent_field, "text_muted", line);
            theme::println(&format!("  {rail}"));
            first = false;
        } else {
            let body = theme::paint(&t, "text_muted", line, false);
            theme::println(&format!("    {body}"));
        }
    }
    if first {
        let rail = theme::paint(&t, accent_field, "\u{258C}", true);
        theme::println(&format!("  {rail}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_drains_iter() {
        let text = stream("test-model", vec!["hello ", "world"]);
        assert_eq!(text, "hello world");
    }

    #[test]
    fn print_reply_panel_does_not_panic() {
        print_reply_panel("hello world", "test-model");
        print_reply_panel("```rust\nfn main() {}\n```", "test-model");
    }

    #[test]
    fn print_reply_panel_uses_rail_not_box() {
        // Render to a string by redirecting stdout is non-trivial in tests,
        // so we just assert that the helpers exist and don't panic. The
        // visible-shape assertions live in tests/ui_rail.rs.
        print_reply_panel("line one\nline two", "m");
    }
}
