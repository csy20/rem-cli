//! Spinner and output utilities.
//! Provides [`SpinnerGuard`] for animated terminal spinners during
//! long-running LLM requests, and output formatting ([`print_reply`], [`print_banner`]).

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::LazyLock;

use tokio::task::JoinHandle;

use crate::blocklist::{is_command_blocked, sanitize_commands};
use crate::provider::Provider;
use crate::types::{file_icon, ModelReply};
use crate::ui::theme;

static COLUMNS_WIDTH: LazyLock<usize> = LazyLock::new(|| {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(80usize)
});

/// An animated terminal spinner shown during long-running operations.
pub struct SpinnerGuard {
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SpinnerGuard {
    /// Creates a new spinner with a status message.
    pub fn new(msg: &'static str) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();
        let t = theme::active();
        let glyph_cache: Vec<String> = [
            "\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}", "\u{2834}", "\u{2826}", "\u{2827}", "\u{2807}",
            "\u{280F}",
        ]
        .iter()
        .map(|c| theme::paint(&t, "accent_dim", c, true))
        .collect();
        let label = theme::paint(&t, "text_faint", msg, false);
        let handle = tokio::spawn(async move {
            let mut i = 0usize;
            while r.load(Ordering::SeqCst) {
                eprint!("\r  {}  {}", glyph_cache[i], label);
                let _ = io::stderr().flush();
                tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                i = (i + 1) % glyph_cache.len();
            }
        });
        Self {
            running,
            handle: Some(handle),
        }
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
        eprint!("\r\x1b[2K{}\r", " ".repeat(*COLUMNS_WIDTH));
        let _ = io::stderr().flush();
    }
}

impl Drop for SpinnerGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Prints the REM banner showing the provider and model name.
pub fn print_banner(client: &Provider) {
    let t = theme::active();
    println!();
    theme::println(&theme::paint_rail(&t, "accent", "text_muted", "REM"));
    theme::println(&format!(
        "  {} {} {}  {}",
        theme::paint(&t, "accent_dim", "\u{258C}", true),
        theme::paint(&t, "text_faint", "provider", false),
        theme::paint(&t, "accent", &client.provider_label(), false),
        theme::paint(&t, "text_faint", "\u{00B7} type /help for commands", false)
    ));
}

/// Prints a structured [`ModelReply`] (explanation, files, code, commands, checks).
pub fn print_reply(reply: &ModelReply, newline: bool) {
    let t = theme::active();
    if newline {
        println!();
    }
    if !reply.explanation.trim().is_empty() {
        theme::println(&format!(
            "  {} {}",
            theme::paint(&t, "accent", "\u{258C}", true),
            reply.explanation
        ));
    }

    if !reply.files.is_empty() {
        theme::println(&format!(
            "  {}",
            theme::paint_success(&t, &format!("generated: {} file(s)", reply.files.len()))
        ));
        for f in &reply.files {
            let icon = file_icon(&f.path);
            if f.path.is_empty() {
                theme::println(&format!(
                    "    {}  {}",
                    icon,
                    theme::paint(&t, "accent_dim", &format!("(unnamed) {} bytes", f.content.len()), false)
                ));
            } else {
                theme::println(&format!("    {}  {}", icon, theme::paint(&t, "accent", &f.path, false)));
            }
        }
        theme::println(&format!(
            "    {}",
            theme::paint(&t, "text_faint", "/write <path> to save", false)
        ));
    } else if !reply.code.trim().is_empty() {
        theme::println(&format!("  {}", theme::paint_success(&t, "code:")));
        for code_line in reply.code.lines() {
            theme::println(&format!("    {}", theme::paint(&t, "accent_dim", code_line, false)));
        }
        theme::println(&format!(
            "    {}",
            theme::paint(&t, "text_faint", "/write <path> to save", false)
        ));
    }
    if !reply.commands.is_empty() {
        theme::println(&format!("  {}", theme::paint(&t, "accent", "commands:", true)));
        for cmd in sanitize_commands(&reply.commands) {
            if is_command_blocked(cmd) {
                theme::println(&format!(
                    "    {}",
                    theme::paint_error(&t, &format!("[blocked] {}", cmd))
                ));
            } else {
                theme::println(&format!("    $ {}", theme::paint(&t, "accent_dim", cmd, false)));
            }
        }
    }
    if !reply.checks.is_empty() {
        theme::println(&format!("  {}", theme::paint(&t, "accent", "checks:", true)));
        for item in &reply.checks {
            theme::println(&format!(
                "    {}",
                theme::paint(&t, "text_muted", &format!("\u{2022} {}", item), false)
            ));
        }
    }
    if !reply.caution.trim().is_empty() {
        theme::println(&format!(
            "  {}",
            theme::paint_error(&t, &format!("caution: {}", reply.caution))
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileEntry;

    #[test]
    fn print_reply_empty() {
        let reply = ModelReply {
            explanation: String::new(),
            code: String::new(),
            files: vec![],
            commands: vec![],
            checks: vec![],
            caution: String::new(),
        };
        print_reply(&reply, false);
    }

    #[test]
    fn print_reply_with_explanation() {
        let reply = ModelReply {
            explanation: "Hello world".into(),
            code: String::new(),
            files: vec![],
            commands: vec![],
            checks: vec![],
            caution: String::new(),
        };
        print_reply(&reply, true);
    }

    #[test]
    fn print_reply_with_files() {
        let reply = ModelReply {
            explanation: String::new(),
            code: String::new(),
            files: vec![FileEntry {
                path: "src/main.rs".into(),
                content: "fn main() {}".into(),
            }],
            commands: vec![],
            checks: vec![],
            caution: String::new(),
        };
        print_reply(&reply, false);
    }

    #[test]
    fn print_reply_with_code() {
        let reply = ModelReply {
            explanation: String::new(),
            code: "fn main() {}".into(),
            files: vec![],
            commands: vec![],
            checks: vec![],
            caution: String::new(),
        };
        print_reply(&reply, false);
    }

    #[test]
    fn print_reply_with_commands() {
        let reply = ModelReply {
            explanation: String::new(),
            code: String::new(),
            files: vec![],
            commands: vec!["ls -la".into(), "cargo test".into()],
            checks: vec![],
            caution: String::new(),
        };
        print_reply(&reply, false);
    }

    #[test]
    fn print_reply_with_blocked_command() {
        let reply = ModelReply {
            explanation: String::new(),
            code: String::new(),
            files: vec![],
            commands: vec!["rm -rf / ".into()],
            checks: vec![],
            caution: String::new(),
        };
        print_reply(&reply, false);
    }

    #[test]
    fn print_reply_with_checks() {
        let reply = ModelReply {
            explanation: String::new(),
            code: String::new(),
            files: vec![],
            commands: vec![],
            checks: vec!["Verify the output".into()],
            caution: String::new(),
        };
        print_reply(&reply, false);
    }

    #[test]
    fn print_reply_with_caution() {
        let reply = ModelReply {
            explanation: String::new(),
            code: String::new(),
            files: vec![],
            commands: vec![],
            checks: vec![],
            caution: "This is dangerous".into(),
        };
        print_reply(&reply, false);
    }

    #[test]
    fn print_reply_all_fields() {
        let reply = ModelReply {
            explanation: "Explanation text".into(),
            code: "code block".into(),
            files: vec![FileEntry {
                path: "test.py".into(),
                content: "print('hello')".into(),
            }],
            commands: vec!["python test.py".into()],
            checks: vec!["Run python test.py".into()],
            caution: "Check output".into(),
        };
        print_reply(&reply, true);
    }

    #[tokio::test]
    async fn spinner_guard_create_and_drop() {
        let spinner = SpinnerGuard::new("loading...");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        drop(spinner);
    }

    #[tokio::test]
    async fn spinner_guard_stop_explicitly() {
        let mut spinner = SpinnerGuard::new("testing...");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        spinner.stop();
    }

    #[test]
    fn print_banner_no_panic() {
        let provider = Provider::new(
            crate::provider::ProviderKind::Ollama,
            "http://localhost:11434".into(),
            "llama3".into(),
            30,
            String::new(),
            None,
            4096,
        );
        print_banner(&provider);
    }
}
