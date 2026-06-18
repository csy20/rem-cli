//! Ollama provider implementation.
//! Contains Ollama-specific request/response types and API methods
//! (`chat_completion`, `chat_completion_stream`, `models`, `health`).

use std::sync::LazyLock;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::json;

use super::tools::{ToolCall, ToolResponse, ToolSpec};
use super::STREAM_CANCELLED;
use std::sync::atomic::Ordering;

static NUM_THREADS: LazyLock<usize> = LazyLock::new(|| {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
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

impl super::Provider {
    pub(super) async fn list_models_ollama(&self) -> Result<Vec<String>> {
        let url = super::api_url(&self.base_url, "tags");
        let resp = self.client.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("Ollama unreachable at {}", self.base_url));
        }
        let parsed: OllamaTagsResponse = resp.json().await.context("invalid tags response")?;
        Ok(parsed.models.into_iter().map(|m| m.name).collect())
    }

    pub(super) async fn healthcheck_ollama(&self) -> Result<()> {
        let models = self.list_models_ollama().await?;
        if models.is_empty() {
            return Err(anyhow!(
                "Ollama reachable but no models are installed. Pull one with `ollama pull rem-coder:latest`"
            ));
        }
        Ok(())
    }

    pub(super) async fn complete_json_ollama(
        &self,
        user_prompt: &str,
    ) -> Result<crate::ModelReply> {
        let url = super::api_url(&self.base_url, "generate");
        let final_prompt = format!(
            "{}\n\nUser request:\n{}\n\nReturn JSON only.",
            self.system_prompt, user_prompt
        );
        let payload = json!({
            "model": self.model,
            "prompt": final_prompt,
            "stream": false,
            "options": { "num_predict": 4096, "num_ctx": self.model_ctx, "num_thread": *NUM_THREADS },
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

        let resp = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("failed to call Ollama")?;
        if !resp.status().is_success() {
            return match self.handle_ollama_error(resp, &url).await {
                Err(e) => Err(e),
                Ok(_) => unreachable!(),
            };
        }

        let raw: OllamaJsonResponse = resp.json().await.context("invalid Ollama response")?;
        Self::parse_json_fallback(&raw.response)
    }

    pub(super) async fn complete_chat_stream_ollama(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let url = super::api_url(&self.base_url, "generate");
        let final_prompt = if history.is_empty() {
            format!("{}\n\nUser: {}\n\nREM:", system_prompt, user_prompt)
        } else {
            format!(
                "{}\n\n{}User: {}\n\nREM:",
                system_prompt, history, user_prompt
            )
        };
        let payload = json!({
            "model": self.model,
            "prompt": final_prompt,
            "stream": true,
            "options": { "num_predict": 4096, "num_ctx": self.model_ctx, "num_thread": *NUM_THREADS }
        });
        let resp = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("failed to call Ollama")?;
        if !resp.status().is_success() {
            return self.handle_ollama_error(resp, &url).await;
        }

        self.stream_ollama_response(resp).await
    }

    pub(super) async fn complete_chat_vision_ollama(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        _mime_type: &str,
        base64_data: &str,
    ) -> Result<String> {
        let url = super::api_url(&self.base_url, "generate");
        let final_prompt = if history.is_empty() {
            format!("{}\n\nUser: {}\n\nREM:", system_prompt, user_prompt)
        } else {
            format!(
                "{}\n\n{}User: {}\n\nREM:",
                system_prompt, history, user_prompt
            )
        };
        let payload = json!({
            "model": self.model,
            "prompt": final_prompt,
            "stream": true,
            "images": [base64_data],
            "options": { "num_predict": 4096, "num_ctx": self.model_ctx, "num_thread": *NUM_THREADS }
        });
        let resp = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("failed to call Ollama vision API")?;
        if !resp.status().is_success() {
            return self.handle_ollama_error(resp, &url).await;
        }

        self.stream_ollama_response(resp).await
    }

    pub(super) async fn complete_chat_stream_tools_ollama(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        tool_specs: &[ToolSpec],
    ) -> Result<ToolResponse> {
        let url = super::api_url(&self.base_url, "chat");

        let mut messages: Vec<serde_json::Value> = vec![];
        messages.push(json!({"role": "system", "content": system_prompt}));
        if !history.is_empty() {
            messages.push(json!({"role": "user", "content": history}));
        }
        messages.push(json!({"role": "user", "content": user_prompt}));

        let tools: Vec<serde_json::Value> = tool_specs.iter().map(|t| t.to_openai_tool()).collect();

        let mut payload = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            "options": { "num_predict": 4096, "num_ctx": self.model_ctx, "num_thread": *NUM_THREADS }
        });
        if !tools.is_empty() {
            payload["tools"] = json!(tools);
        }

        let resp = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("failed to call Ollama chat API")?;

        if !resp.status().is_success() {
            // If tool calling not supported, fall back to plain text
            let text = self
                .complete_chat_stream_ollama(user_prompt, system_prompt, history)
                .await?;
            return Ok(ToolResponse::Text(text));
        }

        let mut full_text = String::with_capacity(4096);
        let mut tool_calls: Vec<(i64, String, String, String)> = Vec::new();
        let mut stream = resp.bytes_stream();
        let mut buf = String::with_capacity(4096);
        let mut cursor = 0usize;

        loop {
            if STREAM_CANCELLED.load(Ordering::SeqCst) {
                break;
            }
            use futures_util::StreamExt;
            let chunk =
                match tokio::time::timeout(std::time::Duration::from_secs(60), stream.next()).await
                {
                    Ok(Some(Ok(c))) => c,
                    Ok(Some(Err(e))) => return Err(anyhow!("stream read error: {}", e)),
                    Ok(None) => break,
                    Err(_) => return Err(anyhow!("stream timed out")),
                };
            buf.push_str(&String::from_utf8_lossy(&chunk));
            loop {
                let tail = &buf[cursor..];
                match tail.find('\n') {
                    Some(pos) => {
                        let line = &tail[..pos];
                        cursor += pos + 1;
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if let Ok(obj) = serde_json::from_str::<OllamaChatStreamLine>(trimmed) {
                            if let Some(ref msg) = obj.message {
                                if let Some(ref content) = msg.content {
                                    full_text.push_str(content);
                                }
                                if let Some(ref calls) = msg.tool_calls {
                                    for tc in calls {
                                        let idx = tool_calls.len() as i64;
                                        let name = tc
                                            .function
                                            .as_ref()
                                            .and_then(|f| f.name.clone())
                                            .unwrap_or_default();
                                        let args = tc
                                            .function
                                            .as_ref()
                                            .and_then(|f| {
                                                f.arguments.as_ref().map(|a| a.to_string())
                                            })
                                            .unwrap_or_default();
                                        tool_calls.push((idx, String::new(), name, args));
                                    }
                                }
                            }
                            if obj.done == Some(true) {
                                break;
                            }
                        }
                    }
                    None => break,
                }
            }
            if full_text.len() > super::MAX_RESPONSE_BYTES {
                return Err(anyhow!(
                    "response too large ({} bytes)",
                    super::MAX_RESPONSE_BYTES
                ));
            }
        }

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
