//! OpenAI-compatible provider implementation.
//! Contains OpenAI-specific request/response types and API methods
//! (`chat_completion`, `chat_completion_stream`, `models`, `health`).

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::json;

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
}

#[derive(Debug, Deserialize)]
pub struct OpenAIStreamDelta {
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIModelsResponse {
    pub data: Vec<OpenAIModelEntry>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIModelEntry {
    pub id: String,
}

impl super::Provider {
    pub(super) async fn list_models_openai(&self) -> Result<Vec<String>> {
        let url = self.base_url.trim_end_matches('/').to_string() + "/models";
        let mut req = self.client.get(&url);
        if !self.api_key_str().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key_str()));
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("OpenAI API unreachable at {}", self.base_url));
        }
        let parsed: OpenAIModelsResponse = resp.json().await.context("invalid models response")?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }

    pub(super) async fn healthcheck_openai(&self) -> Result<()> {
        let _models = self.list_models_openai().await?;
        Ok(())
    }

    pub(super) async fn complete_json_openai(
        &self,
        user_prompt: &str,
    ) -> Result<crate::ModelReply> {
        let url = self.base_url.trim_end_matches('/').to_string() + "/chat/completions";
        let resp = self
            .client
            .post(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.as_deref().unwrap_or("")),
            )
            .json(&json!({
                "model": self.model,
                "messages": [
                    {"role": "system", "content": self.system_prompt},
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
            return Err(self.parse_api_error("OpenAI", resp).await);
        }

        let parsed: OpenAIResponse = resp.json().await.context("invalid OpenAI response")?;
        let content = parsed
            .choices
            .first()
            .map(|c| c.message.content.as_str())
            .unwrap_or("");

        Self::parse_json_fallback(content)
    }

    pub(super) async fn complete_chat_stream_openai(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let url = self.base_url.trim_end_matches('/').to_string() + "/chat/completions";
        let mut messages: Vec<serde_json::Value> = vec![];
        messages.push(json!({"role": "system", "content": system_prompt}));
        if !history.is_empty() {
            messages.push(json!({"role": "user", "content": history}));
        }
        messages.push(json!({"role": "user", "content": user_prompt}));

        let payload = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            "temperature": 0.7,
            "max_tokens": 4096
        });

        let mut req = self.client.post(&url);
        if !self.api_key_str().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key_str()));
        }
        let resp = req
            .json(&payload)
            .send()
            .await
            .context("failed to call OpenAI API")?;

        if !resp.status().is_success() {
            return Err(self.parse_api_error("OpenAI", resp).await);
        }

        self.stream_sse_response(resp).await
    }
}
