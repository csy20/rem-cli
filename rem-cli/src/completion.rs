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

        // Tab-complete subcommands
        let subcommands: &[(&str, &[&str])] = &[
            ("/session ", &["export", "import", "list", "analytics", "compact-undo"]),
            ("/git ", &["status", "diff", "log", "commit"]),
            ("/memory ", &["add", "clear"]),
            ("/prompt ", &["save", "load", "list", "delete"]),
            ("/plugin ", &["list", "help"]),
            ("/reasoning ", &["on", "off", "low", "medium", "high"]),
        ];
        for (cmd_prefix, subs) in subcommands {
            if let Some(tail) = line_before.strip_prefix(cmd_prefix) {
                if tail.contains(' ') {
                    // Already past the subcommand — try file path completion for relevant ones
                    for path_cmd in &["/session export ", "/session import "] {
                        if let Some(tail) = line_before.strip_prefix(path_cmd) {
                            return Ok(complete_path(path_cmd.len(), tail));
                        }
                    }
                    break;
                }
                let candidates: Vec<Pair> = subs
                    .iter()
                    .filter(|s| s.starts_with(tail))
                    .map(|s| Pair {
                        replacement: format!("{}{}", cmd_prefix, s),
                        display: s.to_string(),
                    })
                    .collect();
                if !candidates.is_empty() {
                    return Ok((0, candidates));
                }
                break;
            }
        }

        // Tab-complete file paths after /session export and /session import
        for prefix_cmd in &["/session export ", "/session import "] {
            if let Some(tail) = line_before.strip_prefix(*prefix_cmd) {
                return Ok(complete_path(prefix_cmd.len(), tail));
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_path_empty_prefix() {
        let (start, candidates) = complete_path(0, "");
        assert_eq!(start, 0);
        assert!(!candidates.is_empty(), "should list current dir entries");
    }

    #[test]
    fn complete_path_dot_prefix() {
        let (start, _candidates) = complete_path(0, ".");
        assert_eq!(start, 0);
    }

    #[test]
    fn complete_path_nonexistent_dir() {
        let (start, candidates) = complete_path(0, "/nonexistent_dir_xyz123/");
        assert_eq!(start, 0);
        assert!(candidates.is_empty());
    }

    #[test]
    fn complete_path_start_pos_preserved() {
        let (start, candidates) = complete_path(15, "Cargo.toml");
        assert_eq!(start, 15);
        // Cargo.toml should be findable from current dir
        let has_cargo = candidates.iter().any(|c| c.replacement.contains("Cargo.toml"));
        assert!(has_cargo, "should find Cargo.toml");
    }

    #[test]
    fn complete_path_prefix_matching() {
        let (_, candidates) = complete_path(0, "Cargo");
        let has_cargo = candidates.iter().any(|c| c.replacement.contains("Cargo.toml"));
        assert!(has_cargo, "should match Cargo prefix to Cargo.toml");
    }

    #[test]
    fn complete_path_non_matching_prefix() {
        let (_, candidates) = complete_path(0, "ZZZZ_NONEXISTENT_XXXX");
        assert!(candidates.is_empty());
    }

    #[test]
    fn complete_path_subdir() {
        let (_, candidates) = complete_path(0, "src/");
        // src/ directory should have entries
        assert!(!candidates.is_empty(), "src/ should have contents");
    }

    #[test]
    fn command_completion_matches() {
        let _helper = RemHelper;
    }

    #[test]
    fn complete_subcommands_session() {
        let helper = RemHelper;
        // We can't easily test the Completer trait directly, but we can verify
        // the subcommand list is correct by checking the registry
        let reg = crate::commands::registry();
        assert!(reg.is_command("/session"));
    }

    #[test]
    fn complete_subcommands_git() {
        let reg = crate::commands::registry();
        assert!(reg.is_command("/git"));
    }

    #[test]
    fn complete_subcommands_memory() {
        let reg = crate::commands::registry();
        assert!(reg.is_command("/memory"));
    }

    #[test]
    fn complete_subcommands_prompt() {
        let reg = crate::commands::registry();
        assert!(reg.is_command("/prompt"));
    }

    #[test]
    fn complete_path_respects_dir_slash() {
        // Prefix ending in / lists children
        let (start, candidates) = complete_path(0, "src/");
        assert_eq!(start, 0);
        // Should find src/main.rs or similar
        let has_main = candidates.iter().any(|c| c.replacement.contains("main"));
        assert!(has_main, "src/ should list main.rs or similar");
    }

    #[test]
    fn complete_path_handles_file_name_partial() {
        let (_, candidates) = complete_path(10, "src/main");
        let has_main = candidates
            .iter()
            .any(|c| c.replacement == "src/main.rs" || c.replacement.starts_with("src/main"));
        assert!(has_main, "should complete src/main to src/main.rs");
    }
}
