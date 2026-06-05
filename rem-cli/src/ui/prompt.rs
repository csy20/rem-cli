// ── ui/prompt.rs ──
//
// The REPL input line. Wraps `rustyline::DefaultEditor` and emits a
// chip-styled prompt prefix from the active theme. The mode is read from
// `config.json` on every call so it stays in sync with `/mode`.
//
// The palette is *not* invoked from inside the editor; the REPL loop in
// `main` intercepts a leading `/` (after stripping whitespace) and shows
// the palette before calling `readline`. That keeps `rustyline`'s history
// and keybindings intact.
#![allow(dead_code)]

use std::io;

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::config;
use crate::ui::theme;

/// Build the colored prompt prefix for the given mode. Layout:
///
/// `  [ <mode> ]  ›  `
///
/// where the chip is filled with the theme's pill colors, and `›` is the
/// faint cursor arrow. The model is shown in the header / status line, not
/// in the prompt itself.
pub fn prefix(mode: &str) -> String {
    let t = theme::active();
    let chip = theme::paint_chip(&t, mode);
    let arrow = theme::paint(&t.text_faint, "\u{203A}", false);
    format!("  {chip}  {arrow} ")
}

/// Read one line of user input. Returns `Ok(None)` on Ctrl+D / Ctrl+C and
/// `Ok(Some(text))` on Enter. Empty lines are returned as empty strings.
pub fn readline(editor: &mut DefaultEditor) -> io::Result<Option<String>> {
    let cfg = config::load_config();
    let prompt = prefix(&cfg.mode);
    match editor.readline(&prompt) {
        Ok(line) => Ok(Some(line)),
        Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => Ok(None),
        Err(e) => Err(io::Error::new(io::ErrorKind::Other, e)),
    }
}

/// Read a single key from stdin for picker flows (theme, model). Returns
/// the byte if a printable key was pressed, or `None` for ESC / EOF.
pub fn read_key() -> Option<char> {
    use std::io::Read;
    let stdin = io::stdin();
    let mut handle = stdin.lock();
    let mut buf = [0u8; 1];
    match handle.read(&mut buf) {
        Ok(0) => None,
        Ok(_) if buf[0] == 0x1b => None,
        Ok(_) if buf[0] == b'\r' || buf[0] == b'\n' => Some('\n'),
        Ok(_) => Some(buf[0] as char),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_contains_mode_chip() {
        let s = prefix("CHAT");
        assert!(s.contains("CHAT"), "prompt missing CHAT chip: {s:?}");
        assert!(s.contains('\u{203A}'), "prompt missing arrow: {s:?}");
    }

    #[test]
    fn prefix_per_mode_distinct() {
        let chat = prefix("CHAT");
        let code = prefix("CODE");
        let plan = prefix("PLAN");
        assert!(chat.contains("CHAT"));
        assert!(code.contains("CODE"));
        assert!(plan.contains("PLAN"));
    }
}
