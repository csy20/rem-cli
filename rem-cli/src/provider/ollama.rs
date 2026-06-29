use std::sync::LazyLock;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use super::tools::{ToolCall, ToolResponse, ToolSpec};
use super::{ProviderBackend, ProviderContext};

static NUM_THREADS: LazyLock<usize> = LazyLock::new(|| {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(crate::constants::OLLAMA_NUM_THREADS_FALLBACK)
});

#[derive(Debug, Deserialize)]
pub struct OllamaTagsResponse {
    pub models: Vec<OllamaTagModel>,
}

#[derive(Debug, Deserialize)]
pub struct OllamaTagModel {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct OllamaJsonResponse {
    pub response: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OllamaStreamLine {
    pub response: Option<String>,
    pub done: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct OllamaChatStreamLine {
    pub message: Option<OllamaChatMessage>,
    pub done: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct OllamaChatMessage {
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Debug, Deserialize)]
pub struct OllamaToolCall {
    pub function: Option<OllamaToolCallFunction>,
}

#[derive(Debug, Deserialize)]
pub struct OllamaToolCallFunction {
    pub name: Option<String>,
    pub arguments: Option<serde_json::Value>,
}

pub(super) struct OllamaBackend;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_tags_response_deserialize() {
        let json = r#"{"models":[{"name":"rem-coder:latest"},{"name":"llama3:8b"}]}"#;
        let resp: OllamaTagsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.models.len(), 2);
        assert_eq!(resp.models[0].name, "rem-coder:latest");
    }

    #[test]
    fn ollama_json_response_deserialize() {
        let json = r#"{"response":"{\"explanation\":\"test\"}"}"#;
        let resp: OllamaJsonResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.response, "{\"explanation\":\"test\"}");
    }

    #[test]
    fn ollama_stream_line_deserialize_with_response() {
        let json = r#"{"response":"hello","done":false}"#;
        let line: OllamaStreamLine = serde_json::from_str(json).unwrap();
        assert_eq!(line.response.as_deref(), Some("hello"));
        assert_eq!(line.done, Some(false));
    }

    #[test]
    fn ollama_stream_line_done() {
        let json = r#"{"response":"","done":true}"#;
        let line: OllamaStreamLine = serde_json::from_str(json).unwrap();
        assert_eq!(line.done, Some(true));
    }

    #[test]
    fn ollama_stream_line_partial() {
        let json = r#"{"response":"world"}"#;
        let line: OllamaStreamLine = serde_json::from_str(json).unwrap();
        assert_eq!(line.response.as_deref(), Some("world"));
        assert!(line.done.is_none());
    }

    #[test]
    fn ollama_chat_stream_line_deserialize() {
        let json = r#"{"message":{"content":"hi"},"done":false}"#;
        let line: OllamaChatStreamLine = serde_json::from_str(json).unwrap();
        assert_eq!(line.message.as_ref().and_then(|m| m.content.as_deref()), Some("hi"));
        assert_eq!(line.done, Some(false));
    }

    #[test]
    fn ollama_chat_stream_line_with_tool_calls() {
        let json = r#"{"message":{"content":"","tool_calls":[{"function":{"name":"read_file","arguments":{"path":"x"}}}]},"done":false}"#;
        let line: OllamaChatStreamLine = serde_json::from_str(json).unwrap();
        let calls = line.message.as_ref().and_then(|m| m.tool_calls.as_ref());
        assert!(calls.is_some());
        let calls = calls.unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].function.as_ref().and_then(|f| f.name.as_deref()),
            Some("read_file")
        );
    }

    #[test]
    fn ollama_chat_stream_line_done() {
        let json = r#"{"message":{"content":"bye"},"done":true}"#;
        let line: OllamaChatStreamLine = serde_json::from_str(json).unwrap();
        assert_eq!(line.done, Some(true));
    }

    #[test]
    fn ollama_tool_call_function_arguments() {
        let json = r#"{"function":{"name":"search_files","arguments":{"query":"TODO"}}}"#;
        let tc: OllamaToolCall = serde_json::from_str(json).unwrap();
        assert_eq!(
            tc.function.as_ref().and_then(|f| f.name.as_deref()),
            Some("search_files")
        );
        let args = tc.function.as_ref().and_then(|f| f.arguments.as_ref());
        assert!(args.is_some());
        assert_eq!(args.unwrap().get("query").and_then(|v| v.as_str()), Some("TODO"));
    }
}

#[async_trait]
impl ProviderBackend for OllamaBackend {
    async fn list_models(&self, ctx: &ProviderContext) -> Result<Vec<String>> {
        let url = super::api_url(&ctx.base_url, "tags");
        let resp = ctx.client.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("Ollama unreachable at {}", ctx.base_url));
        }
        let parsed: OllamaTagsResponse = resp.json().await.context("invalid tags response")?;
        Ok(parsed.models.into_iter().map(|m| m.name).collect())
    }

    async fn complete_json(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<crate::ModelReply> {
        let url = super::api_url(&ctx.base_url, "generate");
        let final_prompt = format!("{system_prompt}\n\nUser request:\n{user_prompt}\n\nReturn JSON only.");
        let payload = json!({
            "model": ctx.model,
            "prompt": final_prompt,
            "stream": false,
            "options": { "num_predict": crate::constants::DEFAULT_MAX_TOKENS, "num_ctx": ctx.model_ctx, "num_thread": *NUM_THREADS },
            "format": {
                "type": "object",
                "properties": {
                    "explanation": {"type": "string"},
                    "code": {"type": "string"},
                    "files": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {"path": {"type": "string"}, "content": {"type": "string"}},
                            "required": ["path", "content"]
                        }
                    },
                    "commands": {"type": "array", "items": {"type": "string"}},
                    "checks": {"type": "array", "items": {"type": "string"}},
                    "caution": {"type": "string"}
                },
                "required": ["explanation", "code", "commands", "checks", "caution"]
            }
        });

        let resp = ctx
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("failed to call Ollama")?;
        if !resp.status().is_success() {
            let err = super::handle_ollama_error(resp, &url, &ctx.model).await;
            return Err(err.unwrap_err());
        }

        let raw: OllamaJsonResponse = resp.json().await.context("invalid Ollama response")?;
        super::parse_json_fallback(&raw.response)
    }

    async fn complete_chat_stream(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let url = super::api_url(&ctx.base_url, "chat");
        let mut messages: Vec<serde_json::Value> = vec![json!({"role": "system", "content": system_prompt})];
        if !history.is_empty() {
            messages.push(json!({"role": "user", "content": history}));
        }
        messages.push(json!({"role": "user", "content": user_prompt}));
        let payload = json!({
            "model": ctx.model,
            "messages": messages,
            "stream": true,
            "options": { "num_predict": crate::constants::DEFAULT_MAX_TOKENS, "num_ctx": ctx.model_ctx, "num_thread": *NUM_THREADS }
        });
        let resp = ctx
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("failed to call Ollama")?;
        if !resp.status().is_success() {
            return super::handle_ollama_error(resp, &url, &ctx.model).await;
        }

        let mut full_text = String::with_capacity(crate::constants::INITIAL_BUF_CAPACITY);
        super::stream_buf(resp, |trimmed| {
            if let Ok(obj) = serde_json::from_str::<OllamaChatStreamLine>(trimmed) {
                if let Some(ref msg) = obj.message {
                    if let Some(ref content) = msg.content {
                        full_text.push_str(content);
                        super::emit_token(content);
                    }
                }
                if obj.done == Some(true) {
                    return Ok(false);
                }
            }
            if full_text.len() > super::MAX_RESPONSE_BYTES {
                return Err(anyhow!("response too large ({} bytes)", super::MAX_RESPONSE_BYTES));
            }
            Ok(true)
        })
        .await?;
        Ok(full_text)
    }

    async fn complete_chat_stream_with_vision(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
        mime_type: &str,
        base64_data: &str,
    ) -> Result<String> {
        let url = super::api_url(&ctx.base_url, "chat");
        let mut messages: Vec<serde_json::Value> = vec![];
        if !history.is_empty() {
            messages.push(json!({"role": "user", "content": history}));
        }
        messages.push(json!({
            "role": "user",
            "content": [
                json!({"type": "text", "text": user_prompt}),
                json!({"type": "image_url", "image_url": {"url": format!("data:{};base64,{}", mime_type, base64_data)}})
            ]
        }));
        let payload = json!({
            "model": ctx.model,
            "system": system_prompt,
            "messages": messages,
            "stream": true,
            "options": { "num_predict": crate::constants::DEFAULT_MAX_TOKENS, "num_ctx": ctx.model_ctx, "num_thread": *NUM_THREADS }
        });
        let resp = ctx
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("failed to call Ollama vision API")?;
        if !resp.status().is_success() {
            return super::handle_ollama_error(resp, &url, &ctx.model).await;
        }

        let mut full_text = String::with_capacity(crate::constants::INITIAL_BUF_CAPACITY);
        super::stream_buf(resp, |trimmed| {
            if let Ok(obj) = serde_json::from_str::<OllamaChatStreamLine>(trimmed) {
                if let Some(ref msg) = obj.message {
                    if let Some(ref content) = msg.content {
                        full_text.push_str(content);
                        super::emit_token(content);
                    }
                }
                if obj.done == Some(true) {
                    return Ok(false);
                }
            }
            if full_text.len() > super::MAX_RESPONSE_BYTES {
                return Err(anyhow!("response too large ({} bytes)", super::MAX_RESPONSE_BYTES));
            }
            Ok(true)
        })
        .await?;
        Ok(full_text)
    }

    async fn complete_chat_stream_with_tools(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
        tool_specs: &[ToolSpec],
    ) -> Result<ToolResponse> {
        let url = super::api_url(&ctx.base_url, "chat");

        let mut messages: Vec<serde_json::Value> = vec![];
        messages.push(json!({"role": "system", "content": system_prompt}));
        if !history.is_empty() {
            messages.push(json!({"role": "user", "content": history}));
        }
        messages.push(json!({"role": "user", "content": user_prompt}));

        let tools_json: Vec<serde_json::Value> = tool_specs.iter().map(|t| t.to_openai_tool()).collect();

        let mut payload = json!({
            "model": ctx.model,
            "messages": messages,
            "stream": true,
            "options": { "num_predict": crate::constants::DEFAULT_MAX_TOKENS, "num_ctx": ctx.model_ctx, "num_thread": *NUM_THREADS }
        });
        if !tools_json.is_empty() {
            payload["tools"] = json!(tools_json);
        }

        let resp = ctx
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("failed to call Ollama chat API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            // Only fall back to non-tool chat on 404 (tool calling not supported by model)
            if status == reqwest::StatusCode::NOT_FOUND {
                let text = self
                    .complete_chat_stream(ctx, system_prompt, user_prompt, history)
                    .await?;
                return Ok(ToolResponse::Text(text));
            }
            // Propagate all other errors (auth, server errors, etc.)
            let err = super::handle_ollama_error(resp, &url, &ctx.model).await;
            return Err(err.unwrap_err());
        }

        let mut full_text = String::with_capacity(crate::constants::INITIAL_BUF_CAPACITY);
        let mut tool_calls: Vec<(i64, String, String, String)> = Vec::new();

        super::stream_buf(resp, |trimmed| {
            if let Ok(obj) = serde_json::from_str::<OllamaChatStreamLine>(trimmed) {
                if let Some(ref msg) = obj.message {
                    if let Some(ref content) = msg.content {
                        full_text.push_str(content);
                    }
                    if let Some(ref calls) = msg.tool_calls {
                        for tc in calls {
                            let idx = tool_calls.len() as i64;
                            let name = tc.function.as_ref().and_then(|f| f.name.clone()).unwrap_or_default();
                            let args = tc
                                .function
                                .as_ref()
                                .and_then(|f| f.arguments.as_ref().map(|a| a.to_string()))
                                .unwrap_or_default();
                            tool_calls.push((idx, String::new(), name, args));
                        }
                    }
                }
                if obj.done == Some(true) {
                    return Ok(false);
                }
            }
            if full_text.len() > super::MAX_RESPONSE_BYTES {
                return Err(anyhow!("response too large ({} bytes)", super::MAX_RESPONSE_BYTES));
            }
            Ok(true)
        })
        .await?;

        if !tool_calls.is_empty() {
            let calls: Vec<ToolCall> = tool_calls
                .into_iter()
                .map(|(_, id, name, args_str)| ToolCall {
                    id,
                    name,
                    arguments: serde_json::from_str(&args_str).unwrap_or(serde_json::Value::Null),
                })
                .collect();
            Ok(ToolResponse::ToolCalls(calls))
        } else {
            Ok(ToolResponse::Text(full_text))
        }
    }
}
