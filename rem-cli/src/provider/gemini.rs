//! Google Gemini provider implementation.
//! Contains Gemini-specific request/response types and API methods
//! (`chat_completion`, `chat_completion_stream`, `models`, `health`).

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::json;
use futures_util::StreamExt;
use tokio::time::{timeout, Duration};

#[derive(Debug, Deserialize)]
pub struct GeminiResponse {
    pub candidates: Option<Vec<GeminiCandidate>>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiCandidate {
    pub content: Option<GeminiContent>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiContent {
    pub parts: Option<Vec<GeminiPart>>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiPart {
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiStreamChunk {
    pub candidates: Option<Vec<GeminiStreamCandidate>>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiStreamCandidate {
    pub content: Option<GeminiStreamContent>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiStreamContent {
    pub parts: Option<Vec<GeminiPart>>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiModelsResponse {
    pub models: Option<Vec<GeminiModelEntry>>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiModelEntry {
    pub name: String,
    #[allow(dead_code)]
    pub display_name: Option<String>,
}

impl super::Provider {
    fn gemini_url(&self, path: &str) -> String {
        format!("{}/v1beta{}", self.base_url.trim_end_matches('/'), path)
    }

    pub(super) async fn list_models_gemini(&self) -> Result<Vec<String>> {
        let url = self.gemini_url("/models");
        let resp = self
            .client
            .get(&url)
            .header("x-goog-api-key", self.api_key_str())
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(anyhow!("Gemini API unreachable"));
        }
        let parsed: GeminiModelsResponse = resp.json().await.context("invalid Gemini response")?;
        Ok(parsed
            .models
            .unwrap_or_default()
            .into_iter()
            .map(|m| {
                m.name
                    .strip_prefix("models/")
                    .unwrap_or(&m.name)
                    .to_string()
            })
            .filter(|n| n.contains("gemini"))
            .collect())
    }

    pub(super) async fn healthcheck_gemini(&self) -> Result<()> {
        if self.api_key.as_deref().unwrap_or("").is_empty() {
            return Err(anyhow!(
                "Gemini requires --api-key or GEMINI_API_KEY env var"
            ));
        }
        let models = self.list_models_gemini().await?;
        if models.is_empty() {
            return Err(anyhow!("No Gemini models available"));
        }
        Ok(())
    }

    pub(super) async fn complete_json_gemini(&self, user_prompt: &str) -> Result<crate::ModelReply> {
        let url = self.gemini_url(&format!("/models/{}:generateContent", self.model));
        let payload = json!({
            "contents": [{"parts": [{"text": format!("{}\n\nUser request:\n{}\n\nReturn JSON only.", self.system_prompt, user_prompt)}]}],
            "generationConfig": {
                "temperature": 0.3,
                "maxOutputTokens": 512
            }
        });

        let resp = self
            .client
            .post(&url)
            .header("x-goog-api-key", self.api_key_str())
            .json(&payload)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(self.parse_api_error("Gemini", resp).await);
        }

        let parsed: GeminiResponse = resp.json().await.context("invalid Gemini response")?;
        let text = parsed
            .candidates
            .and_then(|c| c.into_iter().next())
            .and_then(|c| c.content)
            .and_then(|c| c.parts)
            .and_then(|p| p.into_iter().next())
            .and_then(|p| p.text)
            .unwrap_or_default();

        Self::parse_json_fallback(&text)
    }

    pub(super) async fn complete_chat_stream_gemini(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let url = self.gemini_url(&format!("/models/{}:streamGenerateContent?alt=sse", self.model));

        let mut contents = vec![];
        if !history.is_empty() {
            contents.push(json!({"role": "user", "parts": [{"text": history}]}));
        }
        contents.push(json!({"role": "user", "parts": [{"text": user_prompt}]}));

        let mut payload = json!({
            "contents": contents,
            "generationConfig": {
                "temperature": 0.7,
                "maxOutputTokens": 4096
            }
        });

        if !system_prompt.is_empty() {
            payload["systemInstruction"] = json!({"parts": [{"text": system_prompt}]});
        }

        let resp = self
            .client
            .post(&url)
            .header("x-goog-api-key", self.api_key_str())
            .json(&payload)
            .send()
            .await
            .context("failed to call Gemini API")?;

        if !resp.status().is_success() {
            return Err(self.parse_api_error("Gemini", resp).await);
        }

        let mut stream = resp.bytes_stream();
        let mut full = String::new();
        let mut buf = String::new();
        let mut cursor = 0usize;

        loop {
            if super::STREAM_CANCELLED.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            let chunk = match timeout(Duration::from_secs(60), stream.next()).await {
                Ok(Some(Ok(c))) => c,
                Ok(Some(Err(e))) => return Err(anyhow!("stream read error: {}", e)),
                Ok(None) => break,
                Err(_) => return Err(anyhow!("stream timed out (no data for 60s)")),
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));

            loop {
                let tail = &buf[cursor..];
                match tail.find('\n') {
                    Some(pos) => {
                        let line = &tail[..pos];
                        cursor += pos + 1;
                        let trimmed = line.trim();
                        if trimmed.is_empty() || trimmed.starts_with(':') {
                            continue;
                        }
                        if let Some(data) = trimmed.strip_prefix("data: ") {
                            if data == "[DONE]" {
                                continue;
                            }
                            if let Ok(chunk) = serde_json::from_str::<GeminiStreamChunk>(data) {
                                if let Some(text) = chunk
                                    .candidates
                                    .and_then(|c| c.into_iter().next())
                                    .and_then(|c| c.content)
                                    .and_then(|c| c.parts)
                                    .and_then(|p| p.into_iter().next())
                                    .and_then(|p| p.text)
                                {
                                    full.push_str(&text);
                                }
                            }
                        }
                    }
                    None => break,
                }
            }
        }

        Ok(full)
    }
}
