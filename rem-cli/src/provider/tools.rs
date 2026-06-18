use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A tool/function specification sent to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// Result of executing a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    )
}
