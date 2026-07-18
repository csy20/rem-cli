use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time::timeout;

use crate::agentic::{run_lint, run_test};
use crate::blocklist::is_command_blocked;
use crate::find::{find_matches, FindOptions};
use crate::provider::tools::{builtin_tools, ToolCall, ToolResponse, ToolResult as ToolCallResult};
use crate::provider::Provider;
use crate::search::{perform_web_search, provider_from_config};
use crate::ui;

const MAX_TOOL_ROUNDS: usize = crate::constants::MAX_TOOL_ROUNDS;

/// Abstraction for interactive user interactions during tool execution.
/// Avoids capturing session state in multiple closures (which causes borrow conflicts).
pub(crate) trait UserInteraction: Send {
    fn approve_command(&mut self, cmd: &str) -> bool;
    fn ask_question(&mut self, question: &str) -> Option<String>;
}

/// Executes a single tool call and returns the result.
/// `user` handles shell command approval and user questions.
/// `tracked_writes` is populated with paths of written files (for undo/session tracking).
pub(crate) async fn execute_tool_call(
    tool_call: &ToolCall,
    project_dir: &std::path::Path,
    user: &mut dyn UserInteraction,
    tracked_writes: &Mutex<Vec<String>>,
) -> ToolCallResult {
    match tool_call.name.as_str() {
        "read_file" => execute_read_file(tool_call, project_dir).await,
        "write_file" => execute_write_file(tool_call, project_dir, tracked_writes).await,
        "search_files" => execute_search_files(tool_call, project_dir).await,
        "run_lint" => execute_tool_lint(tool_call, project_dir).await,
        "run_test" => execute_tool_test(tool_call, project_dir).await,
        "web_search" => execute_web_search(tool_call).await,
        "list_files" => execute_list_files(tool_call, project_dir).await,
        "run_command" => execute_run_command(tool_call, project_dir, user).await,
        "edit_file" => execute_edit_file(tool_call, project_dir, tracked_writes).await,
        "git_status" => execute_git_command("status", None, project_dir, tool_call.id.as_str()).await,
        "git_diff" => execute_git_diff(tool_call, project_dir).await,
        "git_log" => execute_git_log(tool_call, project_dir).await,
        "ask_user" => execute_ask_user(tool_call, user).await,

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

async fn execute_read_file(tool_call: &ToolCall, project_dir: &std::path::Path) -> ToolCallResult {
    let path_str = match extract_arg(tool_call, "path") {
        Some(p) => p,
        None => return err_result(tool_call, "missing 'path' argument"),
    };
    let path = match resolve_path(project_dir, &path_str) {
        Some(p) => p,
        None => return err_result(tool_call, &format!("path traversal blocked: {}", path_str)),
    };
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => ToolCallResult {
            call_id: tool_call.id.clone(),
            name: "read_file".into(),
            content: format!("File: {}\n```\n{}\n```", path.display(), content),
            is_error: false,
        },
        Err(e) => err_result(tool_call, &format!("cannot read file '{}': {}", path.display(), e)),
    }
}

async fn execute_write_file(
    tool_call: &ToolCall,
    project_dir: &std::path::Path,
    tracked_writes: &Mutex<Vec<String>>,
) -> ToolCallResult {
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
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    match tokio::fs::write(&path, content).await {
        Ok(()) => {
            if let Ok(mut writes) = tracked_writes.lock() {
                writes.push(path.to_string_lossy().to_string());
            }
            ToolCallResult {
                call_id: tool_call.id.clone(),
                name: "write_file".into(),
                content: format!("Successfully wrote {} bytes to {}", content.len(), path.display()),
                is_error: false,
            }
        }
        Err(e) => err_result(tool_call, &format!("cannot write '{}': {}", path.display(), e)),
    }
}

async fn execute_search_files(tool_call: &ToolCall, project_dir: &std::path::Path) -> ToolCallResult {
    let query = match extract_arg(tool_call, "query") {
        Some(q) => q,
        None => return err_result(tool_call, "missing 'query' argument"),
    };
    let pd = project_dir.to_path_buf();
    let report = tokio::task::spawn_blocking(move || find_matches(&pd, &query, &FindOptions::default()))
        .await
        .unwrap_or_else(|_| crate::find::FindReport {
            matches: Vec::new(),
            files_scanned: 0,
            files_skipped: 0,
            elapsed_ms: 0,
            truncated: false,
        });
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

async fn execute_tool_lint(tool_call: &ToolCall, project_dir: &std::path::Path) -> ToolCallResult {
    let path_str = match extract_arg(tool_call, "path") {
        Some(p) => p,
        None => return err_result(tool_call, "missing 'path' argument"),
    };
    let path = match resolve_path(project_dir, &path_str) {
        Some(p) => p,
        None => return err_result(tool_call, &format!("path traversal blocked: {}", path_str)),
    };
    let result = run_lint(&path.to_string_lossy()).await;
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

async fn execute_tool_test(tool_call: &ToolCall, project_dir: &std::path::Path) -> ToolCallResult {
    let path_str = match extract_arg(tool_call, "path") {
        Some(p) => p,
        None => return err_result(tool_call, "missing 'path' argument"),
    };
    let path = match resolve_path(project_dir, &path_str) {
        Some(p) => p,
        None => return err_result(tool_call, &format!("path traversal blocked: {}", path_str)),
    };
    let result = run_test(&path.to_string_lossy()).await;
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
    let search_provider = crate::config::get_cached_config().and_then(|cfg| {
        if cfg.search_provider == "ddg" {
            None
        } else {
            let api_key = cfg.search_api_key.clone().unwrap_or_default();
            let cse_id = cfg.search_cse_id.clone().unwrap_or_default();
            provider_from_config(&cfg.search_provider, &api_key, &cse_id)
        }
    });
    match perform_web_search(&client, &query, search_provider.as_ref()).await {
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

async fn execute_list_files(tool_call: &ToolCall, project_dir: &std::path::Path) -> ToolCallResult {
    let path_str = extract_arg(tool_call, "path").unwrap_or_else(|| ".".to_string());
    let path = match resolve_path(project_dir, &path_str) {
        Some(p) => p,
        None => return err_result(tool_call, &format!("path traversal blocked: {}", path_str)),
    };
    let mut content = String::new();
    if path.is_dir() {
        let mut entries = match tokio::fs::read_dir(&path).await {
            Ok(entries) => entries,
            Err(e) => return err_result(tool_call, &format!("cannot list '{}': {}", path.display(), e)),
        };
        let mut file_names = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            match entry.file_type().await {
                Ok(ft) if ft.is_dir() => file_names.push(format!("{}/\n", name)),
                _ => file_names.push(format!("{}\n", name)),
            }
        }
        for name in file_names {
            content.push_str(&name);
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

async fn execute_run_command(
    tool_call: &ToolCall,
    project_dir: &std::path::Path,
    user: &mut dyn UserInteraction,
) -> ToolCallResult {
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

    // Reconstruct full command for blocklist check (catches split-arg bypass)
    let full_cmd = if args.is_empty() {
        command.clone()
    } else {
        format!("{} {}", command, args.join(" "))
    };
    if is_command_blocked(&full_cmd) || is_command_blocked(&command) {
        return err_result(tool_call, "shell command blocked by security policy");
    }

    if !user.approve_command(&full_cmd) {
        return err_result(tool_call, "shell command execution denied by user");
    }

    let dir = project_dir.to_path_buf();
    let child = match tokio::process::Command::new(&command)
        .args(&args)
        .current_dir(&dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return err_result(tool_call, &format!("failed to spawn command: {}", e)),
    };
    let output_fut = child.wait_with_output();
    let result = timeout(
        Duration::from_secs(crate::constants::TOOL_COMMAND_TIMEOUT.as_secs()),
        output_fut,
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            let stdout = truncate_utf8_safe(&output.stdout, crate::constants::TOOL_COMMAND_STDOUT_MAX);
            let stderr = truncate_utf8_safe(&output.stderr, crate::constants::TOOL_COMMAND_STDERR_MAX);
            let mut content = String::new();
            if !stdout.is_empty() {
                content.push_str(&format!("stdout:\n{}\n", stdout));
            }
            if !stderr.is_empty() {
                content.push_str(&format!("stderr:\n{}\n", stderr));
            }
            ToolCallResult {
                call_id: tool_call.id.clone(),
                name: "run_command".into(),
                content,
                is_error: !output.status.success(),
            }
        }
        Ok(Err(e)) => err_result(tool_call, &format!("command wait failed: {}", e)),
        Err(_) => err_result(
            tool_call,
            &format!("command timed out after {:?}", crate::constants::TOOL_COMMAND_TIMEOUT),
        ),
    }
}

async fn execute_edit_file(
    tool_call: &ToolCall,
    project_dir: &std::path::Path,
    tracked_writes: &Mutex<Vec<String>>,
) -> ToolCallResult {
    let file_path = match extract_arg(tool_call, "file_path") {
        Some(p) => p,
        None => return err_result(tool_call, "missing 'file_path' argument"),
    };
    let old_string = match extract_arg(tool_call, "old_string") {
        Some(s) => s,
        None => return err_result(tool_call, "missing 'old_string' argument"),
    };
    let new_string = match extract_arg(tool_call, "new_string") {
        Some(s) => s,
        None => return err_result(tool_call, "missing 'new_string' argument"),
    };
    let path = match resolve_path(project_dir, &file_path) {
        Some(p) => p,
        None => return err_result(tool_call, &format!("path traversal blocked: {}", file_path)),
    };
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(e) => return err_result(tool_call, &format!("failed to read {}: {}", path.display(), e)),
    };
    let count = content.matches(&old_string).count();
    let pos = content.find(&old_string);
    let Some(pos) = pos else {
        return err_result(tool_call, &format!("old_string not found in {}", path.display()));
    };
    let note = if count > 1 {
        format!(" (replaced first of {} occurrences)", count)
    } else {
        String::new()
    };
    let new_content = format!(
        "{}{}{}",
        &content[..pos],
        new_string,
        &content[pos + old_string.len()..]
    );
    if let Err(e) = tokio::fs::write(&path, &new_content).await {
        return err_result(tool_call, &format!("failed to write {}: {}", path.display(), e));
    }
    if let Ok(mut writes) = tracked_writes.lock() {
        writes.push(file_path.clone());
    }
    ToolCallResult {
        call_id: tool_call.id.clone(),
        name: "edit_file".into(),
        content: format!(
            "Edited {}: replaced {} with {}{}",
            file_path, old_string, new_string, note
        ),
        is_error: false,
    }
}

async fn execute_git_command(
    name: &str,
    extra_args: Option<&[&str]>,
    project_dir: &std::path::Path,
    call_id: &str,
) -> ToolCallResult {
    let output = tokio::process::Command::new("git")
        .args(extra_args.unwrap_or(&[]))
        .current_dir(project_dir)
        .output()
        .await;
    match output {
        Ok(out) => {
            let mut content = String::new();
            if !out.stdout.is_empty() {
                content.push_str(&String::from_utf8_lossy(&out.stdout));
            }
            if !out.stderr.is_empty() {
                content.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            ToolCallResult {
                call_id: call_id.into(),
                name: name.into(),
                content,
                is_error: !out.status.success(),
            }
        }
        Err(e) => ToolCallResult {
            call_id: call_id.into(),
            name: name.into(),
            content: format!("git {} failed: {}", name, e),
            is_error: true,
        },
    }
}

async fn execute_git_diff(tool_call: &ToolCall, project_dir: &std::path::Path) -> ToolCallResult {
    let path = extract_arg(tool_call, "path");
    let mut args = vec!["diff"];
    if let Some(ref p) = path {
        args.push(p);
    }
    execute_git_command("diff", Some(&args), project_dir, tool_call.id.as_str()).await
}

async fn execute_git_log(tool_call: &ToolCall, project_dir: &std::path::Path) -> ToolCallResult {
    let max_count = tool_call
        .arguments
        .get("max_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(10);
    let count_arg = format!("-{}", max_count);
    execute_git_command(
        "log",
        Some(&["--oneline", &count_arg]),
        project_dir,
        tool_call.id.as_str(),
    )
    .await
}

async fn execute_ask_user(tool_call: &ToolCall, user: &mut dyn UserInteraction) -> ToolCallResult {
    let question = match extract_arg(tool_call, "question") {
        Some(q) => q,
        None => return err_result(tool_call, "missing 'question' argument"),
    };
    match user.ask_question(&question) {
        Some(answer) => {
            let trimmed = answer.trim().to_string();
            ToolCallResult {
                call_id: tool_call.id.clone(),
                name: "ask_user".into(),
                content: if trimmed.is_empty() {
                    "User provided no input".into()
                } else {
                    format!("User response: {}", trimmed)
                },
                is_error: false,
            }
        }
        None => err_result(tool_call, "user question not available in this context"),
    }
}

fn resolve_path(base: &std::path::Path, rel: &str) -> Option<PathBuf> {
    crate::types::resolve_safe_path(base, rel)
}

/// Safely truncates a byte slice to `max_bytes` without breaking UTF-8.
fn truncate_utf8_safe(data: &[u8], max_bytes: usize) -> String {
    let end = data.len().min(max_bytes);
    let valid_end = match std::str::from_utf8(&data[..end]) {
        Ok(s) => s.len(),
        Err(e) => e.valid_up_to(),
    };
    String::from_utf8_lossy(&data[..valid_end]).into_owned()
}

fn err_result(tool_call: &ToolCall, msg: &str) -> ToolCallResult {
    ToolCallResult {
        call_id: tool_call.id.clone(),
        name: tool_call.name.clone(),
        content: msg.to_string(),
        is_error: true,
    }
}

/// A no-op `UserInteraction` for parallel non-interactive tool execution.
struct NoopUserInteraction;
impl UserInteraction for NoopUserInteraction {
    fn approve_command(&mut self, _cmd: &str) -> bool {
        false
    }
    fn ask_question(&mut self, _question: &str) -> Option<String> {
        None
    }
}

/// Returns true if a tool call requires user interaction (cannot run in parallel).
fn requires_user_interaction(name: &str) -> bool {
    matches!(name, "run_command" | "ask_user")
}

/// Runs the tool loop: sends a prompt with tools, executes tool calls, and
/// continues until the LLM produces a text response.
/// Non-interactive tool calls (read, write, search, etc.) execute in parallel.
/// Returns (summary_text, written_file_paths).
pub(crate) async fn run_tool_loop(
    client: &Provider,
    prompt: &str,
    system_prompt: &str,
    history: &str,
    user: &mut dyn UserInteraction,
    project_dir: &std::path::Path,
) -> Result<(String, Vec<String>), String> {
    let tools = builtin_tools();
    let t = ui::theme::active();
    let tracked_writes = Arc::new(Mutex::new(Vec::new()));

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
                let writes = tracked_writes.lock().unwrap_or_else(|e| e.into_inner()).clone();
                return Ok((text, writes));
            }
            Ok(ToolResponse::ToolCalls(calls)) => {
                let mut results = Vec::with_capacity(calls.len());
                let mut interactive_futs = Vec::new();
                let mut parallel_handles = Vec::new();

                for call in &calls {
                    if requires_user_interaction(&call.name) {
                        // Flush any pending parallel tasks before interactive call
                        for handle in parallel_handles.drain(..) {
                            let tool_result: ToolCallResult = match handle.await {
                                Ok(r) => r,
                                Err(e) => err_result(call, &format!("parallel tool failed: {}", e)),
                            };
                            results.push(tool_result);
                        }
                        interactive_futs.push(call);
                    } else {
                        let tw = Arc::clone(&tracked_writes);
                        let pd = project_dir.to_path_buf();
                        let c = call.clone();
                        parallel_handles.push(tokio::spawn(async move {
                            execute_tool_call(&c, &pd, &mut crate::tool_executor::NoopUserInteraction, &tw).await
                        }));
                    }
                }

                // Collect remaining parallel results
                for handle in parallel_handles.drain(..) {
                    let tool_result: ToolCallResult = match handle.await {
                        Ok(r) => r,
                        Err(e) => {
                            let tc = ToolCall {
                                id: String::new(),
                                name: "parallel".into(),
                                arguments: serde_json::Value::Null,
                            };
                            err_result(&tc, &format!("parallel tool failed: {}", e))
                        }
                    };
                    results.push(tool_result);
                }

                // Run interactive tools sequentially
                for call in interactive_futs {
                    results.push(execute_tool_call(call, project_dir, user, &tracked_writes).await);
                }

                // Print results
                for result in &results {
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

    struct TestUser;
    impl UserInteraction for TestUser {
        fn approve_command(&mut self, _cmd: &str) -> bool {
            false
        }
        fn ask_question(&mut self, _question: &str) -> Option<String> {
            None
        }
    }

    #[tokio::test]
    async fn execute_unknown_tool_returns_error() {
        let tc = make_tool_call("nonexistent_tool", serde_json::json!({}));
        let mut user = TestUser;
        let tracked = Mutex::new(Vec::new());
        let result = execute_tool_call(&tc, &PathBuf::from("."), &mut user, &tracked).await;
        assert!(result.is_error);
        assert!(result.content.contains("Unknown tool"));
    }

    #[tokio::test]
    async fn execute_read_file_missing_path() {
        let tc = make_tool_call("read_file", serde_json::json!({}));
        let mut user = TestUser;
        let tracked = Mutex::new(Vec::new());
        let result = execute_tool_call(&tc, &PathBuf::from("."), &mut user, &tracked).await;
        assert!(result.is_error);
        assert!(result.content.contains("missing 'path'"));
    }
}
