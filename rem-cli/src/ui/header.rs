//! Welcome header rendering.
//! Displays the REM banner with model name, mode, and version on startup.

use crate::ui::markdown;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Renders the welcome header with model name, mode, and version.
pub fn render(model: &str, mode: &str) {
    for line in markdown::render_welcome(model, mode, VERSION) {
        println!("{line}");
    }
    println!();
}
