use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use super::tools::{ToolCall, ToolResponse, ToolSpec};
use super::{Provider, ProviderBackend};

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
    #[serde(default)]
    pub function_call: Option<GeminiFunctionCall>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiFunctionCall {
    pub name: Option<String>,
    #[serde(default)]
    pub args: Option<serde_json::Value>,
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

pub(super) struct GeminiBackend;

impl GeminiBackend {
    fn gemini_url(&self, provider: &Provider, path: &str) -> String {
        format!("{}/v1beta{}", provider.base_url.trim_end_matches('/'), path)
    }
}

#[async_trait]
impl ProviderBackend for GeminiBackend {
    async fn list_models(&self, provider: &Provider) -> Result<Vec<String>> {
        let url = self.gemini_url(provider, "/models");
        let resp = provider
            .client
            .get(&url)
            .header("x-goog-api-key", provider.api_key_str())
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
            .map(|m| m.name.strip_prefix("models/").unwrap_or(&m.name).to_string())
            .filter(|n| n.contains("gemini"))
            .collect())
    }

    async fn complete_json(&self, provider: &Provider, user_prompt: &str) -> Result<crate::ModelReply> {
        let url = self.gemini_url(provider, &format!("/models/{}:generateContent", provider.model));
        let payload = json!({
            "contents": [{"parts": [{"text": format!("{}\n\nUser request:\n{}\n\nReturn JSON only.", provider.system_prompt, user_prompt)}]}],
            "generationConfig": {
                "temperature": 0.3,
                "maxOutputTokens": 512
            }
        });

        let resp = provider
            .client
            .post(&url)
            .header("x-goog-api-key", provider.api_key_str())
            .json(&payload)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(provider.parse_api_error("Gemini", resp).await);
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

        Provider::parse_json_fallback(&text)
    }

    async fn complete_chat_stream(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let url = self.gemini_url(
            provider,
            &format!("/models/{}:streamGenerateContent?alt=sse", provider.model),
        );

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

        let resp = provider
            .client
            .post(&url)
            .header("x-goog-api-key", provider.api_key_str())
            .json(&payload)
            .send()
            .await
            .context("failed to call Gemini API")?;

        if !resp.status().is_success() {
            return Err(provider.parse_api_error("Gemini", resp).await);
        }

        provider.stream_gemini_sse(resp).await
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
        let url = self.gemini_url(
            provider,
            &format!("/models/{}:streamGenerateContent?alt=sse", provider.model),
        );

        let mut parts: Vec<serde_json::Value> = vec![];
        if !history.is_empty() {
            parts.push(json!({"text": format!("[Previous conversation]:\n{}", history)}));
        }
        parts.push(json!({"text": user_prompt}));
        parts.push(json!({
            "inline_data": {
                "mime_type": mime_type,
                "data": base64_data
            }
        }));

        let mut payload = json!({
            "contents": [{"parts": parts}],
            "generationConfig": {
                "temperature": 0.7,
                "maxOutputTokens": 4096
            }
        });

        if !system_prompt.is_empty() {
            payload["systemInstruction"] = json!({"parts": [{"text": system_prompt}]});
        }

        let resp = provider
            .client
            .post(&url)
            .header("x-goog-api-key", provider.api_key_str())
            .json(&payload)
            .send()
            .await
            .context("failed to call Gemini vision API")?;

        if !resp.status().is_success() {
            return Err(provider.parse_api_error("Gemini", resp).await);
        }

        provider.stream_gemini_sse(resp).await
    }

    async fn complete_chat_stream_with_tools(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        tool_specs: &[ToolSpec],
    ) -> Result<ToolResponse> {
        let url = self.gemini_url(
            provider,
            &format!("/models/{}:streamGenerateContent?alt=sse", provider.model),
        );

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

        if !tool_specs.is_empty() {
            let declarations: Vec<serde_json::Value> =
                tool_specs.iter().map(|t| t.to_gemini_function_declaration()).collect();
            payload["tools"] = json!([{"function_declarations": declarations}]);
        }

        let resp = provider
            .client
            .post(&url)
            .header("x-goog-api-key", provider.api_key_str())
            .json(&payload)
            .send()
            .await
            .context("failed to call Gemini API")?;

        if !resp.status().is_success() {
            return Err(provider.parse_api_error("Gemini", resp).await);
        }

        let mut full_text = String::with_capacity(4096);
        let mut early_tool_response: Option<ToolResponse> = None;

        Provider::stream_buf(resp, |trimmed| {
            if trimmed.is_empty() || trimmed.starts_with(':') {
                return Ok(true);
            }
            if let Some(data) = trimmed.strip_prefix("data: ") {
                if let Ok(chunk) = serde_json::from_str::<GeminiStreamChunk>(data) {
                    if let Some(candidates) = chunk.candidates {
                        if let Some(candidate) = candidates.into_iter().next() {
                            if let Some(content) = candidate.content {
                                if let Some(parts) = content.parts {
                                    for part in parts {
                                        if let Some(text) = part.text {
                                            full_text.push_str(&text);
                                        }
                                        if let Some(fc) = part.function_call {
                                            let name = fc.name.unwrap_or_default();
                                            let args = fc.args.unwrap_or(serde_json::Value::Null);
                                            if !name.is_empty() {
                                                early_tool_response = Some(ToolResponse::ToolCalls(vec![ToolCall {
                                                    id: format!("fc_{}", name),
                                                    name,
                                                    arguments: args,
                                                }]));
                                                return Ok(false);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if full_text.len() > super::MAX_RESPONSE_BYTES {
                return Err(anyhow!("response too large ({} bytes)", super::MAX_RESPONSE_BYTES));
            }
            Ok(true)
        })
        .await?;

        if let Some(response) = early_tool_response {
            return Ok(response);
        }
        Ok(ToolResponse::Text(full_text))
    }
}
