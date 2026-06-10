use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::ui::theme;

pub struct SpinnerGuard {
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SpinnerGuard {
    pub fn new(msg: &'static str) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();
        let handle = tokio::spawn(async move {
            let chars = ["\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}", "\u{2834}", "\u{2826}", "\u{2827}", "\u{2807}", "\u{280F}"];
            let mut i = 0usize;
            while r.load(Ordering::Relaxed) {
                let t = theme::active();
                let glyph = theme::paint(&t, "accent_dim", chars[i], true);
                let label = theme::paint(&t, "text_faint", msg, false);
                eprint!("\r  {glyph}  {label}");
                let _ = io::stderr().flush();
                tokio::time::sleep(std::time::Duration::from_millis(80)).await;
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

/// Clear the spinner line before final rendering.
pub fn clear_spinner_line() {
    eprint!("\r{}\r", " ".repeat(60));
    let _ = io::stderr().flush();
}
