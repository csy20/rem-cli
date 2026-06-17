//! Ollama provider implementation.
//! Contains Ollama-specific request/response types and API methods
//! (`chat_completion`, `chat_completion_stream`, `models`, `health`).

use std::sync::LazyLock;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::json;

use super::tools::{ToolResponse, ToolSpec};

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
        _tool_specs: &[ToolSpec],
    ) -> Result<ToolResponse> {
        // Ollama doesn't natively support tool calling in the /api/generate endpoint.
        // For Ollama >=0.5 we could use /api/chat with OpenAI-compatible format,
        // but for now fall back to plain text streaming.
        let text = self
            .complete_chat_stream_ollama(user_prompt, system_prompt, history)
            .await?;
        Ok(ToolResponse::Text(text))
    }
}
