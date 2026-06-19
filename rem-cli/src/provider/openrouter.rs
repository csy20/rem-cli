use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::json;

use super::openai::OpenAIResponse;
use super::tools::{ToolResponse, ToolSpec};
use super::{Provider, ProviderBackend};

pub(super) struct OpenRouterBackend;

#[async_trait]
impl ProviderBackend for OpenRouterBackend {
    async fn list_models(&self, provider: &Provider) -> Result<Vec<String>> {
        let url = provider.openai_models_url();
        let resp = provider
            .add_openai_auth(provider.client.get(&url))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(anyhow!(
                "OpenRouter API unreachable at {}",
                provider.base_url
            ));
        }
        let parsed: super::openai::OpenAIModelsResponse = resp
            .json()
            .await
            .context("invalid OpenRouter models response")?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }

    async fn complete_json(
        &self,
        provider: &Provider,
        user_prompt: &str,
    ) -> Result<crate::ModelReply> {
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
            .context("failed to call OpenRouter API")?;
        if !resp.status().is_success() {
            return Err(provider.parse_api_error("OpenRouter", resp).await);
        }
        let parsed: OpenAIResponse = resp.json().await.context("invalid OpenRouter response")?;
        let content = parsed
            .choices
            .first()
            .map(|c| c.message.content.as_str())
            .unwrap_or("");
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
        let mut messages: Vec<serde_json::Value> = vec![];
        messages.push(json!({"role": "system", "content": system_prompt}));
        if !history.is_empty() {
            messages.push(json!({"role": "user", "content": history}));
        }
        messages.push(json!({"role": "user", "content": user_prompt}));
        let resp = provider
            .add_openai_auth(provider.client.post(&url))
            .json(&json!({
                "model": provider.model,
                "messages": messages,
                "stream": true,
                "temperature": 0.7,
                "max_tokens": 4096,
            }))
            .send()
            .await
            .context("failed to call OpenRouter API")?;
        if !resp.status().is_success() {
            return Err(provider.parse_api_error("OpenRouter", resp).await);
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
        let resp = provider
            .add_openai_auth(provider.client.post(&url))
            .json(&json!({
                "model": provider.model,
                "messages": messages,
                "stream": true,
                "max_tokens": 4096
            }))
            .send()
            .await
            .context("failed to call OpenRouter vision API")?;
        if !resp.status().is_success() {
            return Err(provider.parse_api_error("OpenRouter", resp).await);
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
            .context("failed to call OpenRouter API")?;
        if !resp.status().is_success() {
            return Err(provider.parse_api_error("OpenRouter", resp).await);
        }

        Provider::stream_openai_tool_response(resp).await
    }
}
