use std::sync::LazyLock;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use super::tools::{ToolCall, ToolResponse, ToolSpec};
use super::{Provider, ProviderBackend};

static NUM_THREADS: LazyLock<usize> =
    LazyLock::new(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4));

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

pub(super) struct OllamaBackend;

#[async_trait]
impl ProviderBackend for OllamaBackend {
    async fn list_models(&self, provider: &Provider) -> Result<Vec<String>> {
        let url = super::api_url(&provider.base_url, "tags");
        let resp = provider.client.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("Ollama unreachable at {}", provider.base_url));
        }
        let parsed: OllamaTagsResponse = resp.json().await.context("invalid tags response")?;
        Ok(parsed.models.into_iter().map(|m| m.name).collect())
    }

    async fn complete_json(&self, provider: &Provider, user_prompt: &str) -> Result<crate::ModelReply> {
        let url = super::api_url(&provider.base_url, "generate");
        let final_prompt = format!(
            "{}\n\nUser request:\n{}\n\nReturn JSON only.",
            provider.system_prompt, user_prompt
        );
        let payload = json!({
            "model": provider.model,
            "prompt": final_prompt,
            "stream": false,
            "options": { "num_predict": 4096, "num_ctx": provider.model_ctx, "num_thread": *NUM_THREADS },
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

        let resp = provider
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("failed to call Ollama")?;
        if !resp.status().is_success() {
            return match provider.handle_ollama_error(resp, &url).await {
                Err(e) => Err(e),
                Ok(_) => unreachable!(),
            };
        }

        let raw: OllamaJsonResponse = resp.json().await.context("invalid Ollama response")?;
        Provider::parse_json_fallback(&raw.response)
    }

    async fn complete_chat_stream(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let url = super::api_url(&provider.base_url, "generate");
        let final_prompt = if history.is_empty() {
            format!("{}\n\nUser: {}\n\nREM:", system_prompt, user_prompt)
        } else {
            format!("{}\n\n{}User: {}\n\nREM:", system_prompt, history, user_prompt)
        };
        let payload = json!({
            "model": provider.model,
            "prompt": final_prompt,
            "stream": true,
            "options": { "num_predict": 4096, "num_ctx": provider.model_ctx, "num_thread": *NUM_THREADS }
        });
        let resp = provider
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("failed to call Ollama")?;
        if !resp.status().is_success() {
            return provider.handle_ollama_error(resp, &url).await;
        }

        provider.stream_ollama_response(resp).await
    }

    async fn complete_chat_stream_with_vision(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        _mime_type: &str,
        base64_data: &str,
    ) -> Result<String> {
        let url = super::api_url(&provider.base_url, "generate");
        let final_prompt = if history.is_empty() {
            format!("{}\n\nUser: {}\n\nREM:", system_prompt, user_prompt)
        } else {
            format!("{}\n\n{}User: {}\n\nREM:", system_prompt, history, user_prompt)
        };
        let payload = json!({
            "model": provider.model,
            "prompt": final_prompt,
            "stream": true,
            "images": [base64_data],
            "options": { "num_predict": 4096, "num_ctx": provider.model_ctx, "num_thread": *NUM_THREADS }
        });
        let resp = provider
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("failed to call Ollama vision API")?;
        if !resp.status().is_success() {
            return provider.handle_ollama_error(resp, &url).await;
        }

        provider.stream_ollama_response(resp).await
    }

    async fn complete_chat_stream_with_tools(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        tool_specs: &[ToolSpec],
    ) -> Result<ToolResponse> {
        let url = super::api_url(&provider.base_url, "chat");

        let mut messages: Vec<serde_json::Value> = vec![];
        messages.push(json!({"role": "system", "content": system_prompt}));
        if !history.is_empty() {
            messages.push(json!({"role": "user", "content": history}));
        }
        messages.push(json!({"role": "user", "content": user_prompt}));

        let tools: Vec<serde_json::Value> = tool_specs.iter().map(|t| t.to_openai_tool()).collect();

        let mut payload = json!({
            "model": provider.model,
            "messages": messages,
            "stream": true,
            "options": { "num_predict": 4096, "num_ctx": provider.model_ctx, "num_thread": *NUM_THREADS }
        });
        if !tools.is_empty() {
            payload["tools"] = json!(tools);
        }

        let resp = provider
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("failed to call Ollama chat API")?;

        if !resp.status().is_success() {
            let text = self
                .complete_chat_stream(provider, user_prompt, system_prompt, history)
                .await?;
            return Ok(ToolResponse::Text(text));
        }

        let mut full_text = String::with_capacity(4096);
        let mut tool_calls: Vec<(i64, String, String, String)> = Vec::new();

        Provider::stream_buf(resp, |trimmed| {
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
