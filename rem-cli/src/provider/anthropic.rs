use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use super::tools::{ToolCall, ToolResponse, ToolSpec};
use super::{Provider, ProviderBackend};

#[derive(Debug, Clone, Default)]
pub(crate) struct AnthropicUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_input_tokens: u32,
    pub cache_read_input_tokens: u32,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicResponse {
    pub content: Option<Vec<AnthropicContentBlock>>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicContentBlock {
    pub text: Option<String>,
    #[serde(rename = "type")]
    #[serde(default)]
    pub block_type: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub input: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicStreamChunk {
    #[serde(rename = "type")]
    pub chunk_type: Option<String>,
    pub delta: Option<AnthropicDelta>,
    pub content_block: Option<AnthropicContentBlock>,
    pub index: Option<u32>,
    #[serde(default)]
    pub usage: Option<AnthropicStreamUsage>,
    #[serde(default)]
    pub message: Option<AnthropicStreamMessage>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicStreamUsage {
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicStreamMessage {
    pub usage: Option<AnthropicStreamUsage>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicDelta {
    pub text: Option<String>,
    #[serde(rename = "type")]
    #[serde(default)]
    pub delta_type: Option<String>,
    #[serde(default)]
    pub partial_json: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicModelsResponse {
    pub data: Option<Vec<AnthropicModelEntry>>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicModelEntry {
    #[allow(dead_code)]
    #[serde(rename = "type")]
    pub _type: Option<String>,
    pub id: Option<String>,
    #[allow(dead_code)]
    pub display_name: Option<String>,
}

pub(super) struct AnthropicBackend;

#[async_trait]
impl ProviderBackend for AnthropicBackend {
    async fn list_models(&self, provider: &Provider) -> Result<Vec<String>> {
        let key = provider.api_key.as_deref().unwrap_or("");
        let base = provider.base_url.trim_end_matches('/');
        let url = format!("{}/v1/models", base);
        let resp = provider
            .client
            .get(url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(anyhow!("Anthropic API unreachable"));
        }
        let parsed: AnthropicModelsResponse =
            resp.json().await.context("invalid Anthropic response")?;
        Ok(parsed
            .data
            .unwrap_or_default()
            .into_iter()
            .filter_map(|m| m.id)
            .collect())
    }

    async fn complete_chat_stream_with_vision(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        mime_type: &str,
        base64_data: &str,
    ) -> Result<String> {
        let key = provider.api_key.as_deref().unwrap_or("");
        let base = provider.base_url.trim_end_matches('/');
        let url = format!("{}/v1/messages", base);

        let mut content_blocks: Vec<serde_json::Value> = vec![];
        if !history.is_empty() {
            content_blocks.push(json!({"type": "text", "text": format!("[Previous conversation]:\n{}", history), "cache_control": {"type": "ephemeral"}}));
        }
        content_blocks.push(json!({"type": "text", "text": user_prompt}));
        content_blocks.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": mime_type,
                "data": base64_data
            }
        }));

        let system_with_cache = json!([
            {"type": "text", "text": system_prompt, "cache_control": {"type": "ephemeral"}}
        ]);

        let payload = json!({
            "model": provider.model,
            "system": system_with_cache,
            "messages": [
                {"role": "user", "content": content_blocks}
            ],
            "max_tokens": 4096,
            "stream": true
        });

        let resp = provider
            .client
            .post(url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .context("failed to call Anthropic vision API")?;

        if !resp.status().is_success() {
            return Err(provider.parse_api_error("Anthropic", resp).await);
        }

        provider.stream_anthropic_sse(resp).await
    }

    async fn complete_json(
        &self,
        provider: &Provider,
        user_prompt: &str,
    ) -> Result<crate::ModelReply> {
        let key = provider.api_key.as_deref().unwrap_or("");
        let base = provider.base_url.trim_end_matches('/');
        let url = format!("{}/v1/messages", base);

        let system_with_cache = json!([
            {"type": "text", "text": provider.system_prompt.clone(), "cache_control": {"type": "ephemeral"}}
        ]);

        let payload = json!({
            "model": provider.model,
            "system": system_with_cache,
            "messages": [
                {"role": "user", "content": format!("{}\n\nReturn JSON only. Respond with a valid JSON object.", user_prompt)}
            ],
            "max_tokens": 512
        });

        let resp = provider
            .client
            .post(url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(provider.parse_api_error("Anthropic", resp).await);
        }

        let parsed: AnthropicResponse = resp.json().await.context("invalid Anthropic response")?;
        let text = parsed
            .content
            .and_then(|c| c.into_iter().next())
            .and_then(|b| b.text)
            .unwrap_or_default();

        Provider::parse_json_fallback(&text)
    }

    async fn complete_chat_stream(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let key = provider.api_key.as_deref().unwrap_or("");
        let base = provider.base_url.trim_end_matches('/');
        let url = format!("{}/v1/messages", base);

        let mut messages: Vec<serde_json::Value> = vec![];
        if !history.is_empty() {
            messages.push(json!({"role": "user", "content": [
                {"type": "text", "text": format!("[Previous conversation]:\n{}", history), "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": user_prompt}
            ]}));
        } else {
            messages.push(json!({"role": "user", "content": user_prompt}));
        }

        let system_with_cache = json!([
            {"type": "text", "text": system_prompt, "cache_control": {"type": "ephemeral"}}
        ]);

        let mut payload = json!({
            "model": provider.model,
            "system": system_with_cache,
            "messages": messages,
            "max_tokens": 4096,
            "stream": true
        });

        if provider.reasoning_config.enabled
            && crate::reasoning::is_reasoning_model(&provider.model)
        {
            payload["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": provider.reasoning_config.thinking_budget
            });
        }

        let resp = provider
            .client
            .post(url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .context("failed to call Anthropic API")?;

        if !resp.status().is_success() {
            return Err(provider.parse_api_error("Anthropic", resp).await);
        }

        provider.stream_anthropic_sse(resp).await
    }

    async fn complete_chat_stream_with_tools(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        tool_specs: &[ToolSpec],
    ) -> Result<ToolResponse> {
        let key = provider.api_key.as_deref().unwrap_or("");
        let base = provider.base_url.trim_end_matches('/');
        let url = format!("{}/v1/messages", base);

        let mut messages: Vec<serde_json::Value> = vec![];
        if !history.is_empty() {
            messages.push(json!({"role": "user", "content": [
                {"type": "text", "text": format!("[Previous conversation]:\n{}", history), "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": user_prompt}
            ]}));
        } else {
            messages.push(json!({"role": "user", "content": user_prompt}));
        }

        let tools: Vec<serde_json::Value> =
            tool_specs.iter().map(|t| t.to_anthropic_tool()).collect();

        let system_with_cache = json!([
            {"type": "text", "text": system_prompt, "cache_control": {"type": "ephemeral"}}
        ]);

        let mut payload = json!({
            "model": provider.model,
            "system": system_with_cache,
            "messages": messages,
            "max_tokens": 4096,
            "stream": true
        });
        if !tools.is_empty() {
            payload["tools"] = json!(tools);
        }

        let resp = provider
            .client
            .post(url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .context("failed to call Anthropic API")?;

        if !resp.status().is_success() {
            return Err(provider.parse_api_error("Anthropic", resp).await);
        }

        let mut full_text = String::with_capacity(4096);
        let mut tool_calls: Vec<(u32, String, String, String)> = Vec::new();

        Provider::stream_buf(resp, |trimmed| {
            if trimmed.starts_with("event: ") {
                return Ok(true);
            }
            if let Some(data) = trimmed.strip_prefix("data: ") {
                if let Ok(chunk) = serde_json::from_str::<AnthropicStreamChunk>(data) {
                    let ct = chunk.chunk_type.as_deref().unwrap_or("");
                    match ct {
                        "content_block_start" => {
                            if let Some(ref block) = chunk.content_block {
                                if block.block_type.as_deref() == Some("tool_use") {
                                    let idx = chunk.index.unwrap_or(0);
                                    let id = block.id.clone().unwrap_or_default();
                                    let name = block.name.clone().unwrap_or_default();
                                    if let Some(pos) =
                                        tool_calls.iter().position(|(i, _, _, _)| *i == idx)
                                    {
                                        tool_calls[pos].1 = id;
                                        tool_calls[pos].2 = name;
                                    } else {
                                        tool_calls.push((idx, id, name, String::new()));
                                    }
                                } else if block.block_type.as_deref() == Some("text") {
                                    if let Some(ref text) = block.text {
                                        full_text.push_str(text);
                                    }
                                }
                            }
                        }
                        "content_block_delta" => {
                            if let Some(ref delta) = chunk.delta {
                                if delta.delta_type.as_deref() == Some("input_json_delta") {
                                    let idx = chunk.index.unwrap_or(0);
                                    if let Some(ref partial) = delta.partial_json {
                                        if let Some(pos) =
                                            tool_calls.iter().position(|(i, _, _, _)| *i == idx)
                                        {
                                            tool_calls[pos].3.push_str(partial);
                                        }
                                    }
                                } else if let Some(ref text) = delta.text {
                                    full_text.push_str(text);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            if full_text.len() > super::MAX_RESPONSE_BYTES {
                return Err(anyhow!(
                    "response too large ({} bytes)",
                    super::MAX_RESPONSE_BYTES
                ));
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
