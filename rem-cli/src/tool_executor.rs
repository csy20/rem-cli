use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use tokio::io::AsyncBufReadExt;
use tokio::time::timeout;

use crate::agentic::{run_lint, run_test};
use crate::blocklist::is_command_blocked;
use crate::find::{find_matches, FindOptions};
use crate::provider::tools::{builtin_tools, ToolCall, ToolResponse, ToolResult as ToolCallResult};
use crate::provider::Provider;
use crate::search::perform_web_search;
use crate::ui;

const MAX_TOOL_ROUNDS: usize = crate::constants::MAX_TOOL_ROUNDS;

/// Executes a single tool call and returns the result.
pub(crate) async fn execute_tool_call(tool_call: &ToolCall, project_dir: &std::path::Path) -> ToolCallResult {
    match tool_call.name.as_str() {
        "read_file" => execute_read_file(tool_call, project_dir),
        "write_file" => execute_write_file(tool_call, project_dir),
        "search_files" => execute_search_files(tool_call, project_dir),
        "run_lint" => execute_tool_lint(tool_call, project_dir),
        "run_test" => execute_tool_test(tool_call, project_dir),
        "web_search" => execute_web_search(tool_call).await,
        "list_files" => execute_list_files(tool_call, project_dir),
        "run_command" => execute_run_command(tool_call, project_dir).await,
        name => ToolCallResult {
            call_id: tool_call.id.clone(),
            name: name.to_string(),
            content: format!("Unknown tool: {}", name),
            is_error: true,
        },
    }
}

fn extract_arg(tool_call: &ToolCall, key: &str) -> Option<String> {
    let args = &tool_call.arguments;
    args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn execute_read_file(tool_call: &ToolCall, project_dir: &std::path::Path) -> ToolCallResult {
    let path_str = match extract_arg(tool_call, "path") {
        Some(p) => p,
        None => return err_result(tool_call, "missing 'path' argument"),
    };
    let path = match resolve_path(project_dir, &path_str) {
        Some(p) => p,
        None => return err_result(tool_call, &format!("path traversal blocked: {}", path_str)),
    };
    match std::fs::read_to_string(&path) {
        Ok(content) => ToolCallResult {
            call_id: tool_call.id.clone(),
            name: "read_file".into(),
            content: format!("File: {}\n```\n{}\n```", path.display(), content),
            is_error: false,
        },
        Err(e) => err_result(tool_call, &format!("cannot read file '{}': {}", path.display(), e)),
    }
}

fn execute_write_file(tool_call: &ToolCall, project_dir: &std::path::Path) -> ToolCallResult {
    let path_str = match extract_arg(tool_call, "path") {
        Some(p) => p,
        None => return err_result(tool_call, "missing 'path' argument"),
    };
    let content = match tool_call.arguments.get("content").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => return err_result(tool_call, "missing 'content' argument"),
    };
    let path = match resolve_path(project_dir, &path_str) {
        Some(p) => p,
        None => return err_result(tool_call, &format!("path traversal blocked: {}", path_str)),
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, content) {
        Ok(()) => ToolCallResult {
            call_id: tool_call.id.clone(),
            name: "write_file".into(),
            content: format!("Successfully wrote {} bytes to {}", content.len(), path.display()),
            is_error: false,
        },
        Err(e) => err_result(tool_call, &format!("cannot write '{}': {}", path.display(), e)),
    }
}

fn execute_search_files(tool_call: &ToolCall, project_dir: &std::path::Path) -> ToolCallResult {
    let query = match extract_arg(tool_call, "query") {
        Some(q) => q,
        None => return err_result(tool_call, "missing 'query' argument"),
    };
    let report = find_matches(project_dir, &query, &FindOptions::default());
    let mut content = format!(
        "Found {} matches in {} files:\n",
        report.matches.len(),
        report.files_scanned
    );
    for m in report.matches.iter().take(30) {
        let rel = m.path.strip_prefix(project_dir).unwrap_or(&m.path);
        content.push_str(&format!("{}:{}: {}\n", rel.display(), m.line_no, m.line.trim()));
    }
    if report.matches.len() > 30 {
        content.push_str(&format!("... and {} more matches", report.matches.len() - 30));
    }
    ToolCallResult {
        call_id: tool_call.id.clone(),
        name: "search_files".into(),
        content,
        is_error: false,
    }
}

fn execute_tool_lint(tool_call: &ToolCall, project_dir: &std::path::Path) -> ToolCallResult {
    let path_str = match extract_arg(tool_call, "path") {
        Some(p) => p,
        None => return err_result(tool_call, "missing 'path' argument"),
    };
    let path = match resolve_path(project_dir, &path_str) {
        Some(p) => p,
        None => return err_result(tool_call, &format!("path traversal blocked: {}", path_str)),
    };
    let result = run_lint(&path.to_string_lossy());
    ToolCallResult {
        call_id: tool_call.id.clone(),
        name: "run_lint".into(),
        content: format!(
            "Lint result for {}:\n{}\n{}",
            path.display(),
            result.stdout,
            result.stderr
        ),
        is_error: !result.success,
    }
}

fn execute_tool_test(tool_call: &ToolCall, project_dir: &std::path::Path) -> ToolCallResult {
    let path_str = match extract_arg(tool_call, "path") {
        Some(p) => p,
        None => return err_result(tool_call, "missing 'path' argument"),
    };
    let path = match resolve_path(project_dir, &path_str) {
        Some(p) => p,
        None => return err_result(tool_call, &format!("path traversal blocked: {}", path_str)),
    };
    let result = run_test(&path.to_string_lossy());
    ToolCallResult {
        call_id: tool_call.id.clone(),
        name: "run_test".into(),
        content: format!(
            "Test result for {}:\n{}\n{}",
            path.display(),
            result.stdout,
            result.stderr
        ),
        is_error: !result.success,
    }
}

async fn execute_web_search(tool_call: &ToolCall) -> ToolCallResult {
    let query = match extract_arg(tool_call, "query") {
        Some(q) => q,
        None => return err_result(tool_call, "missing 'query' argument"),
    };
    let client = crate::provider::HTTP_CLIENT.clone();
    match perform_web_search(&client, &query, None).await {
        Ok(results) => {
            let mut content = String::new();
            for (i, r) in results.iter().enumerate().take(5) {
                content.push_str(&format!("{}. {}: {}\n", i + 1, r.title, r.snippet));
            }
            if results.is_empty() {
                content = "No web search results found.".to_string();
            }
            ToolCallResult {
                call_id: tool_call.id.clone(),
                name: "web_search".into(),
                content,
                is_error: false,
            }
        }
        Err(e) => err_result(tool_call, &format!("web search failed: {}", e)),
    }
}

fn execute_list_files(tool_call: &ToolCall, project_dir: &std::path::Path) -> ToolCallResult {
    let path_str = extract_arg(tool_call, "path").unwrap_or_else(|| ".".to_string());
    let path = match resolve_path(project_dir, &path_str) {
        Some(p) => p,
        None => return err_result(tool_call, &format!("path traversal blocked: {}", path_str)),
    };
    let mut content = String::new();
    if path.is_dir() {
        let entries = match std::fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(e) => return err_result(tool_call, &format!("cannot list '{}': {}", path.display(), e)),
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                content.push_str(&format!("{}/\n", name));
            } else {
                content.push_str(&format!("{}\n", name));
            }
        }
    } else {
        content = format!("Not a directory: {}", path.display());
    }
    ToolCallResult {
        call_id: tool_call.id.clone(),
        name: "list_files".into(),
        content,
        is_error: false,
    }
}

async fn execute_run_command(tool_call: &ToolCall, project_dir: &std::path::Path) -> ToolCallResult {
    let command = match extract_arg(tool_call, "command") {
        Some(c) => c,
        None => return err_result(tool_call, "missing 'command' argument"),
    };
    let args: Vec<String> = tool_call
        .arguments
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    // Blocklist check against FULL reconstructed command (prevents bypass via split args)
    let full_cmd_str = if args.is_empty() {
        command.clone()
    } else {
        format!("{} {}", command, args.join(" "))
    };
    if is_command_blocked(&full_cmd_str) || is_command_blocked(&command) || args.iter().any(|a| is_command_blocked(a)) {
        return err_result(tool_call, "shell command blocked by security policy");
    }

    // Interactive approval prompt for shell commands
    if std::io::stdin().is_terminal() {
        let full_cmd = if args.is_empty() {
            command.clone()
        } else {
            format!("{} {}", command, args.join(" "))
        };
        eprint!("  ! Allow shell command? [y/N] {} ", full_cmd);
        let _ = io::stderr().flush();
        let mut input = String::new();
        let mut reader = tokio::io::BufReader::new(tokio::io::stdin());
        match reader.read_line(&mut input).await {
            Ok(_) => {
                let trimmed = input.trim().to_lowercase();
                if trimmed != "y" && trimmed != "yes" {
                    return err_result(tool_call, "shell command execution denied by user");
                }
            }
            Err(_) => {
                return err_result(tool_call, "failed to read user input for approval");
            }
        }
    }

    let cmd = command.clone();
    let args_clone = args.clone();
    let dir = project_dir.to_path_buf();
    let result = timeout(
        Duration::from_secs(crate::constants::TOOL_COMMAND_TIMEOUT.as_secs()),
        tokio::task::spawn_blocking(move || Command::new(&cmd).args(&args_clone).current_dir(&dir).output()),
    )
    .await;

    match result {
        Ok(Ok(Ok(output))) => {
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let mut content = String::new();
            if !stdout.is_empty() {
                content.push_str(&format!(
                    "stdout:\n{}\n",
                    &stdout
                        .chars()
                        .take(crate::constants::TOOL_COMMAND_STDOUT_MAX)
                        .collect::<String>()
                ));
            }
            if !stderr.is_empty() {
                content.push_str(&format!(
                    "stderr:\n{}\n",
                    &stderr
                        .chars()
                        .take(crate::constants::TOOL_COMMAND_STDERR_MAX)
                        .collect::<String>()
                ));
            }
            ToolCallResult {
                call_id: tool_call.id.clone(),
                name: "run_command".into(),
                content,
                is_error: !output.status.success(),
            }
        }
        Ok(Ok(Err(e))) => err_result(tool_call, &format!("command failed: {}", e)),
        Ok(Err(_)) => err_result(tool_call, "command thread panicked"),
        Err(_) => err_result(
            tool_call,
            &format!("command timed out after {:?}", crate::constants::TOOL_COMMAND_TIMEOUT),
        ),
    }
}

fn resolve_path(base: &std::path::Path, rel: &str) -> Option<PathBuf> {
    crate::types::resolve_safe_path(base, rel)
}

fn err_result(tool_call: &ToolCall, msg: &str) -> ToolCallResult {
    ToolCallResult {
        call_id: tool_call.id.clone(),
        name: tool_call.name.clone(),
        content: msg.to_string(),
        is_error: true,
    }
}

/// Runs the tool loop: sends a prompt with tools, executes tool calls, and
/// continues until the LLM produces a text response.
pub(crate) async fn run_tool_loop(
    client: &Provider,
    prompt: &str,
    system_prompt: &str,
    history: &str,
) -> Result<String, String> {
    let tools = builtin_tools();
    let project_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let t = ui::theme::active();

    let mut current_prompt = prompt.to_string();
    let current_system = system_prompt.to_string();
    let mut current_history = history.to_string();
    let mut round = 0usize;

    loop {
        if round >= MAX_TOOL_ROUNDS {
            return Err("Max tool rounds reached".to_string());
        }
        round += 1;

        match client
            .complete_chat_stream_with_tools(&current_prompt, &current_system, &current_history, &tools)
            .await
        {
            Ok(ToolResponse::Text(text)) => {
                return Ok(text);
            }
            Ok(ToolResponse::ToolCalls(calls)) => {
                let mut results = Vec::new();
                for call in &calls {
                    let result = execute_tool_call(call, &project_dir).await;
                    results.push(result.clone());
                    if result.is_error {
                        println!(
                            "  {} {} tool '{}' failed: {}",
                            ui::theme::paint_warning(&t, "!"),
                            ui::theme::paint_dim(&t, "tool"),
                            result.name,
                            result.content.chars().take(200).collect::<String>()
                        );
                    } else {
                        println!(
                            "  {} {} {} — {} bytes output",
                            ui::theme::paint_success_label(&t, "✓"),
                            ui::theme::paint_dim(&t, "tool"),
                            result.name,
                            result.content.len()
                        );
                    }
                }

                // Build follow-up message with tool results
                let mut follow_up = String::from("Tool execution results:\n\n");
                for r in &results {
                    follow_up.push_str(&format!(
                        "[Tool: {}]\n{}\n---\n",
                        r.name,
                        r.content
                            .chars()
                            .take(crate::constants::TOOL_RESULT_MAX_CHARS)
                            .collect::<String>()
                    ));
                }
                follow_up.push_str("\nContinue with the task based on these results.");

                // For the next round, the tool results become the user prompt
                // and system/history are passed through
                current_prompt = follow_up;
                current_history.clear();
            }
            Err(e) => {
                return Err(format!("LLM call failed: {}", e));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_tool_call(name: &str, args: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "test-1".into(),
            name: name.into(),
            arguments: args,
        }
    }

    #[test]
    fn extract_arg_returns_value() {
        let tc = make_tool_call("read_file", serde_json::json!({"path": "src/main.rs"}));
        assert_eq!(extract_arg(&tc, "path"), Some("src/main.rs".into()));
    }

    #[test]
    fn extract_arg_returns_none_for_missing() {
        let tc = make_tool_call("read_file", serde_json::json!({}));
        assert_eq!(extract_arg(&tc, "path"), None);
    }

    #[test]
    fn extract_arg_returns_none_for_wrong_type() {
        let tc = make_tool_call("read_file", serde_json::json!({"path": 42}));
        assert_eq!(extract_arg(&tc, "path"), None);
    }

    #[test]
    fn err_result_sets_error_flag() {
        let tc = make_tool_call("test_tool", serde_json::json!({}));
        let err = err_result(&tc, "something went wrong");
        assert!(err.is_error);
        assert_eq!(err.name, "test_tool");
        assert!(err.content.contains("something went wrong"));
    }

    #[tokio::test]
    async fn execute_unknown_tool_returns_error() {
        let tc = make_tool_call("nonexistent_tool", serde_json::json!({}));
        let result = execute_tool_call(&tc, &PathBuf::from(".")).await;
        assert!(result.is_error);
        assert!(result.content.contains("Unknown tool"));
    }

    #[tokio::test]
    async fn execute_read_file_missing_path() {
        let tc = make_tool_call("read_file", serde_json::json!({}));
        let result = execute_tool_call(&tc, &PathBuf::from(".")).await;
        assert!(result.is_error);
        assert!(result.content.contains("missing 'path'"));
    }
}
