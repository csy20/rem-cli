//! Autonomous agent loop utilities.
//! Provides lint/test tool execution, result formatting, agentic prompt
//! construction for iterative code generation, and goal signal extraction.

use std::process::Command;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::ui;

/// Result of running an external tool (linter, test runner, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_name: String,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub action: String,
}

/// Programming language target for linting/testing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LintTarget {
    Rust,
    Python,
    Go,
    JavaScript,
    TypeScript,
    Css,
    Html,
    Unknown,
}

impl LintTarget {
    /// Detects the language target from a file path extension.
    pub fn detect(path: &str) -> Self {
        if path.ends_with(".rs") {
            LintTarget::Rust
        } else if path.ends_with(".py") {
            LintTarget::Python
        } else if path.ends_with(".go") {
            LintTarget::Go
        } else if path.ends_with(".js") {
            LintTarget::JavaScript
        } else if path.ends_with(".ts") || path.ends_with(".tsx") {
            LintTarget::TypeScript
        } else if path.ends_with(".css") {
            LintTarget::Css
        } else if path.ends_with(".html") || path.ends_with(".htm") {
            LintTarget::Html
        } else {
            LintTarget::Unknown
        }
    }
}

/// Runs the appropriate linter for a file path.
pub fn run_lint(path: &str) -> ToolResult {
    let target = LintTarget::detect(path);
    let start = Instant::now();

    let (name, cmd, args): (&str, &str, Vec<&str>) = match target {
        LintTarget::Rust => ("rustfmt", "rustfmt", vec!["--check", path]),
        LintTarget::Python => ("ruff", "ruff", vec!["check", path]),
        LintTarget::Go => ("gofmt", "gofmt", vec!["-d", path]),
        LintTarget::JavaScript | LintTarget::TypeScript => {
            ("eslint", "npx", vec!["eslint", path, "--format", "compact"])
        }
        LintTarget::Css => ("stylelint", "npx", vec!["stylelint", path]),
        LintTarget::Html => ("htmlhint", "npx", vec!["--no-install", "htmlhint", path]),
        LintTarget::Unknown => {
            return ToolResult {
                tool_name: "unknown".into(),
                success: false,
                stdout: String::new(),
                stderr: "No linter configured for this file type".into(),
                duration_ms: start.elapsed().as_millis() as u64,
                action: "lint".into(),
            };
        }
    };

    match Command::new(cmd).args(&args).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            ToolResult {
                tool_name: name.into(),
                success: output.status.success(),
                stdout,
                stderr,
                duration_ms: start.elapsed().as_millis() as u64,
                action: "lint".into(),
            }
        }
        Err(e) => ToolResult {
            tool_name: name.into(),
            success: false,
            stdout: String::new(),
            stderr: format!("Failed to run {}: {}", name, e),
            duration_ms: start.elapsed().as_millis() as u64,
            action: "lint".into(),
        },
    }
}

/// Runs the appropriate test runner for a file path (cargo test, pytest, etc.).
pub fn run_test(path: &str) -> ToolResult {
    let target = LintTarget::detect(path);
    let start = Instant::now();

    let result = match target {
        LintTarget::Rust => Command::new("cargo").args(["test", "--quiet"]).output(),
        LintTarget::Python => Command::new("python3")
            .args(["-m", "pytest", path, "-q"])
            .output(),
        LintTarget::Go => Command::new("go").args(["test", "./..."]).output(),
        LintTarget::JavaScript | LintTarget::TypeScript => Command::new("npx")
            .args(["jest", path, "--no-coverage"])
            .output(),
        LintTarget::Css | LintTarget::Html | LintTarget::Unknown => {
            return ToolResult {
                tool_name: "test".into(),
                success: false,
                stdout: String::new(),
                stderr: "No test runner configured for this file type".into(),
                duration_ms: start.elapsed().as_millis() as u64,
                action: "test".into(),
            };
        }
    };

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let combined = if stdout.len() > 2000 {
                format!("{}...\n[truncated to 2000 chars]", &stdout[..2000])
            } else {
                stdout.clone()
            };
            ToolResult {
                tool_name: "test".into(),
                success: output.status.success(),
                stdout: combined,
                stderr,
                duration_ms: start.elapsed().as_millis() as u64,
                action: "test".into(),
            }
        }
        Err(e) => ToolResult {
            tool_name: "test".into(),
            success: false,
            stdout: String::new(),
            stderr: format!("Failed to run tests: {}", e),
            duration_ms: start.elapsed().as_millis() as u64,
            action: "test".into(),
        },
    }
}

/// Formats tool execution output with styled status and truncated stdout/stderr.
pub fn format_tool_output(result: &ToolResult) -> String {
    let t = ui::theme::active();
    let status = if result.success {
        ui::theme::paint_success_label(&t, "PASS")
    } else {
        ui::theme::paint_error_label(&t, "FAIL")
    };

    let mut output = format!(
        "\n{} {} {} ({:.1}s)\n",
        ui::theme::paint_dim(&t, "\u{2502}"),
        status,
        result.tool_name,
        result.duration_ms as f64 / 1000.0
    );

    if !result.stdout.trim().is_empty() {
        output.push_str(&format!(
            "{} stdout:\n{}\n",
            ui::theme::paint_dim(&t, "\u{2502}"),
            result.stdout.trim()
        ));
    }

    if !result.stderr.trim().is_empty() {
        output.push_str(&format!(
            "{} {} stderr:\n{}\n",
            ui::theme::paint_dim(&t, "\u{2502}"),
            ui::theme::paint_warning(&t, "\u{26a0}"),
            result.stderr.trim()
        ));
    }

    output
}

/// Builds a combined tool output context string from optional lint/test/build results.
pub fn build_tool_context(
    lint_result: Option<&ToolResult>,
    test_result: Option<&ToolResult>,
    build_result: Option<&ToolResult>,
) -> String {
    let mut ctx = String::new();

    if let Some(r) = lint_result {
        ctx.push_str("[Tool: Lint]\n");
        ctx.push_str(&format_tool_output(r));
        ctx.push('\n');
    }

    if let Some(r) = test_result {
        ctx.push_str("[Tool: Test]\n");
        ctx.push_str(&format_tool_output(r));
        ctx.push('\n');
    }

    if let Some(r) = build_result {
        ctx.push_str("[Tool: Build]\n");
        ctx.push_str(&format_tool_output(r));
        ctx.push('\n');
    }

    if ctx.is_empty() {
        ctx.push_str("[No tool results available]\n");
    }

    ctx
}

/// Builds the agentic loop prompt with iteration tracking and tool output.
pub fn build_agentic_prompt(
    task: &str,
    tool_output: &str,
    iteration: usize,
    max_iterations: usize,
) -> String {
    format!(
        r##"You are REM in autonomous agent mode (iteration {}/{}).

Task: {}

{}

Instructions:
1. Analyze any lint/test/build errors above
2. Generate fixed code using ### path/file headings
3. If an iteration fails, try a different approach
4. Signal completion: GOAL_ACHIEVED: <summary>
5. Signal failure: GOAL_FAILED: <reason>
6. Be concise — only generate what's needed

Generate corrected code now:"##,
        iteration, max_iterations, task, tool_output
    )
}

/// Extracts `GOAL_ACHIEVED`/`GOAL_FAILED` signal from an LLM response.
pub fn extract_goal_signal(response: &str) -> Option<(bool, String)> {
    for line in response.lines() {
        if let Some(summary) = line.strip_prefix("GOAL_ACHIEVED:") {
            return Some((true, summary.trim().to_string()));
        }
        if let Some(reason) = line.strip_prefix("GOAL_FAILED:") {
            return Some((false, reason.trim().to_string()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_rust_from_rs_extension() {
        assert_eq!(LintTarget::detect("src/main.rs"), LintTarget::Rust);
    }

    #[test]
    fn detect_python_from_py_extension() {
        assert_eq!(LintTarget::detect("script.py"), LintTarget::Python);
    }

    #[test]
    fn detect_go_from_go_extension() {
        assert_eq!(LintTarget::detect("main.go"), LintTarget::Go);
    }

    #[test]
    fn detect_javascript_from_js_extension() {
        assert_eq!(LintTarget::detect("app.js"), LintTarget::JavaScript);
    }

    #[test]
    fn detect_typescript_from_ts_extension() {
        assert_eq!(LintTarget::detect("app.ts"), LintTarget::TypeScript);
        assert_eq!(LintTarget::detect("app.tsx"), LintTarget::TypeScript);
    }

    #[test]
    fn detect_css_from_css_extension() {
        assert_eq!(LintTarget::detect("style.css"), LintTarget::Css);
    }

    #[test]
    fn detect_html_from_html_extension() {
        assert_eq!(LintTarget::detect("index.html"), LintTarget::Html);
        assert_eq!(LintTarget::detect("page.htm"), LintTarget::Html);
    }

    #[test]
    fn detect_unknown_for_unrecognized_extension() {
        assert_eq!(LintTarget::detect("Makefile"), LintTarget::Unknown);
        assert_eq!(LintTarget::detect("data.txt"), LintTarget::Unknown);
    }

    #[test]
    fn extract_goal_achieved_signal() {
        let resp = "Some work\nGOAL_ACHIEVED: All tests pass\nDone";
        let result = extract_goal_signal(resp);
        assert_eq!(result, Some((true, "All tests pass".to_string())));
    }

    #[test]
    fn extract_goal_failed_signal() {
        let resp = "Tried approach A\nGOAL_FAILED: Compilation error persists";
        let result = extract_goal_signal(resp);
        assert_eq!(
            result,
            Some((false, "Compilation error persists".to_string()))
        );
    }

    #[test]
    fn extract_no_signal_when_absent() {
        let resp = "Just a regular response without signals";
        assert_eq!(extract_goal_signal(resp), None);
    }

    #[test]
    fn format_tool_output_includes_status() {
        let result = ToolResult {
            tool_name: "rustfmt".into(),
            success: true,
            stdout: "formatted OK".into(),
            stderr: String::new(),
            duration_ms: 150,
            action: "lint".into(),
        };
        let output = format_tool_output(&result);
        assert!(output.contains("PASS"));
        assert!(output.contains("rustfmt"));
    }

    #[test]
    fn format_tool_output_shows_stderr() {
        let result = ToolResult {
            tool_name: "ruff".into(),
            success: false,
            stdout: String::new(),
            stderr: "syntax error".into(),
            duration_ms: 200,
            action: "lint".into(),
        };
        let output = format_tool_output(&result);
        assert!(output.contains("FAIL"));
        assert!(output.contains("syntax error"));
    }

    #[test]
    fn build_tool_context_returns_empty_when_all_none() {
        let ctx = build_tool_context(None, None, None);
        assert!(ctx.contains("No tool results available"));
    }
}
