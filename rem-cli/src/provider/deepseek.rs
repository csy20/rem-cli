use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

use super::tools::{ToolResponse, ToolSpec};
use super::{openai, ProviderBackend, ProviderContext};

pub(super) struct DeepSeekBackend;

#[derive(Debug, Deserialize)]
struct DeepSeekStreamChunk {
    pub choices: Option<Vec<DeepSeekChoice>>,
}

#[derive(Debug, Deserialize)]
struct DeepSeekChoice {
    pub delta: DeepSeekDelta,
    #[allow(dead_code)]
    #[serde(rename = "finish_reason")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeepSeekDelta {
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[allow(dead_code)]
    pub tool_calls: Option<Vec<openai::OpenAIStreamToolCall>>,
}

#[async_trait]
impl ProviderBackend for DeepSeekBackend {
    async fn list_models(&self, ctx: &ProviderContext) -> Result<Vec<String>> {
        let url = super::openai_models_url(&ctx.base_url);
        let resp = super::add_openai_auth(ctx.client.get(&url), ctx.api_key_str(), ctx.kind)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(anyhow!("DeepSeek API unreachable at {}", ctx.base_url));
        }
        let parsed: openai::OpenAIModelsResponse = resp.json().await.context("invalid DeepSeek models response")?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }

    async fn complete_json(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<crate::ModelReply> {
        let is_reasoner = ctx.model.contains("deepseek-reasoner");
        if is_reasoner {
            let url = super::openai_chat_url(&ctx.base_url, ctx.kind, &ctx.model);
            let system = if ctx.model.contains("deepseek-reasoner") {
                None
            } else {
                Some(system_prompt)
            };
            let mut messages: Vec<serde_json::Value> = Vec::new();
            if let Some(sp) = system {
                messages.push(serde_json::json!({"role": "system", "content": sp}));
            }
            messages.push(serde_json::json!({"role": "user", "content": format!("Return JSON only. {}", user_prompt)}));
            let payload = serde_json::json!({
                "model": ctx.model,
                "messages": messages,
                "temperature": crate::constants::JSON_TEMPERATURE,
                "max_tokens": crate::constants::JSON_MAX_TOKENS,
            });
            let resp = super::add_openai_auth(ctx.client.post(&url), ctx.api_key_str(), ctx.kind)
                .json(&payload)
                .send()
                .await?;
            if !resp.status().is_success() {
                return Err(anyhow!("DeepSeek API error: {}", resp.status()));
            }
            let text = resp.text().await?;
            return super::parse_json_fallback(&text);
        }
        super::openai_compat_complete_json(ctx, ctx.kind, "DeepSeek", system_prompt, user_prompt).await
    }

    async fn complete_chat_stream(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let is_reasoner = ctx.model.contains("deepseek-reasoner");
        if is_reasoner {
            // deepseek-reasoner: no system prompt, handle reasoning_content
            let url = super::openai_chat_url(&ctx.base_url, ctx.kind, &ctx.model);
            let messages = super::build_messages_from_history(history, user_prompt, None);
            let payload = serde_json::json!({
                "model": ctx.model,
                "messages": messages,
                "stream": true,
                "temperature": crate::constants::DEFAULT_TEMPERATURE,
                "max_tokens": crate::constants::DEFAULT_MAX_TOKENS,
            });
            let resp = super::add_openai_auth(ctx.client.post(&url), ctx.api_key_str(), ctx.kind)
                .json(&payload)
                .send()
                .await?;
            if !resp.status().is_success() {
                return Err(super::parse_api_error("DeepSeek", resp, Some(ctx.api_key_str())).await);
            }
            let mut full_text = String::with_capacity(crate::constants::INITIAL_BUF_CAPACITY);
            super::stream_buf(resp, |trimmed| {
                if !trimmed.starts_with("data: ") {
                    return Ok(true);
                }
                let data = &trimmed[6..];
                if data == "[DONE]" {
                    return Ok(false);
                }
                if let Ok(chunk) = serde_json::from_str::<DeepSeekStreamChunk>(data) {
                    if let Some(choices) = chunk.choices {
                        if let Some(choice) = choices.into_iter().next() {
                            if let Some(content) = choice.delta.content {
                                full_text.push_str(&content);
                                super::emit_token(&content);
                            }
                            if let Some(reasoning) = choice.delta.reasoning_content {
                                if ctx.reasoning_config.show_reasoning {
                                    super::emit_token("[reasoning]");
                                    super::emit_token(&reasoning);
                                    super::emit_token("[/reasoning]");
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
            return Ok(full_text);
        }
        super::openai_compat_chat_stream(ctx, ctx.kind, "DeepSeek", user_prompt, system_prompt, history).await
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
        super::openai_compat_chat_stream_with_vision(
            ctx,
            ctx.kind,
            "DeepSeek",
            user_prompt,
            system_prompt,
            history,
            mime_type,
            base64_data,
        )
        .await
    }

    async fn complete_chat_stream_with_tools(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
        tool_specs: &[ToolSpec],
    ) -> Result<ToolResponse> {
        let is_reasoner = ctx.model.contains("deepseek-reasoner");
        if is_reasoner {
            // deepseek-reasoner does not support tool calls
            let text = self
                .complete_chat_stream(ctx, system_prompt, user_prompt, history)
                .await?;
            return Ok(ToolResponse::Text(text));
        }
        super::openai_compat_chat_stream_with_tools(
            ctx,
            ctx.kind,
            "DeepSeek",
            user_prompt,
            system_prompt,
            history,
            tool_specs,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deepseek_stream_chunk_deserialize() {
        let json = r#"{"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        let chunk: DeepSeekStreamChunk = serde_json::from_str(json).unwrap();
        assert!(chunk.choices.is_some());
        let choice = chunk.choices.unwrap().into_iter().next().unwrap();
        assert_eq!(choice.delta.content.unwrap(), "Hello");
    }

    #[test]
    fn test_deepseek_stream_chunk_with_reasoning() {
        let json =
            r#"{"choices":[{"delta":{"content":"Hello","reasoning_content":"thinking..."},"finish_reason":null}]}"#;
        let chunk: DeepSeekStreamChunk = serde_json::from_str(json).unwrap();
        let choice = chunk.choices.unwrap().into_iter().next().unwrap();
        assert_eq!(choice.delta.content.unwrap(), "Hello");
        assert_eq!(choice.delta.reasoning_content.unwrap(), "thinking...");
    }

    #[test]
    fn test_deepseek_stream_chunk_empty_delta() {
        let json = r#"{"choices":[{"delta":{},"finish_reason":null}]}"#;
        let chunk: DeepSeekStreamChunk = serde_json::from_str(json).unwrap();
        assert!(chunk.choices.is_some());
    }
}
