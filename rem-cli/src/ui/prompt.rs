use std::io;

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::ui::markdown;
use crate::ui::theme;

pub fn prefix() -> String {
    let t = theme::active();
    let arrow = markdown::prompt_arrow();
    format!("  {arrow} ")
}

pub fn readline(editor: &mut DefaultEditor) -> io::Result<Option<String>> {
    let prompt = prefix();
    match editor.readline(&prompt) {
        Ok(line) => Ok(Some(line)),
        Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => Ok(None),
        Err(e) => Err(io::Error::new(io::ErrorKind::Other, e)),
    }
}
