use std::path::Path;

use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Context, Helper};

#[derive(Default)]
pub(crate) struct RemHelper;

impl Completer for RemHelper {
    type Candidate = Pair;

    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> rustyline::Result<(usize, Vec<Pair>)> {
        let line_before = &line[..pos];

        if let Some(at_pos) = line_before.rfind('@') {
            let prefix = &line_before[at_pos + 1..];
            if prefix.starts_with("http") {
                return Ok((0, vec![]));
            }
            return Ok(complete_path(at_pos + 1, prefix));
        }

        if !line_before.contains(' ') && line_before.starts_with('/') {
            let reg = crate::commands::registry();
            let names = reg.command_names();
            let candidates: Vec<Pair> = names
                .iter()
                .filter(|n| n.starts_with(line_before))
                .map(|n| Pair {
                    replacement: n.to_string(),
                    display: n.to_string(),
                })
                .collect();
            return Ok((0, candidates));
        }

        Ok((0, vec![]))
    }
}

fn complete_path(start_pos: usize, prefix: &str) -> (usize, Vec<Pair>) {
    let (dir, partial) = if prefix.is_empty() || prefix.ends_with('/') {
        (Path::new(if prefix.is_empty() { "." } else { prefix }), "")
    } else {
        let path = Path::new(prefix);
        let d = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let p = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        (d, p)
    };

    let dir_prefix = if prefix.is_empty() || prefix.ends_with('/') {
        prefix.to_string()
    } else {
        let parent = Path::new(prefix).parent().and_then(|p| p.to_str()).unwrap_or("");
        if parent.is_empty() {
            String::new()
        } else {
            format!("{}/", parent)
        }
    };

    let Ok(entries) = std::fs::read_dir(dir) else {
        return (start_pos, vec![]);
    };

    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy().into_owned();
        if !name_str.starts_with(partial) {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let display = format!("{}{}{}", dir_prefix, name_str, if is_dir { "/" } else { "" });
        candidates.push(Pair {
            replacement: display.clone(),
            display,
        });
    }

    (start_pos, candidates)
}

impl Hinter for RemHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Option<String> {
        let line_before = &line[..pos];

        if line_before.starts_with('/') && !line_before.contains(' ') {
            let reg = crate::commands::registry();
            let names = reg.command_names();
            let matches: Vec<&&str> = names.iter().filter(|n| n.starts_with(line_before)).collect();
            if matches.len() == 1 {
                let hint = matches[0][line_before.len()..].to_string();
                if !hint.is_empty() {
                    return Some(hint);
                }
            }
        }

        None
    }
}

impl Validator for RemHelper {
    fn validate(&self, _ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        Ok(ValidationResult::Valid(None))
    }
}

impl Highlighter for RemHelper {}

impl Helper for RemHelper {}
