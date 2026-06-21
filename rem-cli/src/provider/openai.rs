use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use super::tools::{ToolResponse, ToolSpec};
use super::{Provider, ProviderBackend};

/// Response from a non-streaming OpenAI chat completion.
#[derive(Debug, Deserialize)]
pub struct OpenAIResponse {
    pub choices: Vec<OpenAIChoice>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIChoice {
    pub message: OpenAIMessage,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIMessage {
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIStreamChunk {
    pub choices: Vec<OpenAIStreamChoice>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIStreamChoice {
    pub delta: OpenAIStreamDelta,
    #[serde(default)]
    #[allow(dead_code)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIStreamDelta {
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<OpenAIStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIStreamToolCall {
    pub index: i64,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type")]
    #[serde(default)]
    #[allow(dead_code)]
    pub call_type: Option<String>,
    #[serde(default)]
    pub function: Option<OpenAIStreamToolCallFunction>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIStreamToolCallFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct AccumulatedToolCalls {
    pub calls: Vec<AccumulatedToolCall>,
}

#[derive(Debug)]
pub(crate) struct AccumulatedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

impl AccumulatedToolCalls {
    pub fn absorb_chunk(&mut self, tool_calls: &[OpenAIStreamToolCall]) {
        for tc in tool_calls {
            let idx = tc.index as usize;
            while self.calls.len() <= idx {
                self.calls.push(AccumulatedToolCall {
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                });
            }
            if let Some(ref id) = tc.id {
                if self.calls[idx].id.is_empty() {
                    self.calls[idx].id = id.clone();
                }
            }
            if let Some(ref func) = tc.function {
                if let Some(ref name) = func.name {
                    if self.calls[idx].name.is_empty() {
                        self.calls[idx].name = name.clone();
                    }
                }
                if let Some(ref args) = func.arguments {
                    self.calls[idx].arguments.push_str(args);
                }
            }
        }
    }

    pub fn to_tool_calls(&self) -> Vec<super::tools::ToolCall> {
        self.calls
            .iter()
            .map(|c| super::tools::ToolCall {
                id: c.id.clone(),
                name: c.name.clone(),
                arguments: serde_json::from_str(&c.arguments).unwrap_or(serde_json::Value::Null),
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }
}

#[derive(Debug, Deserialize)]
pub struct OpenAIModelsResponse {
    pub data: Vec<OpenAIModelEntry>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIModelEntry {
    pub id: String,
}

pub(super) struct OpenAIBackend;

#[async_trait]
impl ProviderBackend for OpenAIBackend {
    async fn list_models(&self, provider: &Provider) -> Result<Vec<String>> {
        let url = provider.openai_models_url();
        let resp = provider.add_openai_auth(provider.client.get(&url)).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("OpenAI API unreachable at {}", provider.base_url));
        }
        let parsed: OpenAIModelsResponse = resp.json().await.context("invalid models response")?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }

    async fn complete_json(&self, provider: &Provider, user_prompt: &str) -> Result<crate::ModelReply> {
        let url = provider.openai_chat_url();
        let resp = provider
            .add_openai_auth(provider.client.post(&url))
            .json(&json!({
                "model": provider.model,
                "messages": [
                    {"role": "system", "content": provider.system_prompt},
                    {"role": "user", "content": format!("{}\n\nReturn JSON only.", user_prompt)}
                ],
                "temperature": 0.3,
                "max_tokens": 512,
                "response_format": {"type": "json_object"}
            }))
            .send()
            .await
            .context("failed to call OpenAI API")?;

        if !resp.status().is_success() {
            return Err(provider.parse_api_error("OpenAI", resp).await);
        }

        let parsed: OpenAIResponse = resp.json().await.context("invalid OpenAI response")?;
        let content = parsed.choices.first().map(|c| c.message.content.as_str()).unwrap_or("");

        Provider::parse_json_fallback(content)
    }

    async fn complete_chat_stream(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let url = provider.openai_chat_url();

        let is_reasoning = crate::reasoning::is_reasoning_model(&provider.model);
        let no_system = crate::reasoning::system_prompt_not_supported(&provider.model);
        let no_stream = crate::reasoning::requires_non_streaming(&provider.model);

        let mut messages: Vec<serde_json::Value> = vec![];
        if no_system {
            let combined = format!("{}\n\n{}", system_prompt, user_prompt);
            messages.push(json!({"role": "user", "content": combined}));
        } else {
            messages.push(json!({"role": "system", "content": system_prompt}));
            if !history.is_empty() {
                messages.push(json!({"role": "user", "content": history}));
            }
            messages.push(json!({"role": "user", "content": user_prompt}));
        }

        let mut payload = serde_json::Map::new();
        payload.insert("model".into(), json!(provider.model));
        payload.insert("messages".into(), json!(messages));
        payload.insert("max_tokens".into(), json!(4096));

        if is_reasoning && provider.reasoning_config.enabled {
            let effort = provider.reasoning_config.effort.as_str();
            payload.insert("reasoning_effort".into(), json!(effort));
        } else {
            payload.insert("temperature".into(), json!(0.7));
        }

        if !no_stream {
            payload.insert("stream".into(), json!(true));
        }

        if no_stream {
            let resp = provider
                .add_openai_auth(provider.client.post(&url))
                .json(&payload)
                .send()
                .await
                .context("failed to call OpenAI API")?;
            if !resp.status().is_success() {
                return Err(provider.parse_api_error("OpenAI", resp).await);
            }
            let parsed: OpenAIResponse = resp.json().await.context("invalid OpenAI response")?;
            let content = parsed.choices.first().map(|c| c.message.content.as_str()).unwrap_or("");
            return Ok(content.to_string());
        }

        let resp = provider
            .add_openai_auth(provider.client.post(&url))
            .json(&payload)
            .send()
            .await
            .context("failed to call OpenAI API")?;

        if !resp.status().is_success() {
            return Err(provider.parse_api_error("OpenAI", resp).await);
        }

        provider.stream_sse_response(resp).await
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
        let url = provider.openai_chat_url();
        let data_uri = format!("data:{};base64,{}", mime_type, base64_data);

        let mut messages: Vec<serde_json::Value> = vec![];
        messages.push(json!({"role": "system", "content": system_prompt}));
        if !history.is_empty() {
            messages.push(json!({"role": "user", "content": history}));
        }
        messages.push(json!({
            "role": "user",
            "content": [
                {"type": "text", "text": user_prompt},
                {"type": "image_url", "image_url": {"url": data_uri}}
            ]
        }));

        let payload = json!({
            "model": provider.model,
            "messages": messages,
            "stream": true,
            "max_tokens": 4096
        });

        let resp = provider
            .add_openai_auth(provider.client.post(&url))
            .json(&payload)
            .send()
            .await
            .context("failed to call OpenAI vision API")?;

        if !resp.status().is_success() {
            return Err(provider.parse_api_error("OpenAI", resp).await);
        }

        provider.stream_sse_response(resp).await
    }

    async fn complete_chat_stream_with_tools(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        tool_specs: &[ToolSpec],
    ) -> Result<ToolResponse> {
        let url = provider.openai_chat_url();
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
            "temperature": 0.7,
            "max_tokens": 4096,
        });
        if !tools.is_empty() {
            payload["tools"] = json!(tools);
            payload["tool_choice"] = json!("auto");
        }

        let resp = provider
            .add_openai_auth(provider.client.post(&url))
            .json(&payload)
            .send()
            .await
            .context("failed to call OpenAI API")?;

        if !resp.status().is_success() {
            return Err(provider.parse_api_error("OpenAI", resp).await);
        }

        Provider::stream_openai_tool_response(resp).await
    }
}
