use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A tool/function specification sent to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolSpec {
    pub fn to_openai_tool(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.parameters
            }
        })
    }

    pub fn to_anthropic_tool(&self) -> Value {
        serde_json::json!({
            "name": self.name,
            "description": self.description,
            "input_schema": self.parameters
        })
    }

    pub fn to_gemini_function_declaration(&self) -> Value {
        serde_json::json!({
            "name": self.name,
            "description": self.description,
            "parameters": self.parameters
        })
    }
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// Result of executing a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResult {
    pub call_id: String,
    pub name: String,
    pub content: String,
    pub is_error: bool,
}

/// Response from a tool-capable LLM call.
#[derive(Debug, Clone)]
pub enum ToolResponse {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

/// Returns the built-in tool definitions.
pub fn builtin_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "read_file".into(),
            description: "Read the contents of a file from the project workspace".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative or absolute path to the file"
                    }
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "write_file".into(),
            description: "Write content to a file. Creates parent directories if needed.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path where to write the file"
                    },
                    "content": {
                        "type": "string",
                        "description": "Full file content to write"
                    }
                },
                "required": ["path", "content"]
            }),
        },
        ToolSpec {
            name: "search_files".into(),
            description: "Search for text patterns inside project files".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Text or pattern to search for"
                    }
                },
                "required": ["query"]
            }),
        },
        ToolSpec {
            name: "run_lint".into(),
            description: "Run a linter on a file in the project".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to lint"
                    }
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "run_test".into(),
            description: "Run tests for a file or the whole project".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to test file or project directory"
                    }
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "web_search".into(),
            description: "Search the web for current information".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    }
                },
                "required": ["query"]
            }),
        },
        ToolSpec {
            name: "list_files".into(),
            description: "List files and directories inside a project path".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path to list"
                    }
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "run_command".into(),
            description: "Run a shell command and capture output".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to execute"
                    },
                    "args": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Command arguments"
                    }
                },
                "required": ["command"]
            }),
        },
        ToolSpec {
            name: "edit_file".into(),
            description: "Edit a file by replacing the first occurrence of old_string with new_string. Prefer this over write_file for targeted changes.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the file to edit"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "Text to search for (must be unique in the file)"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "Replacement text"
                    }
                },
                "required": ["file_path", "old_string", "new_string"]
            }),
        },
        ToolSpec {
            name: "git_status".into(),
            description: "Show the working tree status (git status)".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolSpec {
            name: "git_diff".into(),
            description: "Show changes in the working tree (git diff)".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Optional file path to restrict diff to"
                    }
                },
                "required": []
            }),
        },
        ToolSpec {
            name: "git_log".into(),
            description: "Show recent commit history (git log)".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "max_count": {
                        "type": "integer",
                        "description": "Number of recent commits to show (default 10)"
                    }
                },
                "required": []
            }),
        },
        ToolSpec {
            name: "ask_user".into(),
            description: "Ask the user a question and wait for their response. Use this to clarify intent or get approval.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "Question to ask the user"
                    }
                },
                "required": ["question"]
            }),
        },
    ]
}

/// Whether a provider kind supports native tool/function calling.
pub fn provider_supports_tools(kind: &super::ProviderKind) -> bool {
    matches!(
        kind,
        super::ProviderKind::OpenAI
            | super::ProviderKind::Anthropic
            | super::ProviderKind::Gemini
            | super::ProviderKind::Azure
            | super::ProviderKind::OpenRouter
            | super::ProviderKind::Ollama
            | super::ProviderKind::DeepSeek
            | super::ProviderKind::GitHub
            | super::ProviderKind::Groq
            | super::ProviderKind::XAI
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_tools_count() {
        let tools = builtin_tools();
        assert_eq!(tools.len(), 13);
    }

    #[test]
    fn builtin_tools_have_names() {
        let tools = builtin_tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"search_files"));
        assert!(names.contains(&"run_lint"));
        assert!(names.contains(&"run_test"));
        assert!(names.contains(&"web_search"));
        assert!(names.contains(&"list_files"));
        assert!(names.contains(&"run_command"));
        assert!(names.contains(&"edit_file"));
        assert!(names.contains(&"git_status"));
        assert!(names.contains(&"git_diff"));
        assert!(names.contains(&"git_log"));
        assert!(names.contains(&"ask_user"));
    }

    #[test]
    fn builtin_tools_have_non_empty_descriptions() {
        for tool in &builtin_tools() {
            assert!(!tool.description.is_empty(), "tool {} has empty description", tool.name);
        }
    }

    #[test]
    fn builtin_tools_have_required_params() {
        for tool in &builtin_tools() {
            let params = tool.parameters.as_object().expect("parameters should be an object");
            let required = params.get("required").and_then(|r| r.as_array());
            assert!(required.is_some(), "tool {} has no required field", tool.name);
            // Tools with no required params (git_status, git_diff, git_log) have empty required
            match tool.name.as_str() {
                "git_status" | "git_diff" | "git_log" => {
                    assert!(
                        required.unwrap().is_empty(),
                        "tool {} should have empty required",
                        tool.name
                    );
                }
                _ => {
                    assert!(!required.unwrap().is_empty(), "tool {} has empty required", tool.name);
                }
            }
        }
    }

    #[test]
    fn to_openai_tool_format() {
        let spec = ToolSpec {
            name: "test_tool".into(),
            description: "a test".into(),
            parameters: serde_json::json!({"type": "object", "properties": {"x": {"type": "string"}}}),
        };
        let v = spec.to_openai_tool();
        assert_eq!(v["type"], "function");
        assert_eq!(v["function"]["name"], "test_tool");
        assert_eq!(v["function"]["description"], "a test");
        assert!(v["function"]["parameters"].is_object());
    }

    #[test]
    fn to_anthropic_tool_format() {
        let spec = ToolSpec {
            name: "anthropic_tool".into(),
            description: "anthropic desc".into(),
            parameters: serde_json::json!({"type": "object"}),
        };
        let v = spec.to_anthropic_tool();
        assert_eq!(v["name"], "anthropic_tool");
        assert_eq!(v["description"], "anthropic desc");
        assert_eq!(v["input_schema"]["type"], "object");
    }

    #[test]
    fn to_gemini_function_declaration_format() {
        let spec = ToolSpec {
            name: "gemini_func".into(),
            description: "gemini desc".into(),
            parameters: serde_json::json!({"type": "object"}),
        };
        let v = spec.to_gemini_function_declaration();
        assert_eq!(v["name"], "gemini_func");
        assert_eq!(v["description"], "gemini desc");
        assert_eq!(v["parameters"]["type"], "object");
    }

    #[test]
    fn tool_result_construction() {
        let result = ToolResult {
            call_id: "call_1".into(),
            name: "read_file".into(),
            content: "file content".into(),
            is_error: false,
        };
        assert_eq!(result.call_id, "call_1");
        assert_eq!(result.name, "read_file");
        assert_eq!(result.content, "file content");
        assert!(!result.is_error);
    }

    #[test]
    fn tool_result_error_flag() {
        let result = ToolResult {
            call_id: "call_err".into(),
            name: "run_command".into(),
            content: "command failed".into(),
            is_error: true,
        };
        assert!(result.is_error);
    }

    #[test]
    fn tool_response_text_variant() {
        let response = ToolResponse::Text("hello".into());
        match response {
            ToolResponse::Text(t) => assert_eq!(t, "hello"),
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn tool_response_tool_calls_variant() {
        let calls = vec![ToolCall {
            id: "1".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "test.txt"}),
        }];
        let response = ToolResponse::ToolCalls(calls);
        match response {
            ToolResponse::ToolCalls(c) => {
                assert_eq!(c.len(), 1);
                assert_eq!(c[0].name, "read_file");
            }
            _ => panic!("expected ToolCalls variant"),
        }
    }

    #[test]
    fn tool_call_arguments() {
        let tc = ToolCall {
            id: "1".into(),
            name: "write_file".into(),
            arguments: serde_json::json!({"path": "f.txt", "content": "hi"}),
        };
        assert_eq!(tc.arguments["path"], "f.txt");
        assert_eq!(tc.arguments["content"], "hi");
    }

    #[test]
    fn provider_supports_tools_openai() {
        assert!(provider_supports_tools(&super::super::ProviderKind::OpenAI));
    }

    #[test]
    fn provider_supports_tools_anthropic() {
        assert!(provider_supports_tools(&super::super::ProviderKind::Anthropic));
    }

    #[test]
    fn provider_supports_tools_gemini() {
        assert!(provider_supports_tools(&super::super::ProviderKind::Gemini));
    }

    #[test]
    fn provider_supports_tools_azure() {
        assert!(provider_supports_tools(&super::super::ProviderKind::Azure));
    }

    #[test]
    fn provider_supports_tools_openrouter() {
        assert!(provider_supports_tools(&super::super::ProviderKind::OpenRouter));
    }

    #[test]
    fn provider_supports_tools_ollama() {
        assert!(provider_supports_tools(&super::super::ProviderKind::Ollama));
    }

    #[cfg(feature = "bedrock")]
    #[test]
    fn provider_supports_tools_bedrock() {
        assert!(!provider_supports_tools(&super::super::ProviderKind::Bedrock));
    }

    #[test]
    fn tool_spec_debug_and_clone() {
        let spec = ToolSpec {
            name: "test".into(),
            description: "desc".into(),
            parameters: serde_json::json!({}),
        };
        let cloned = spec.clone();
        assert_eq!(spec.name, cloned.name);
        let debug_str = format!("{:?}", spec);
        assert!(debug_str.contains("test"));
    }

    #[test]
    fn tool_call_default_arguments_null() {
        let tc = ToolCall {
            id: "2".into(),
            name: "search".into(),
            arguments: serde_json::Value::Null,
        };
        assert!(tc.arguments.is_null());
    }
}
