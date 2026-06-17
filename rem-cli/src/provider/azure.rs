use anyhow::{anyhow, Context, Result};
use serde_json::json;

use super::openai::{AccumulatedToolCalls, OpenAIResponse, OpenAIStreamChunk};
use super::tools::{ToolResponse, ToolSpec};
use super::STREAM_CANCELLED;
use std::sync::atomic::Ordering;

const AZURE_API_VERSION: &str = "2024-02-15-preview";

fn azure_url(base_url: &str, endpoint: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let ep = endpoint.trim_start_matches('/');
    if ep.contains('?') {
        format!("{}/{}&api-version={}", base, ep, AZURE_API_VERSION)
    } else {
        format!("{}/{}?api-version={}", base, ep, AZURE_API_VERSION)
    }
}

impl super::Provider {
    fn azure_headers(&self) -> Vec<(&str, String)> {
        let mut headers = vec![("Content-Type", "application/json".to_string())];
        if let Some(ref key) = self.api_key {
            if !key.is_empty() {
                headers.push(("api-key", key.clone()));
            }
        }
        headers
    }

    pub(super) async fn list_models_azure(&self) -> Result<Vec<String>> {
        let url = azure_url(&self.base_url, "/models");
        let mut req = self.client.get(&url);
        for (k, v) in self.azure_headers() {
            req = req.header(k, v);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("Azure OpenAI API unreachable at {}", self.base_url));
        }
        Ok(vec![self.model.clone()])
    }

    pub(super) async fn healthcheck_azure(&self) -> Result<()> {
        if self.api_key.as_deref().is_none_or(|k| k.is_empty()) {
            return Err(anyhow!(
                "Azure OpenAI requires --api-key or AZURE_OPENAI_API_KEY env var"
            ));
        }
        let _ = self.list_models_azure().await?;
        Ok(())
    }

    pub(super) async fn complete_json_azure(&self, user_prompt: &str) -> Result<crate::ModelReply> {
        let url = azure_url(&self.base_url, "/chat/completions");
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
        for (k, v) in self.azure_headers() {
            req = req.header(k, v);
        }
        let resp = req
            .json(&payload)
            .send()
            .await
            .context("failed to call Azure OpenAI API")?;
        if !resp.status().is_success() {
            return Err(self.parse_api_error("Azure OpenAI", resp).await);
        }
        let parsed: OpenAIResponse = resp.json().await.context("invalid Azure OpenAI response")?;
        let content = parsed
            .choices
            .first()
            .map(|c| c.message.content.as_str())
            .unwrap_or("");
        Self::parse_json_fallback(content)
    }

    pub(super) async fn complete_chat_stream_azure(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let url = azure_url(&self.base_url, "/chat/completions");
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
        for (k, v) in self.azure_headers() {
            req = req.header(k, v);
        }
        let resp = req
            .json(&payload)
            .send()
            .await
            .context("failed to call Azure OpenAI API")?;
        if !resp.status().is_success() {
            return Err(self.parse_api_error("Azure OpenAI", resp).await);
        }
        self.stream_sse_response(resp).await
    }

    pub(super) async fn complete_chat_vision_azure(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        mime_type: &str,
        base64_data: &str,
    ) -> Result<String> {
        let url = azure_url(&self.base_url, "/chat/completions");
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
        for (k, v) in self.azure_headers() {
            req = req.header(k, v);
        }
        let resp = req
            .json(&payload)
            .send()
            .await
            .context("failed to call Azure OpenAI vision API")?;
        if !resp.status().is_success() {
            return Err(self.parse_api_error("Azure OpenAI", resp).await);
        }
        self.stream_sse_response(resp).await
    }

    pub(super) async fn complete_chat_stream_tools_azure(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        tool_specs: &[ToolSpec],
    ) -> Result<ToolResponse> {
        let url = azure_url(&self.base_url, "/chat/completions");
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
        for (k, v) in self.azure_headers() {
            req = req.header(k, v);
        }
        let resp = req
            .json(&payload)
            .send()
            .await
            .context("failed to call Azure OpenAI API")?;
        if !resp.status().is_success() {
            return Err(self.parse_api_error("Azure OpenAI", resp).await);
        }

        let mut full_text = String::with_capacity(4096);
        let mut tool_acc = AccumulatedToolCalls::default();
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
                        if let Ok(chunk) = serde_json::from_str::<OpenAIStreamChunk>(data) {
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
