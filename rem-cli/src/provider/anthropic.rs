//! Anthropic (Claude) provider implementation.
//! Contains Anthropic-specific request/response types and API methods
//! (`chat_completion`, `chat_completion_stream`, `models`, `health`).

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct AnthropicResponse {
    pub content: Option<Vec<AnthropicContentBlock>>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicContentBlock {
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicStreamChunk {
    #[serde(rename = "type")]
    pub chunk_type: Option<String>,
    pub delta: Option<AnthropicDelta>,
    pub content_block: Option<AnthropicContentBlock>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicDelta {
    pub text: Option<String>,
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

impl super::Provider {
    pub(super) async fn list_models_anthropic(&self) -> Result<Vec<String>> {
        let key = self.api_key.as_deref().unwrap_or("");
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/v1/models", base);
        let resp = self
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

    pub(super) async fn healthcheck_anthropic(&self) -> Result<()> {
        if self.api_key.as_deref().unwrap_or("").is_empty() {
            return Err(anyhow!(
                "Anthropic requires --api-key or ANTHROPIC_API_KEY env var"
            ));
        }
        let models = self.list_models_anthropic().await?;
        if models.is_empty() {
            return Err(anyhow!("No Anthropic models available"));
        }
        Ok(())
    }

    pub(super) async fn complete_json_anthropic(
        &self,
        user_prompt: &str,
    ) -> Result<crate::ModelReply> {
        let key = self.api_key.as_deref().unwrap_or("");
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/v1/messages", base);
        let payload = json!({
            "model": self.model,
            "system": self.system_prompt,
            "messages": [
                {"role": "user", "content": format!("{}\n\nReturn JSON only. Respond with a valid JSON object.", user_prompt)}
            ],
            "max_tokens": 512
        });

        let resp = self
            .client
            .post(url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(self.parse_api_error("Anthropic", resp).await);
        }

        let parsed: AnthropicResponse = resp.json().await.context("invalid Anthropic response")?;
        let text = parsed
            .content
            .and_then(|c| c.into_iter().next())
            .and_then(|b| b.text)
            .unwrap_or_default();

        Self::parse_json_fallback(&text)
    }

    pub(super) async fn complete_chat_stream_anthropic(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let key = self.api_key.as_deref().unwrap_or("");
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/v1/messages", base);

        let mut messages: Vec<serde_json::Value> = vec![];
        if !history.is_empty() {
            messages.push(json!({"role": "user", "content": format!("[Previous conversation]:\n{}", history)}));
        }
        messages.push(json!({"role": "user", "content": user_prompt}));

        let payload = json!({
            "model": self.model,
            "system": system_prompt,
            "messages": messages,
            "max_tokens": 4096,
            "stream": true
        });

        let resp = self
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
            return Err(self.parse_api_error("Anthropic", resp).await);
        }

        self.stream_anthropic_sse(resp).await
    }
}
