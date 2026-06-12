use std::process::Command;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::ui;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_name: String,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub action: String,
}

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
