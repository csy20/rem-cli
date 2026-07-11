use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use super::tools::{ToolCall, ToolResponse, ToolSpec};
use super::{ProviderBackend, ProviderContext};

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
    #[serde(default)]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiModelEntry {
    pub name: String,
}

pub(super) struct GeminiBackend;

impl GeminiBackend {
    fn gemini_url(&self, ctx: &ProviderContext, path: &str) -> String {
        format!("{}/v1beta{}", ctx.base_url.trim_end_matches('/'), path)
    }
}

#[async_trait]
impl ProviderBackend for GeminiBackend {
    async fn list_models(&self, ctx: &ProviderContext) -> Result<Vec<String>> {
        let url = self.gemini_url(ctx, "/models");
        let mut all_models = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut req = ctx.client.get(&url).header("x-goog-api-key", ctx.api_key_str());
            if let Some(ref token) = page_token {
                req = req.query(&[("pageToken", token)]);
            }
            let resp = req.send().await?;
            if !resp.status().is_success() {
                return Err(anyhow!(super::ProviderError::Other("Gemini API unreachable".into())));
            }
            let parsed: GeminiModelsResponse = resp.json().await.context("invalid Gemini response")?;
            if let Some(models) = parsed.models {
                for m in models {
                    let name = m.name.strip_prefix("models/").unwrap_or(&m.name).to_string();
                    if name.contains("gemini") {
                        all_models.push(name);
                    }
                }
            }
            match parsed.next_page_token {
                Some(token) if !token.is_empty() => page_token = Some(token),
                _ => break,
            }
        }
        Ok(all_models)
    }

    async fn complete_json(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<crate::ModelReply> {
        let url = self.gemini_url(ctx, &format!("/models/{}:generateContent", ctx.model));
        let payload = json!({
            "contents": [{"parts": [{"text": format!("{}\n\nUser request:\n{}\n\nReturn JSON only.", system_prompt, user_prompt)}]}],
            "generationConfig": {
                "temperature": crate::constants::JSON_TEMPERATURE,
                "maxOutputTokens": crate::constants::JSON_MAX_TOKENS
            }
        });

        let resp = ctx
            .client
            .post(&url)
            .header("x-goog-api-key", ctx.api_key_str())
            .json(&payload)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(super::parse_api_error("Gemini", resp, None).await);
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

        super::parse_json_fallback(&text)
    }

    async fn complete_chat_stream(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let url = self.gemini_url(ctx, &format!("/models/{}:streamGenerateContent?alt=sse", ctx.model));

        let mut contents = vec![];
        if !history.is_empty() {
            for (user_msg, assistant_msg) in super::parse_history_turns(history) {
                contents.push(json!({"role": "user", "parts": [{"text": user_msg}]}));
                if !assistant_msg.is_empty() {
                    contents.push(json!({"role": "model", "parts": [{"text": assistant_msg}]}));
                }
            }
        }
        contents.push(json!({"role": "user", "parts": [{"text": user_prompt}]}));

        let mut payload = json!({
            "contents": contents,
            "generationConfig": {
                "temperature": crate::constants::DEFAULT_TEMPERATURE,
                "maxOutputTokens": crate::constants::DEFAULT_MAX_TOKENS
            }
        });

        if !system_prompt.is_empty() {
            payload["systemInstruction"] = json!({"parts": [{"text": system_prompt}]});
        }

        let resp = ctx
            .client
            .post(&url)
            .header("x-goog-api-key", ctx.api_key_str())
            .json(&payload)
            .send()
            .await
            .context("failed to call Gemini API")?;

        if !resp.status().is_success() {
            return Err(super::parse_api_error("Gemini", resp, None).await);
        }

        super::stream_gemini_sse(resp).await
    }

    async fn complete_chat_stream_with_vision(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
        mime_type: &str,
        base64_data: &str,
    ) -> Result<String> {
        let url = self.gemini_url(ctx, &format!("/models/{}:streamGenerateContent?alt=sse", ctx.model));

        let mut parts: Vec<serde_json::Value> = vec![];
        parts.push(json!({"text": user_prompt}));
        parts.push(json!({
            "inline_data": {
                "mime_type": mime_type,
                "data": base64_data
            }
        }));

        let mut contents: Vec<serde_json::Value> = vec![];
        if !history.is_empty() {
            for (user_msg, assistant_msg) in super::parse_history_turns(history) {
                contents.push(json!({"role": "user", "parts": [{"text": user_msg}]}));
                if !assistant_msg.is_empty() {
                    contents.push(json!({"role": "model", "parts": [{"text": assistant_msg}]}));
                }
            }
        }
        contents.push(json!({"role": "user", "parts": parts}));

        let mut payload = json!({
            "contents": contents,
            "generationConfig": {
                "temperature": crate::constants::DEFAULT_TEMPERATURE,
                "maxOutputTokens": crate::constants::DEFAULT_MAX_TOKENS
            }
        });

        if !system_prompt.is_empty() {
            payload["systemInstruction"] = json!({"parts": [{"text": system_prompt}]});
        }

        let resp = ctx
            .client
            .post(&url)
            .header("x-goog-api-key", ctx.api_key_str())
            .json(&payload)
            .send()
            .await
            .context("failed to call Gemini vision API")?;

        if !resp.status().is_success() {
            return Err(super::parse_api_error("Gemini", resp, None).await);
        }

        super::stream_gemini_sse(resp).await
    }

    async fn complete_chat_stream_with_tools(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
        tool_specs: &[ToolSpec],
    ) -> Result<ToolResponse> {
        let url = self.gemini_url(ctx, &format!("/models/{}:streamGenerateContent?alt=sse", ctx.model));

        let mut contents = vec![];
        if !history.is_empty() {
            for (user_msg, assistant_msg) in super::parse_history_turns(history) {
                contents.push(json!({"role": "user", "parts": [{"text": user_msg}]}));
                if !assistant_msg.is_empty() {
                    contents.push(json!({"role": "model", "parts": [{"text": assistant_msg}]}));
                }
            }
        }
        contents.push(json!({"role": "user", "parts": [{"text": user_prompt}]}));

        let mut payload = json!({
            "contents": contents,
            "generationConfig": {
                "temperature": crate::constants::DEFAULT_TEMPERATURE,
                "maxOutputTokens": crate::constants::DEFAULT_MAX_TOKENS
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

        let resp = ctx
            .client
            .post(&url)
            .header("x-goog-api-key", ctx.api_key_str())
            .json(&payload)
            .send()
            .await
            .context("failed to call Gemini API")?;

        if !resp.status().is_success() {
            return Err(super::parse_api_error("Gemini", resp, None).await);
        }

        let mut full_text = String::with_capacity(crate::constants::INITIAL_BUF_CAPACITY);
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        super::stream_buf(resp, |trimmed| {
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
                                            super::emit_token(&text);
                                        }
                                        if let Some(fc) = part.function_call {
                                            let name = fc.name.unwrap_or_default();
                                            let args = fc.args.unwrap_or(serde_json::Value::Null);
                                            if !name.is_empty() {
                                                tool_calls.push(ToolCall {
                                                    id: format!("fc_{}", name),
                                                    name,
                                                    arguments: args,
                                                });
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
                return Err(anyhow!(super::ProviderError::ResponseTooLarge(
                    super::MAX_RESPONSE_BYTES as u64
                )));
            }
            Ok(true)
        })
        .await?;

        if !tool_calls.is_empty() {
            return Ok(ToolResponse::ToolCalls(tool_calls));
        }
        Ok(ToolResponse::Text(full_text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_url_basic() {
        let backend = GeminiBackend;
        let provider = super::super::Provider::new(
            super::super::ProviderKind::Gemini,
            "https://generativelanguage.googleapis.com".into(),
            "gemini-2.0-flash".into(),
            30,
            "system".into(),
            Some("key".into()),
            4096,
        );
        let url = backend.gemini_url(&provider.ctx, "/models");
        assert_eq!(url, "https://generativelanguage.googleapis.com/v1beta/models");
    }

    #[test]
    fn gemini_url_stream() {
        let backend = GeminiBackend;
        let provider = super::super::Provider::new(
            super::super::ProviderKind::Gemini,
            "https://generativelanguage.googleapis.com/".into(),
            "gemini-2.0-flash".into(),
            30,
            "".into(),
            None,
            4096,
        );
        let url = backend.gemini_url(&provider.ctx, "/models/gemini-pro:streamGenerateContent?alt=sse");
        assert!(url.contains("v1beta/models/gemini-pro:streamGenerateContent"));
    }

    #[test]
    fn gemini_response_deserialize() {
        let json = r#"{"candidates":[{"content":{"parts":[{"text":"hello"}]}}]}"#;
        let resp: GeminiResponse = serde_json::from_str(json).unwrap();
        let text = resp
            .candidates
            .and_then(|c| c.into_iter().next())
            .and_then(|c| c.content)
            .and_then(|c| c.parts)
            .and_then(|p| p.into_iter().next())
            .and_then(|p| p.text);
        assert_eq!(text.as_deref(), Some("hello"));
    }

    #[test]
    fn gemini_response_empty_candidates() {
        let json = r#"{}"#;
        let resp: GeminiResponse = serde_json::from_str(json).unwrap();
        assert!(resp.candidates.is_none());
    }

    #[test]
    fn gemini_stream_chunk_deserialize() {
        let json = r#"{"candidates":[{"content":{"parts":[{"text":"world"}]}}]}"#;
        let chunk: GeminiStreamChunk = serde_json::from_str(json).unwrap();
        let text = chunk
            .candidates
            .and_then(|c| c.into_iter().next())
            .and_then(|c| c.content)
            .and_then(|c| c.parts)
            .and_then(|p| p.into_iter().next())
            .and_then(|p| p.text);
        assert_eq!(text.as_deref(), Some("world"));
    }

    #[test]
    fn gemini_models_response_deserialize() {
        let json = r#"{"models":[{"name":"models/gemini-2.0-flash"},{"name":"models/gemini-pro"}]}"#;
        let resp: GeminiModelsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.models.as_ref().map(|m| m.len()), Some(2));
    }

    #[test]
    fn gemini_part_with_function_call() {
        let json = r#"{"text":null,"function_call":{"name":"read_file","args":{"path":"test.txt"}}}"#;
        let part: GeminiPart = serde_json::from_str(json).unwrap();
        assert!(part.function_call.is_some());
        let fc = part.function_call.unwrap();
        assert_eq!(fc.name.as_deref(), Some("read_file"));
        assert_eq!(
            fc.args.as_ref().and_then(|a| a.get("path")).and_then(|v| v.as_str()),
            Some("test.txt")
        );
    }

    #[test]
    fn gemini_candidate_no_content() {
        let json = r#"{"candidates":[{}]}"#;
        let resp: GeminiResponse = serde_json::from_str(json).unwrap();
        assert!(resp.candidates.unwrap()[0].content.is_none());
    }
}
