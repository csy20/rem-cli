//! Text parsing utilities.
//! Extracts code blocks from markdown, guesses filenames from content,
//! strips HTML entities, and detects bold-marked file paths.

use regex::Regex;
use std::sync::LazyLock;

static RE_BOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*(.+?)\*\*").expect("invalid regex literal"));

/// Extracts the first fenced code block from markdown text.
pub fn extract_code_block(text: &str) -> String {
    let mut in_fence = false;
    let mut code_lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_fence {
                break;
            }
            in_fence = true;
            continue;
        }
        if in_fence {
            code_lines.push(line);
        }
    }
    if code_lines.is_empty() {
        String::new()
    } else {
        code_lines.join("\n")
    }
}

/// Extracts a filename from a bold markdown segment (e.g., `**file.rs**`).
pub fn current_name_from_bold(line: &str) -> Option<String> {
    if let Some(cap) = RE_BOLD.captures(line) {
        let name = cap.get(1)?.as_str().trim().to_string();
        if name.contains('.') {
            return Some(name);
        }
    }
    None
}

/// Guesses a filename from the first few lines of code content.
pub fn guess_filename(lines: &[&str]) -> String {
    for line in lines.iter().take(3) {
        let trimmed = line.trim();
        if trimmed.starts_with("<!DOCTYPE") || trimmed.starts_with("<html") || trimmed.contains("<head") {
            return "index.html".to_string();
        }
        if trimmed.starts_with("fn ")
            || trimmed.starts_with("pub ")
            || trimmed.starts_with("use ")
            || trimmed.starts_with("mod ")
            || trimmed.starts_with("impl ")
            || trimmed.starts_with("trait ")
            || trimmed.starts_with("#![")
            || trimmed.starts_with("extern crate")
        {
            return "main.rs".to_string();
        }
        if trimmed.starts_with("import {")
            || trimmed.starts_with("import type {")
            || trimmed.starts_with("import { type")
        {
            return "index.ts".to_string();
        }
        if trimmed.starts_with("def ")
            || trimmed.starts_with("import ")
            || trimmed.starts_with("from ")
            || trimmed.starts_with("class ")
            || trimmed.starts_with("if __name__")
        {
            return "main.py".to_string();
        }
        if trimmed.starts_with("package ")
            || trimmed.starts_with("func ")
            || trimmed.starts_with("type ")
            || trimmed.starts_with("var (")
        {
            return "main.go".to_string();
        }
        if trimmed.starts_with("interface ")
            || trimmed.starts_with("export type")
            || trimmed.starts_with("export interface")
            || trimmed.starts_with("declare ")
            || trimmed.starts_with("namespace ")
            || trimmed.starts_with("import type")
        {
            return "index.ts".to_string();
        }
        if trimmed.starts_with("const ")
            || trimmed.starts_with("let ")
            || trimmed.starts_with("var ")
            || trimmed.starts_with("function ")
            || trimmed.starts_with("document.")
            || trimmed.starts_with("fetch(")
            || trimmed.starts_with("addEventListener")
        {
            return "script.js".to_string();
        }
        if trimmed.starts_with("body ")
            || trimmed.starts_with('.')
            || trimmed.starts_with('#')
            || trimmed.starts_with("@media")
            || trimmed.starts_with(":root")
            || (trimmed.contains("{")
                && trimmed.contains("}")
                && !trimmed.contains("function")
                && !trimmed.starts_with('{')
                && !trimmed.contains("=>"))
        {
            return "style.css".to_string();
        }
    }
    String::new()
}

/// Removes fenced code blocks from text, returning only the non-code parts.
pub fn strip_code_blocks(text: &str) -> String {
    let mut result = String::new();
    let mut in_fence = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if line.starts_with("### ") || line.starts_with("## ") {
            continue;
        }
        result.push_str(line);
        result.push('\n');
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_first_fenced_block() {
        let text = "a\n```js\nconst x = 1;\n```\nb";
        assert_eq!(extract_code_block(text), "const x = 1;");
    }

    #[test]
    fn guesses_filename_for_python() {
        let lines = vec!["def run():", "    pass"];
        assert_eq!(guess_filename(&lines), "main.py");
    }

    #[test]
    fn strips_code_fences() {
        let input = "hello\n```html\n<div>x</div>\n```\nworld";
        let out = strip_code_blocks(input);
        assert!(out.contains("hello"));
        assert!(out.contains("world"));
        assert!(!out.contains("<div>x</div>"));
    }

    #[test]
    fn extracts_named_file_header() {
        let text = "### index.html\n```html\n<h1>Hello</h1>\n```";
        let first = extract_code_block(text);
        assert_eq!(first, "<h1>Hello</h1>");
    }

    #[test]
    fn strips_code_blocks_from_chat_text() {
        let text = "Answer:\n```js\nconst x = 1;\n```\nDone.";
        let stripped = strip_code_blocks(text);
        assert!(stripped.contains("Answer:"));
        assert!(stripped.contains("Done."));
        assert!(!stripped.contains("const x = 1"));
    }
}
