//! Shared types, models, and utility functions for the REM CLI.
//! Provides [`FileEntry`], [`ModelReply`], path resolution, file icons,
//! text truncation, command sanitization, and related helpers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::parsing::{current_name_from_bold, extract_code_block, guess_filename};
use crate::ui;

// ── Constants ───────────────────────────────────────────────────────────────

pub(crate) const BLOCKED_COMMAND_PATTERNS: [&str; 6] = [
    "rm -rf /",
    "mkfs",
    "dd if=",
    ":(){:|:&};:",
    "shutdown",
    "reboot",
];

/// Compiled regex for `@filename` references in user input.
pub(crate) static RE_AT_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@([^\s]+)").expect("invalid regex literal"));

// ── Model types ─────────────────────────────────────────────────────────────

/// A named code block extracted from an LLM response.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct FileEntry {
    /// Relative file path extracted from `### <path>` heading.
    pub(crate) path: String,
    /// File content from the code fence.
    pub(crate) content: String,
}

/// Parsed response from the LLM (JSON or fallback).
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ModelReply {
    #[serde(default)]
    pub(crate) explanation: String,
    #[serde(default)]
    pub(crate) code: String,
    #[serde(default)]
    pub(crate) files: Vec<FileEntry>,
    #[serde(default)]
    pub(crate) commands: Vec<String>,
    #[serde(default)]
    pub(crate) checks: Vec<String>,
    #[serde(default)]
    pub(crate) caution: String,
}

impl ModelReply {
    /// Creates a [`ModelReply`] by parsing non-JSON LLM output.
    pub(crate) fn fallback(raw_text: &str) -> Self {
        let mut commands = Vec::new();
        for line in raw_text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('$') {
                commands.push(trimmed.trim_start_matches('$').trim().to_string());
            } else if looks_like_shell_command(trimmed) {
                commands.push(trimmed.to_string());
            }
        }
        let files = extract_code_blocks_with_names(raw_text);
        let single_code = extract_code_block(raw_text);
        Self {
            explanation: raw_text.trim().to_string(),
            code: single_code,
            files,
            commands,
            checks: vec!["Verify each step before running.".to_string()],
            caution: "Model returned non-JSON output. Review everything carefully.".to_string(),
        }
    }
}

// ── Code block extraction ───────────────────────────────────────────────────

/// Extracts named code blocks from LLM multi-file output format.
/// Parses `### <path>` headings followed by code fences.
pub(crate) fn extract_code_blocks_with_names(text: &str) -> Vec<FileEntry> {
    let mut files = Vec::new();
    let mut current_name = String::new();
    let mut in_fence = false;
    let mut code_lines: Vec<&str> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("```") {
            if in_fence {
                let content = code_lines.join("\n");
                if !content.trim().is_empty() {
                    let path = if current_name.is_empty() {
                        guess_filename(&code_lines)
                    } else {
                        current_name.clone()
                    };
                    files.push(FileEntry { path, content });
                }
                code_lines.clear();
                current_name.clear();
                in_fence = false;
            } else {
                in_fence = true;
            }
            continue;
        }

        if in_fence {
            code_lines.push(line);
            continue;
        }

        if let Some(name) = trimmed
            .strip_prefix("### ")
            .or_else(|| trimmed.strip_prefix("## "))
        {
            current_name = name.trim().to_string();
            continue;
        }

        if let Some(name) = current_name_from_bold(trimmed) {
            current_name = name;
            continue;
        }
    }

    if in_fence && !code_lines.is_empty() {
        let content = code_lines.join("\n");
        if !content.trim().is_empty() {
            let path = if current_name.is_empty() {
                guess_filename(&code_lines)
            } else {
                current_name.clone()
            };
            files.push(FileEntry { path, content });
        }
    }

    files
}

// ── Path resolution ─────────────────────────────────────────────────────────

/// Resolves a relative path against a base, preventing directory traversal.
pub(crate) fn resolve_safe_path(base: &Path, rel: &str) -> Option<PathBuf> {
    let t = ui::theme::active();
    let expanded = if rel.starts_with('~') {
        if let Some(home) = dirs::home_dir() {
            if rel == "~" || rel.starts_with("~/") {
                home.join(rel.trim_start_matches("~/"))
            } else {
                PathBuf::from(rel)
            }
        } else {
            PathBuf::from(rel)
        }
    } else {
        PathBuf::from(rel)
    };

    let candidate = if expanded.is_relative() {
        base.join(&expanded)
    } else {
        expanded
    };

    let resolved = match std::fs::canonicalize(&candidate) {
        Ok(r) => r,
        Err(_) => {
            let parent = candidate.parent()?;
            let canonical_parent = std::fs::canonicalize(parent).ok()?;
            canonical_parent.join(candidate.file_name()?)
        }
    };

    if resolved.starts_with(base) {
        Some(resolved)
    } else {
        eprintln!(
            "  {} path traversal blocked: {}",
            ui::theme::paint_error_label(&t, "\u{2717}"),
            ui::theme::paint_warning(&t, rel)
        );
        None
    }
}

// ── File icons ──────────────────────────────────────────────────────────────

/// Returns a styled emoji icon for a file path based on extension.
pub(crate) fn file_icon(path: &str) -> String {
    let t = ui::theme::active();
    let emoji = file_icon_for(path);
    ui::theme::paint(&t, "text_muted", emoji, false)
}

fn file_icon_for(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".html") || lower.ends_with(".htm") {
        "\u{1F310}"
    } else if lower.ends_with(".css") {
        "\u{1F3A8}"
    } else if lower.ends_with(".js") || lower.ends_with(".mjs") || lower.ends_with(".ts") {
        "\u{26A1}"
    } else if lower.ends_with(".json") {
        "\u{1F4CB}"
    } else if lower.ends_with(".md") || lower.ends_with(".txt") {
        "\u{1F4C4}"
    } else if lower.ends_with(".py") {
        "\u{1F40D}"
    } else {
        "\u{1F4C4}"
    }
}

// ── Byte/string utilities ───────────────────────────────────────────────────

/// Formats byte counts as human-readable strings (e.g., `1.5K`, `3.2M`).
pub(crate) fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Truncates a string to at most `max` bytes, preserving char boundaries.
pub(crate) fn truncate_bytes(s: &str, max: usize) -> String {
    if max == 0 || s.is_empty() {
        return "[truncated]".to_string();
    }
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    if end == 0 {
        return "[truncated]".to_string();
    }
    format!("{}\n...[truncated]", &s[..end])
}

/// Truncates a string to at most `max_lines` lines.
pub(crate) fn truncate_to_lines(s: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = s.lines().take(max_lines).collect();
    let mut result = lines.join("\n");
    if s.lines().count() > max_lines {
        result.push_str("\n...[truncated]");
    }
    result
}

// ── Timestamp ───────────────────────────────────────────────────────────────

/// Returns the current UTC timestamp as `YYYY-MM-DD HH:MM:SS`.
pub(crate) fn format_timestamp() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = dur.as_secs();

    let days = total_secs / 86400;
    let time_secs = total_secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    let mut y = 1970i64;
    let mut d = days as i64;
    loop {
        let year_days = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
            366
        } else {
            365
        };
        if d < year_days {
            break;
        }
        d -= year_days;
        y += 1;
    }
    let is_leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let month_days = [
        31u64,
        if is_leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1usize;
    let mut day = d as u64;
    for &md in &month_days {
        if day < md {
            break;
        }
        day -= md;
        month += 1;
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y,
        month,
        day + 1,
        hours,
        minutes,
        seconds
    )
}

// ── Command sanitization ────────────────────────────────────────────────────

pub(crate) fn is_command_blocked(cmd: &str) -> bool {
    let lower = cmd.to_lowercase();
    BLOCKED_COMMAND_PATTERNS.iter().any(|p| lower.contains(p))
}

fn looks_like_shell_command(line: &str) -> bool {
    let first = line.split_whitespace().next().unwrap_or_default();
    matches!(
        first,
        "ls" | "pwd"
            | "cd"
            | "mkdir"
            | "cp"
            | "mv"
            | "touch"
            | "cat"
            | "echo"
            | "rm"
            | "find"
            | "grep"
    )
}

pub(crate) fn sanitize_commands(cmds: &[String]) -> Vec<&str> {
    let mut seen = BTreeMap::<String, ()>::new();
    let mut out = Vec::new();
    for cmd in cmds {
        let key = cmd.trim().to_string();
        if key.is_empty() || seen.contains_key(&key) {
            continue;
        }
        seen.insert(key.clone(), ());
        out.push(cmd.trim());
    }
    out
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_dangerous_commands() {
        assert!(is_command_blocked("rm -rf /tmp"));
        assert!(is_command_blocked("shutdown now"));
        assert!(!is_command_blocked("ls -la"));
    }

    #[test]
    fn truncates_string() {
        let out = truncate_bytes("abcdef", 3);
        assert!(out.starts_with("abc"));
    }

    #[test]
    fn command_sanitization_dedups() {
        let input = vec![" ls ".to_string(), "ls".to_string(), "".to_string()];
        let out = sanitize_commands(&input);
        assert_eq!(out, vec!["ls"]);
    }

    #[test]
    fn fallback_extracts_commands() {
        let out = ModelReply::fallback("Use:\nmkdir project\ncd project");
        assert!(out.commands.iter().any(|c| c == "mkdir project"));
    }

    #[test]
    fn fallback_extracts_code_block() {
        let out = ModelReply::fallback("Here:\n```html\n<div>hi</div>\n```\ndone");
        assert_eq!(out.code, "<div>hi</div>");
    }

    #[test]
    fn resolve_safe_path_allows_workspace_relative_path() {
        let base = std::env::temp_dir().join(format!(
            "rem-cli-safe-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).expect("temp base should be created");

        let result = resolve_safe_path(&base, "main.rs");
        assert!(result.is_some());
        let resolved = result.expect("path should resolve");
        assert!(resolved.starts_with(&base));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resolve_safe_path_blocks_parent_traversal() {
        let base = std::env::temp_dir().join(format!(
            "rem-cli-traversal-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).expect("temp base should be created");

        let result = resolve_safe_path(&base, "../escape.txt");
        assert!(result.is_none());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn file_icon_returns_styled_icon() {
        let icon = file_icon("index.html");
        assert!(!icon.is_empty());
    }

    #[test]
    fn human_size_works() {
        assert_eq!(human_size(500), "500");
        assert_eq!(human_size(2048), "2.0K");
        assert_eq!(human_size(5_242_880), "5.0M");
    }

    #[test]
    fn extract_code_blocks_with_names_parses_multi_file() {
        let input = "### src/main.rs\n```rust\nfn main() {}\n```\n### src/lib.rs\n```rust\npub fn hello() {}\n```";
        let files = extract_code_blocks_with_names(input);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(files[1].path, "src/lib.rs");
    }

    #[test]
    fn extract_code_blocks_unnamed_fallback() {
        let input = "```rust\nfn main() {}\n```";
        let files = extract_code_blocks_with_names(input);
        assert!(!files.is_empty());
        assert_eq!(files[0].content, "fn main() {}");
    }

    #[test]
    fn truncate_to_lines_limits_lines() {
        let input = "line1\nline2\nline3\nline4";
        let out = truncate_to_lines(input, 2);
        assert_eq!(out.lines().count(), 3);
        assert!(out.ends_with("[truncated]"));
    }

    #[test]
    fn truncate_to_lines_passes_short() {
        let input = "short";
        let out = truncate_to_lines(input, 10);
        assert_eq!(out, "short");
    }

    #[test]
    fn format_timestamp_returns_valid_format() {
        let ts = format_timestamp();
        assert_eq!(ts.len(), 19);
        assert!(ts.chars().nth(4) == Some('-'));
        assert!(ts.chars().nth(7) == Some('-'));
    }

    #[test]
    fn truncate_bytes_preserves_char_boundaries() {
        let s = "Hell\u{00e9} world";
        let out = truncate_bytes(s, 5);
        assert_eq!(out, "Hell\n...[truncated]");
        assert!(!out.contains('\u{00e9}'));
    }
}
