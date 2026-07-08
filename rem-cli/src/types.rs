//! Shared types, models, and utility functions for the REM CLI.
//! Provides [`FileEntry`], [`ModelReply`], path resolution, file icons,
//! text truncation, command sanitization, and related helpers.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::blocklist::looks_like_shell_command;
use crate::parsing::{current_name_from_bold, extract_code_block, guess_filename};
use crate::ui;

// ── Constants ───────────────────────────────────────────────────────────────

/// Compiled regex for `@filename` references in user input.
/// The filter for http(s) URLs happens at the call site since the `regex`
/// crate does not support lookahead assertions.
pub(crate) static RE_AT_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@([^\s]+)").expect("invalid regex literal"));

/// Tracks a written file and its original content for safe undo.
#[derive(Debug, Clone)]
pub(crate) struct BackupEntry {
    pub(crate) path: PathBuf,
    /// Original content before overwrite (None = file didn't exist).
    pub(crate) original: Option<String>,
}

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
                commands.push(trimmed.strip_prefix('$').unwrap_or(trimmed).trim().to_string());
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

        if let Some(name) = trimmed.strip_prefix("### ").or_else(|| trimmed.strip_prefix("## ")) {
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
/// Uses parent directory canonicalization for paths that don't exist yet
/// to avoid TOCTOU issues.
pub(crate) fn resolve_safe_path(base: &Path, rel: &str) -> Option<PathBuf> {
    let t = ui::theme::active();
    let expanded = if rel.starts_with('~') {
        if let Some(home) = dirs::home_dir() {
            if rel == "~" {
                home
            } else if rel.starts_with("~/") {
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

    // Try canonicalize directly; on failure (e.g. file doesn't exist yet),
    // canonicalize the parent directory instead. This avoids TOCTOU between
    // exists() and canonicalize().
    let resolved = match std::fs::canonicalize(&candidate) {
        Ok(p) => p,
        Err(_) => {
            let parent = candidate.parent()?;
            let file_name = candidate.file_name()?;
            let canonical_parent = std::fs::canonicalize(parent).ok()?;
            canonical_parent.join(file_name)
        }
    };

    let canonical_base = std::fs::canonicalize(base).ok()?;
    if resolved.starts_with(&canonical_base) {
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
    if path.ends_with(".html") || path.ends_with(".htm") || path.ends_with(".HTML") || path.ends_with(".HTM") {
        "\u{1F310}"
    } else if path.ends_with(".css") || path.ends_with(".CSS") {
        "\u{1F3A8}"
    } else if path.ends_with(".js")
        || path.ends_with(".JS")
        || path.ends_with(".ts")
        || path.ends_with(".TS")
        || path.ends_with(".mjs")
        || path.ends_with(".MJS")
    {
        "\u{26A1}"
    } else if path.ends_with(".json") || path.ends_with(".JSON") {
        "\u{1F4CB}"
    } else if path.ends_with(".md") || path.ends_with(".MD") || path.ends_with(".txt") || path.ends_with(".TXT") {
        "\u{1F4C4}"
    } else if path.ends_with(".py") || path.ends_with(".PY") {
        "\u{1F40D}"
    } else {
        "\u{1F4C4}"
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── BackupEntry tests ────────────────────────────────────────────

    #[test]
    fn backup_entry_new_file_has_no_original() {
        let dir = std::env::temp_dir().join(format!("rem-test-be-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("new_file.txt");
        let original = std::fs::read_to_string(&path).ok();
        let entry = BackupEntry {
            path: path.clone(),
            original,
        };
        assert!(entry.original.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_entry_existing_file_captures_original() {
        let dir = std::env::temp_dir().join(format!("rem-test-be2-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("existing.txt");
        std::fs::write(&path, "original content").unwrap();
        let original = std::fs::read_to_string(&path).ok();
        let entry = BackupEntry {
            path: path.clone(),
            original,
        };
        assert_eq!(entry.original.as_deref(), Some("original content"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prop_resolve_safe_path_never_escapes_workspace() {
        proptest::proptest!(|(workspace in "[a-z]{1,10}", sub_path in "[a-z/]{1,20}")| {
            let workspace_dir = std::env::temp_dir().join(format!("rem-proptest-{workspace}"));
            let _ = std::fs::create_dir_all(&workspace_dir);
            let target = workspace_dir.join(&sub_path);
            let result = resolve_safe_path(&workspace_dir, target.to_str().unwrap_or("/tmp/test"));
            if let Some(p) = result {
                assert!(p.starts_with(&workspace_dir) || p == workspace_dir,
                    "resolve_safe_path should stay within workspace");
            }
            let _ = std::fs::remove_dir_all(&workspace_dir);
        });
    }

    #[test]
    fn prop_file_icon_never_empty_for_known_extensions() {
        proptest::proptest!(|(ext in "[a-z]{1,5}")| {
            let path = format!("test.{}", ext);
            let icon = file_icon(&path);
            assert!(!icon.is_empty(), "file_icon for .{} should return non-empty", ext);
        });
    }
}
