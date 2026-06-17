use anyhow::{anyhow, Context, Result};
use serde_json::json;

use super::openai::OpenAIResponse;
use super::tools::{ToolResponse, ToolSpec};
use super::STREAM_CANCELLED;
use std::sync::atomic::Ordering;

impl super::Provider {
    fn openrouter_headers(&self) -> Vec<(&str, String)> {
        vec![
            ("Content-Type", "application/json".to_string()),
            (
                "Authorization",
                format!("Bearer {}", self.api_key.as_deref().unwrap_or("")),
            ),
        ]
    }

    pub(super) async fn list_models_openrouter(&self) -> Result<Vec<String>> {
        let url = self.base_url.trim_end_matches('/').to_string() + "/models";
        let mut req = self.client.get(&url);
        for (k, v) in self.openrouter_headers() {
            req = req.header(k, v);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("OpenRouter API unreachable at {}", self.base_url));
        }
        let parsed: super::openai::OpenAIModelsResponse = resp
            .json()
            .await
            .context("invalid OpenRouter models response")?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }

    pub(super) async fn healthcheck_openrouter(&self) -> Result<()> {
        let _models = self.list_models_openrouter().await?;
        Ok(())
    }

    pub(super) async fn complete_json_openrouter(
        &self,
        user_prompt: &str,
    ) -> Result<crate::ModelReply> {
        let url = self.base_url.trim_end_matches('/').to_string() + "/chat/completions";
        let payload = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": self.system_prompt},
                {"role": "user", "content": format!("{}\n\nReturn JSON only.", user_prompt)}
            ],
            "temperature": 0.3,
            "max_tokens": 512,
            "response_format": {"type": "json_object"}
        });
        let mut req = self.client.post(&url);
        for (k, v) in self.openrouter_headers() {
            req = req.header(k, v);
        }
        let resp = req
            .json(&payload)
            .send()
            .await
            .context("failed to call OpenRouter API")?;
        if !resp.status().is_success() {
            return Err(self.parse_api_error("OpenRouter", resp).await);
        }
        let parsed: OpenAIResponse = resp.json().await.context("invalid OpenRouter response")?;
        let content = parsed
            .choices
            .first()
            .map(|c| c.message.content.as_str())
            .unwrap_or("");
        Self::parse_json_fallback(content)
    }

    pub(super) async fn complete_chat_stream_openrouter(
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
            "max_tokens": 4096,
        });
        let mut req = self.client.post(&url);
        for (k, v) in self.openrouter_headers() {
            req = req.header(k, v);
        }
        let resp = req
            .json(&payload)
            .send()
            .await
            .context("failed to call OpenRouter API")?;
        if !resp.status().is_success() {
            return Err(self.parse_api_error("OpenRouter", resp).await);
        }
        self.stream_sse_response(resp).await
    }

    pub(super) async fn complete_chat_vision_openrouter(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        mime_type: &str,
        base64_data: &str,
    ) -> Result<String> {
        let url = self.base_url.trim_end_matches('/').to_string() + "/chat/completions";
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
        let payload = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            "max_tokens": 4096
        });
        let mut req = self.client.post(&url);
        for (k, v) in self.openrouter_headers() {
            req = req.header(k, v);
        }
        let resp = req
            .json(&payload)
            .send()
            .await
            .context("failed to call OpenRouter vision API")?;
        if !resp.status().is_success() {
            return Err(self.parse_api_error("OpenRouter", resp).await);
        }
        self.stream_sse_response(resp).await
    }

    pub(super) async fn complete_chat_stream_tools_openrouter(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        tool_specs: &[ToolSpec],
    ) -> Result<ToolResponse> {
        let url = self.base_url.trim_end_matches('/').to_string() + "/chat/completions";
        let mut messages: Vec<serde_json::Value> = vec![];
        messages.push(json!({"role": "system", "content": system_prompt}));
        if !history.is_empty() {
            messages.push(json!({"role": "user", "content": history}));
        }
        messages.push(json!({"role": "user", "content": user_prompt}));

        let tools: Vec<serde_json::Value> = tool_specs.iter().map(|t| t.to_openai_tool()).collect();
        let mut payload = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            "temperature": 0.7,
            "max_tokens": 4096,
        });
        if !tools.is_empty() {
            payload["tools"] = json!(tools);
            payload["tool_choice"] = json!("auto");
        }

        let mut req = self.client.post(&url);
        for (k, v) in self.openrouter_headers() {
            req = req.header(k, v);
        }
        let resp = req
            .json(&payload)
            .send()
            .await
            .context("failed to call OpenRouter API")?;
        if !resp.status().is_success() {
            return Err(self.parse_api_error("OpenRouter", resp).await);
        }

        let mut full_text = String::with_capacity(4096);
        let mut tool_acc = super::openai::AccumulatedToolCalls::default();
        let mut stream = resp.bytes_stream();
        let mut buf = String::with_capacity(4096);
        let mut cursor = 0usize;

        loop {
            if STREAM_CANCELLED.load(Ordering::SeqCst) {
                break;
            }
            use futures_util::StreamExt;
            let chunk =
                match tokio::time::timeout(std::time::Duration::from_secs(60), stream.next()).await
                {
                    Ok(Some(Ok(c))) => c,
                    Ok(Some(Err(e))) => return Err(anyhow!("stream read error: {}", e)),
                    Ok(None) => break,
                    Err(_) => return Err(anyhow!("stream timed out")),
                };
            buf.push_str(&String::from_utf8_lossy(&chunk));
            loop {
                let tail = &buf[cursor..];
                match tail.find('\n') {
                    Some(pos) => {
                        let line = &tail[..pos];
                        cursor += pos + 1;
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if !trimmed.starts_with("data: ") {
                            continue;
                        }
                        let data = &trimmed[6..];
                        if data == "[DONE]" {
                            break;
                        }
                        if let Ok(chunk) =
                            serde_json::from_str::<super::openai::OpenAIStreamChunk>(data)
                        {
                            if let Some(content) = chunk
                                .choices
                                .first()
                                .and_then(|c| c.delta.content.as_deref())
                            {
                                full_text.push_str(content);
                            }
                            if let Some(tool_calls) = chunk
                                .choices
                                .first()
                                .and_then(|c| c.delta.tool_calls.as_ref())
                            {
                                tool_acc.absorb_chunk(tool_calls);
                            }
                        }
                    }
                    None => break,
                }
            }
            if full_text.len() > super::MAX_RESPONSE_BYTES {
                return Err(anyhow!(
                    "response too large ({} bytes)",
                    super::MAX_RESPONSE_BYTES
                ));
            }
        }

        if !tool_acc.is_empty() {
            Ok(ToolResponse::ToolCalls(tool_acc.to_tool_calls()))
        } else {
            Ok(ToolResponse::Text(full_text))
        }
    }
}
