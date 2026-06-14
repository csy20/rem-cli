//! Ollama provider implementation.
//! Contains Ollama-specific request/response types and API methods
//! (`chat_completion`, `chat_completion_stream`, `models`, `health`).

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::json;

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
        let num_thread = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let payload = json!({
            "model": self.model,
            "prompt": final_prompt,
            "stream": false,
            "options": { "num_predict": 512, "num_ctx": self.model_ctx, "num_thread": num_thread },
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
            let status = resp.status();
            let body = resp
                .text()
                .await
                .unwrap_or_else(|e| format!("(read error: {})", e));
            let err_msg = serde_json::from_str::<super::LlmErrorResponse>(&body)
                .map(|v| v.error.to_string())
                .unwrap_or_else(|_| body.clone());
            if status.as_u16() == 404 && err_msg.to_lowercase().contains("model") {
                return Err(anyhow!(
                    "Model '{}' not found. Pull it: `ollama pull {}`",
                    self.model,
                    self.model
                ));
            }
            if status.as_u16() == 404 {
                return Err(anyhow!(
                    "Endpoint not found (404 at {}). Check --ollama-url",
                    url
                ));
            }
            return Err(anyhow!("Ollama failed: {} — {}", status, err_msg));
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
        let num_thread = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let payload = json!({
            "model": self.model,
            "prompt": final_prompt,
            "stream": true,
            "options": { "num_predict": 512, "num_ctx": self.model_ctx, "num_thread": num_thread }
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
}
