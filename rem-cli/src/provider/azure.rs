use anyhow::{anyhow, Context, Result};
use serde_json::json;

use super::openai::OpenAIResponse;
use super::tools::{ToolResponse, ToolSpec};

impl super::Provider {
    pub(super) async fn list_models_azure(&self) -> Result<Vec<String>> {
        let url = self.openai_models_url();
        let resp = self.add_openai_auth(self.client.get(&url)).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("Azure OpenAI API unreachable at {}", self.base_url));
        }
        Ok(vec![self.model.clone()])
    }

    pub(super) async fn complete_json_azure(&self, user_prompt: &str) -> Result<crate::ModelReply> {
        let url = self.openai_chat_url();
        let resp = self
            .add_openai_auth(self.client.post(&url))
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
        let url = self.openai_chat_url();
        let mut messages: Vec<serde_json::Value> = vec![];
        messages.push(json!({"role": "system", "content": system_prompt}));
        if !history.is_empty() {
            messages.push(json!({"role": "user", "content": history}));
        }
        messages.push(json!({"role": "user", "content": user_prompt}));

        let resp = self
            .add_openai_auth(self.client.post(&url))
            .json(&json!({
                "model": self.model,
                "messages": messages,
                "stream": true,
                "temperature": 0.7,
                "max_tokens": 4096,
            }))
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
        let url = self.openai_chat_url();
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
        let resp = self
            .add_openai_auth(self.client.post(&url))
            .json(&json!({
                "model": self.model,
                "messages": messages,
                "stream": true,
                "max_tokens": 4096
            }))
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
        let url = self.openai_chat_url();
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

        let resp = self
            .add_openai_auth(self.client.post(&url))
            .json(&payload)
            .send()
            .await
            .context("failed to call Azure OpenAI API")?;
        if !resp.status().is_success() {
            return Err(self.parse_api_error("Azure OpenAI", resp).await);
        }

        Self::stream_openai_tool_response(resp).await
    }
}
