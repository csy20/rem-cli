use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use super::tools::{ToolResponse, ToolSpec};
use super::{ProviderBackend, ProviderContext};

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
    pub content: Option<String>,
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
    #[serde(default)]
    pub tool_calls: Option<Vec<OpenAIStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIStreamToolCall {
    pub index: i64,
    #[serde(default)]
    pub id: Option<String>,
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
    async fn list_models(&self, ctx: &ProviderContext) -> Result<Vec<String>> {
        let url = super::openai_models_url(&ctx.base_url);
        let resp = super::add_openai_auth(ctx.client.get(&url), ctx.api_key_str(), ctx.kind)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(anyhow!("OpenAI API unreachable at {}", ctx.base_url));
        }
        let parsed: OpenAIModelsResponse = resp.json().await.context("invalid models response")?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }

    async fn complete_json(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<crate::ModelReply> {
        super::openai_compat_complete_json(ctx, ctx.kind, "OpenAI", system_prompt, user_prompt).await
    }

    async fn complete_chat_stream(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let url = super::openai_chat_url(&ctx.base_url, ctx.kind, &ctx.model);

        let is_reasoning = crate::reasoning::is_reasoning_model(&ctx.model);
        let no_system = crate::reasoning::system_prompt_not_supported(&ctx.model);
        let no_stream = crate::reasoning::requires_non_streaming(&ctx.model);
        let lower = ctx.model.to_lowercase();

        let mut messages: Vec<serde_json::Value> = vec![];
        if !history.is_empty() {
            for (user_msg, assistant_msg) in super::parse_history_turns(history) {
                messages.push(json!({"role": "user", "content": user_msg}));
                if !assistant_msg.is_empty() {
                    messages.push(json!({"role": "assistant", "content": assistant_msg}));
                }
            }
        }
        if no_system {
            let combined = format!("{system_prompt}\n\n{user_prompt}");
            messages.push(json!({"role": "user", "content": combined}));
        } else {
            messages.push(json!({"role": "system", "content": system_prompt}));
            messages.push(json!({"role": "user", "content": user_prompt}));
        }

        let mut payload = serde_json::Map::new();
        payload.insert("model".into(), json!(&ctx.model));
        payload.insert("messages".into(), json!(messages));
        payload.insert("max_tokens".into(), json!(crate::constants::DEFAULT_MAX_TOKENS));

        if is_reasoning && ctx.reasoning_config.enabled && (lower.starts_with("o1-") || lower.starts_with("o3-")) {
            let effort = ctx.reasoning_config.effort.as_str();
            payload.insert("reasoning_effort".into(), json!(effort));
        } else {
            payload.insert("temperature".into(), json!(crate::constants::DEFAULT_TEMPERATURE));
        }

        if !no_stream {
            payload.insert("stream".into(), json!(true));
        }

        if no_stream {
            let resp = super::add_openai_auth(ctx.client.post(&url), ctx.api_key_str(), ctx.kind)
                .json(&payload)
                .send()
                .await
                .context("failed to call OpenAI API")?;
            if !resp.status().is_success() {
                return Err(super::parse_api_error("OpenAI", resp).await);
            }
            let parsed: OpenAIResponse = resp.json().await.context("invalid OpenAI response")?;
            let content = parsed
                .choices
                .first()
                .and_then(|c| c.message.content.as_deref())
                .unwrap_or("");
            return Ok(content.to_string());
        }

        let resp = super::add_openai_auth(ctx.client.post(&url), ctx.api_key_str(), ctx.kind)
            .json(&payload)
            .send()
            .await
            .context("failed to call OpenAI API")?;

        if !resp.status().is_success() {
            return Err(super::parse_api_error("OpenAI", resp).await);
        }

        super::stream_sse_response(resp).await
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
            "OpenAI",
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
        super::openai_compat_chat_stream_with_tools(
            ctx,
            ctx.kind,
            "OpenAI",
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

    fn make_tool_call(index: i64, id: &str, name: &str, args: &str) -> OpenAIStreamToolCall {
        OpenAIStreamToolCall {
            index,
            id: if id.is_empty() { None } else { Some(id.to_string()) },
            function: Some(OpenAIStreamToolCallFunction {
                name: if name.is_empty() { None } else { Some(name.to_string()) },
                arguments: if args.is_empty() { None } else { Some(args.to_string()) },
            }),
        }
    }

    #[test]
    fn accumulated_tool_calls_empty_initially() {
        let acc = AccumulatedToolCalls::default();
        assert!(acc.is_empty());
        assert!(acc.calls.is_empty());
    }

    #[test]
    fn accumulated_tool_calls_absorb_single() {
        let mut acc = AccumulatedToolCalls::default();
        let calls = vec![make_tool_call(0, "call_1", "read_file", r#"{"path":"x"}"#)];
        acc.absorb_chunk(&calls);
        assert!(!acc.is_empty());
        assert_eq!(acc.calls.len(), 1);
        assert_eq!(acc.calls[0].id, "call_1");
        assert_eq!(acc.calls[0].name, "read_file");
        assert_eq!(acc.calls[0].arguments, r#"{"path":"x"}"#);
    }

    #[test]
    fn accumulated_tool_calls_absorb_multiple_indices() {
        let mut acc = AccumulatedToolCalls::default();
        let calls = vec![
            make_tool_call(0, "call_0", "read_file", r#"{"path":"a"}"#),
            make_tool_call(2, "call_2", "write_file", r#"{"path":"b"}"#),
        ];
        acc.absorb_chunk(&calls);
        assert_eq!(acc.calls.len(), 3);
        assert_eq!(acc.calls[0].id, "call_0");
        assert!(acc.calls[1].id.is_empty());
        assert_eq!(acc.calls[2].id, "call_2");
    }

    #[test]
    fn accumulated_tool_calls_absorb_chunked_arguments() {
        let mut acc = AccumulatedToolCalls::default();
        acc.absorb_chunk(&[make_tool_call(0, "call_1", "read_file", r##"{"path":""##)]);
        acc.absorb_chunk(&[make_tool_call(0, "", "", r#"test.txt"}"#)]);
        assert_eq!(acc.calls[0].arguments, r#"{"path":"test.txt"}"#);
    }

    #[test]
    fn accumulated_tool_calls_to_tool_calls() {
        let mut acc = AccumulatedToolCalls::default();
        acc.absorb_chunk(&[make_tool_call(0, "id1", "search", r#"{"q":"hello"}"#)]);
        let tcs = acc.to_tool_calls();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].id, "id1");
        assert_eq!(tcs[0].name, "search");
        assert_eq!(tcs[0].arguments["q"], "hello");
    }

    #[test]
    fn accumulated_tool_calls_to_tool_calls_empty() {
        let acc = AccumulatedToolCalls::default();
        assert!(acc.to_tool_calls().is_empty());
    }

    #[test]
    fn accumulated_tool_calls_id_only_set_once() {
        let mut acc = AccumulatedToolCalls::default();
        acc.absorb_chunk(&[make_tool_call(0, "first_id", "tool_a", "")]);
        acc.absorb_chunk(&[make_tool_call(0, "second_id", "tool_b", "")]);
        assert_eq!(acc.calls[0].id, "first_id");
        assert_eq!(acc.calls[0].name, "tool_a");
    }

    #[test]
    fn openai_stream_chunk_deserialize() {
        let json = r#"{"choices":[{"delta":{"content":"hello"}}]}"#;
        let chunk: OpenAIStreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.choices.len(), 1);
        assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hello"));
    }

    #[test]
    fn openai_response_deserialize() {
        let json = r#"{"choices":[{"message":{"content":"hi"}}]}"#;
        let resp: OpenAIResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.choices[0].message.content.as_deref(), Some("hi"));
    }

    #[test]
    fn openai_models_response_deserialize() {
        let json = r#"{"data":[{"id":"gpt-4"},{"id":"gpt-3.5"}]}"#;
        let resp: OpenAIModelsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data.len(), 2);
        assert_eq!(resp.data[0].id, "gpt-4");
    }

    #[test]
    fn openai_stream_chunk_with_tool_calls() {
        let json = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":"{\"path\":\"x\"}"}}]}}]}"#;
        let chunk: OpenAIStreamChunk = serde_json::from_str(json).unwrap();
        let tcs = chunk.choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].id.as_deref(), Some("call_1"));
    }

    #[test]
    fn openai_stream_delta_default_tool_calls() {
        let json = r#"{"choices":[{"delta":{"content":"hello"}}]}"#;
        let chunk: OpenAIStreamChunk = serde_json::from_str(json).unwrap();
        assert!(chunk.choices[0].delta.tool_calls.is_none());
    }
}
