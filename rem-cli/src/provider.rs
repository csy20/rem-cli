use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

use crate::ui;

// ── Shared response types ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct LlmErrorResponse {
    error: String,
}

// ── Ollama response types ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaTagModel>,
}

#[derive(Debug, Deserialize)]
struct OllamaTagModel {
    name: String,
}

#[derive(Debug, Deserialize)]
pub struct OllamaJsonResponse {
    pub response: String,
}

// ── OpenAI-compatible response types ───────────────────────────────────

#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAIMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamChunk {
    choices: Vec<OpenAIStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamChoice {
    delta: OpenAIStreamDelta,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamDelta {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIModelsResponse {
    data: Vec<OpenAIModelEntry>,
}

#[derive(Debug, Deserialize)]
struct OpenAIModelEntry {
    id: String,
}

// ── Gemini response types ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    content: Option<GeminiContent>,
}

#[derive(Debug, Deserialize)]
struct GeminiContent {
    parts: Option<Vec<GeminiPart>>,
}

#[derive(Debug, Deserialize)]
struct GeminiPart {
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiStreamChunk {
    candidates: Option<Vec<GeminiStreamCandidate>>,
}

#[derive(Debug, Deserialize)]
struct GeminiStreamCandidate {
    content: Option<GeminiStreamContent>,
}

#[derive(Debug, Deserialize)]
struct GeminiStreamContent {
    parts: Option<Vec<GeminiPart>>,
}

#[derive(Debug, Deserialize)]
struct GeminiModelsResponse {
    models: Option<Vec<GeminiModelEntry>>,
}

#[derive(Debug, Deserialize)]
struct GeminiModelEntry {
    name: String,
    #[allow(dead_code)]
    display_name: Option<String>,
}

// ── Anthropic response types ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Option<Vec<AnthropicContentBlock>>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlock {
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicStreamChunk {
    #[serde(rename = "type")]
    chunk_type: Option<String>,
    delta: Option<AnthropicDelta>,
}

#[derive(Debug, Deserialize)]
struct AnthropicDelta {
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModelsResponse {
    data: Option<Vec<AnthropicModelEntry>>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModelEntry {
    #[allow(dead_code)]
    #[serde(rename = "type")]
    _type: Option<String>,
    id: Option<String>,
    #[allow(dead_code)]
    display_name: Option<String>,
}

// ── URL helper ─────────────────────────────────────────────────────────

pub fn api_url(base_url: &str, endpoint: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let ep = endpoint.trim_start_matches('/');
    if base.ends_with("/api") {
        format!("{}/{}", base, ep)
    } else {
        format!("{}/api/{}", base, ep)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderKind {
    Ollama,
    OpenAI,
    Gemini,
    Anthropic,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Ollama => "ollama",
            ProviderKind::OpenAI => "openai",
            ProviderKind::Gemini => "gemini",
            ProviderKind::Anthropic => "anthropic",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "openai" => ProviderKind::OpenAI,
            "gemini" | "google" => ProviderKind::Gemini,
            "anthropic" | "claude" => ProviderKind::Anthropic,
            _ => ProviderKind::Ollama,
        }
    }
}

pub struct Provider {
    pub kind: ProviderKind,
    pub client: Client,
    pub base_url: String,
    pub model: String,
    pub system_prompt: String,
    api_key: Option<String>,
    pub model_ctx: usize,
}

impl Provider {
    pub fn new_ollama(
        base_url: String,
        model: String,
        timeout_s: u64,
        system_prompt: String,
        model_ctx: usize,
    ) -> Self {
        Self {
            kind: ProviderKind::Ollama,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(timeout_s))
                .build()
                .unwrap_or_else(|_| Client::new()),
            base_url,
            model,
            system_prompt,
            api_key: None,
            model_ctx,
        }
    }

    pub fn new_openai(
        base_url: String,
        model: String,
        timeout_s: u64,
        system_prompt: String,
        api_key: String,
        model_ctx: usize,
    ) -> Self {
        Self {
            kind: ProviderKind::OpenAI,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(timeout_s))
                .build()
                .unwrap_or_else(|_| Client::new()),
            base_url,
            model,
            system_prompt,
            api_key: Some(api_key),
            model_ctx,
        }
    }

    pub fn new_gemini(
        api_key: String,
        model: String,
        timeout_s: u64,
        system_prompt: String,
        model_ctx: usize,
    ) -> Self {
        Self {
            kind: ProviderKind::Gemini,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(timeout_s))
                .build()
                .unwrap_or_else(|_| Client::new()),
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            model,
            system_prompt,
            api_key: Some(api_key),
            model_ctx,
        }
    }

    pub fn new_anthropic(
        api_key: String,
        model: String,
        timeout_s: u64,
        system_prompt: String,
        model_ctx: usize,
    ) -> Self {
        Self {
            kind: ProviderKind::Anthropic,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(timeout_s))
                .build()
                .unwrap_or_else(|_| Client::new()),
            base_url: "https://api.anthropic.com".to_string(),
            model,
            system_prompt,
            api_key: Some(api_key),
            model_ctx,
        }
    }

    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    pub fn provider_label(&self) -> String {
        match self.kind {
            ProviderKind::Ollama => format!("ollama/{}", self.model),
            ProviderKind::OpenAI => format!("openai/{}", self.model),
            ProviderKind::Gemini => format!("gemini/{}", self.model),
            ProviderKind::Anthropic => format!("anthropic/{}", self.model),
        }
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        match self.kind {
            ProviderKind::Ollama => self.list_models_ollama().await,
            ProviderKind::OpenAI => self.list_models_openai().await,
            ProviderKind::Gemini => self.list_models_gemini().await,
            ProviderKind::Anthropic => self.list_models_anthropic().await,
        }
    }

    pub async fn healthcheck(&self) -> Result<()> {
        match self.kind {
            ProviderKind::Ollama => self.healthcheck_ollama().await,
            ProviderKind::OpenAI => self.healthcheck_openai().await,
            ProviderKind::Gemini => self.healthcheck_gemini().await,
            ProviderKind::Anthropic => self.healthcheck_anthropic().await,
        }
    }

    pub async fn complete_json(&self, user_prompt: &str) -> Result<crate::ModelReply> {
        match self.kind {
            ProviderKind::Ollama => self.complete_json_ollama(user_prompt).await,
            ProviderKind::OpenAI => self.complete_json_openai(user_prompt).await,
            ProviderKind::Gemini => self.complete_json_gemini(user_prompt).await,
            ProviderKind::Anthropic => self.complete_json_anthropic(user_prompt).await,
        }
    }

    pub async fn complete_chat_stream(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        match self.kind {
            ProviderKind::Ollama => {
                self.complete_chat_stream_ollama(user_prompt, system_prompt, history)
                    .await
            }
            ProviderKind::OpenAI => {
                self.complete_chat_stream_openai(user_prompt, system_prompt, history)
                    .await
            }
            ProviderKind::Gemini => {
                self.complete_chat_stream_gemini(user_prompt, system_prompt, history)
                    .await
            }
            ProviderKind::Anthropic => {
                self.complete_chat_stream_anthropic(user_prompt, system_prompt, history)
                    .await
            }
        }
    }

    // ── Ollama ───────────────────────────────────────────────────────

    async fn list_models_ollama(&self) -> Result<Vec<String>> {
        let url = api_url(&self.base_url, "tags");
        let resp = self.client.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("Ollama unreachable at {}", self.base_url));
        }
        let parsed: OllamaTagsResponse = resp.json().await.context("invalid tags response")?;
        Ok(parsed.models.into_iter().map(|m| m.name).collect())
    }

    async fn healthcheck_ollama(&self) -> Result<()> {
        let models = self.list_models_ollama().await?;
        if models.is_empty() {
            return Err(anyhow!(
                "Ollama reachable but no models are installed. Pull one with `ollama pull rem-coder:latest`"
            ));
        }
        Ok(())
    }

    async fn complete_json_ollama(&self, user_prompt: &str) -> Result<crate::ModelReply> {
        let url = api_url(&self.base_url, "generate");
        let final_prompt = format!(
            "{}\n\nUser request:\n{}\n\nReturn JSON only.",
            self.system_prompt, user_prompt
        );
        let payload = json!({
            "model": self.model,
            "prompt": final_prompt,
            "stream": false,
            "options": { "num_predict": 512, "num_ctx": self.model_ctx, "num_thread": 4 },
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
            let err_msg = serde_json::from_str::<LlmErrorResponse>(&body)
                .map(|v| v.error)
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
        match serde_json::from_str::<crate::ModelReply>(raw.response.trim()) {
            Ok(parsed) => Ok(parsed),
            Err(e) => {
                eprintln!(
                    "  {} JSON parse: {} — falling back",
                    ui::theme::paint_warning(&ui::theme::active(), "!"),
                    e
                );
                Ok(crate::ModelReply::fallback(raw.response.trim()))
            }
        }
    }

    async fn complete_chat_stream_ollama(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let url = api_url(&self.base_url, "generate");
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
            "options": { "num_predict": 512, "num_ctx": self.model_ctx, "num_thread": 4 }
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

    async fn handle_ollama_error(&self, resp: reqwest::Response, url: &str) -> Result<String> {
        let status = resp.status();
        let body = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("(read error: {})", e));
        let err_msg = serde_json::from_str::<LlmErrorResponse>(&body)
            .map(|v| v.error)
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
        Err(anyhow!("Ollama failed: {} — {}", status, err_msg))
    }

    async fn stream_ollama_response(&self, resp: reqwest::Response) -> Result<String> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_clone = cancelled.clone();
        let ctrlc_task = tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            cancelled_clone.store(true, Ordering::SeqCst);
        });

        let mut stream = resp.bytes_stream();
        let mut full = String::new();
        let mut buf = String::new();

        while let Some(chunk) = stream.next().await {
            if cancelled.load(Ordering::SeqCst) {
                ctrlc_task.abort();
                break;
            }
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    ctrlc_task.abort();
                    return Err(anyhow!("stream read error: {}", e));
                }
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));
            if buf.len() > 32_000 {
                ctrlc_task.abort();
                return Err(anyhow!("response too large ({} bytes buffered)", buf.len()));
            }
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].to_string();
                buf = buf[pos + 1..].to_string();
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(obj) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    if let Some(token) = obj["response"].as_str() {
                        full.push_str(token);
                    }
                    if obj["done"].as_bool() == Some(true) {
                        if cancelled.load(Ordering::SeqCst) {
                            ctrlc_task.abort();
                            break;
                        }
                        ctrlc_task.abort();
                        return Ok(full);
                    }
                }
            }
        }
        ctrlc_task.abort();
        Ok(full)
    }

    // ── OpenAI-compatible (OpenAI, DeepSeek, etc.) ───────────────────

    async fn list_models_openai(&self) -> Result<Vec<String>> {
        let url = self.base_url.trim_end_matches('/').to_string() + "/models";
        let resp = self
            .client
            .get(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.as_deref().unwrap_or("")),
            )
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(anyhow!("OpenAI API unreachable at {}", self.base_url));
        }
        let parsed: OpenAIModelsResponse = resp.json().await.context("invalid models response")?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }

    async fn healthcheck_openai(&self) -> Result<()> {
        let _models = self.list_models_openai().await?;
        Ok(())
    }

    async fn complete_json_openai(&self, user_prompt: &str) -> Result<crate::ModelReply> {
        let url = self.base_url.trim_end_matches('/').to_string() + "/chat/completions";
        let resp = self
            .client
            .post(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.as_deref().unwrap_or("")),
            )
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
            .context("failed to call OpenAI API")?;

        if !resp.status().is_success() {
            return Err(self.parse_api_error("OpenAI", resp).await);
        }

        let parsed: OpenAIResponse = resp.json().await.context("invalid OpenAI response")?;
        let content = parsed
            .choices
            .first()
            .map(|c| c.message.content.as_str())
            .unwrap_or("");

        match serde_json::from_str::<crate::ModelReply>(content.trim()) {
            Ok(parsed) => Ok(parsed),
            Err(e) => {
                eprintln!(
                    "  {} JSON parse: {} — falling back",
                    ui::theme::paint_warning(&ui::theme::active(), "!"),
                    e
                );
                Ok(crate::ModelReply::fallback(content.trim()))
            }
        }
    }

    async fn complete_chat_stream_openai(
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
            "max_tokens": 4096
        });

        let resp = self
            .client
            .post(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.as_deref().unwrap_or("")),
            )
            .json(&payload)
            .send()
            .await
            .context("failed to call OpenAI API")?;

        if !resp.status().is_success() {
            return Err(self.parse_api_error("OpenAI", resp).await);
        }

        self.stream_sse_response(resp).await
    }

    // ── Gemini ──────────────────────────────────────────────────────

    async fn list_models_gemini(&self) -> Result<Vec<String>> {
        let key = self.api_key.as_deref().unwrap_or("");
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models?key={}",
            key
        );
        let resp = self.client.get(&url).send().await?;
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

    async fn healthcheck_gemini(&self) -> Result<()> {
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

    async fn complete_json_gemini(&self, user_prompt: &str) -> Result<crate::ModelReply> {
        let key = self.api_key.as_deref().unwrap_or("");
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, key
        );
        let payload = json!({
            "contents": [{"parts": [{"text": format!("{}\n\nUser request:\n{}\n\nReturn JSON only.", self.system_prompt, user_prompt)}]}],
            "generationConfig": {
                "temperature": 0.3,
                "maxOutputTokens": 512
            }
        });

        let resp = self.client.post(&url).json(&payload).send().await?;
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

        match serde_json::from_str::<crate::ModelReply>(text.trim()) {
            Ok(parsed) => Ok(parsed),
            Err(e) => {
                eprintln!(
                    "  {} JSON parse: {} — falling back",
                    ui::theme::paint_warning(&ui::theme::active(), "!"),
                    e
                );
                Ok(crate::ModelReply::fallback(text.trim()))
            }
        }
    }

    async fn complete_chat_stream_gemini(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let key = self.api_key.as_deref().unwrap_or("");
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
            self.model, key
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

        let resp = self
            .client
            .post(&url)
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

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("stream read error")?;
            buf.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].to_string();
                buf = buf[pos + 1..].to_string();
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
        }

        Ok(full)
    }

    // ── Anthropic ────────────────────────────────────────────────────

    async fn list_models_anthropic(&self) -> Result<Vec<String>> {
        let key = self.api_key.as_deref().unwrap_or("");
        let url = "https://api.anthropic.com/v1/models";
        let resp = self
            .client
            .get(url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(anyhow!("Anthropic API unreachable"));
        }
        let parsed: AnthropicModelsResponse =
            resp.json().await.context("invalid Anthropic response")?;
        Ok(parsed
            .data
            .unwrap_or_default()
            .into_iter()
            .filter_map(|m| m.id)
            .collect())
    }

    async fn healthcheck_anthropic(&self) -> Result<()> {
        if self.api_key.as_deref().unwrap_or("").is_empty() {
            return Err(anyhow!(
                "Anthropic requires --api-key or ANTHROPIC_API_KEY env var"
            ));
        }
        let models = self.list_models_anthropic().await?;
        if models.is_empty() {
            return Err(anyhow!("No Anthropic models available"));
        }
        Ok(())
    }

    async fn complete_json_anthropic(&self, user_prompt: &str) -> Result<crate::ModelReply> {
        let key = self.api_key.as_deref().unwrap_or("");
        let url = "https://api.anthropic.com/v1/messages";
        let payload = json!({
            "model": self.model,
            "system": self.system_prompt,
            "messages": [
                {"role": "user", "content": format!("{}\n\nReturn JSON only. Respond with a valid JSON object.", user_prompt)}
            ],
            "max_tokens": 512
        });

        let resp = self
            .client
            .post(url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(self.parse_api_error("Anthropic", resp).await);
        }

        let parsed: AnthropicResponse = resp.json().await.context("invalid Anthropic response")?;
        let text = parsed
            .content
            .and_then(|c| c.into_iter().next())
            .and_then(|b| b.text)
            .unwrap_or_default();

        match serde_json::from_str::<crate::ModelReply>(text.trim()) {
            Ok(parsed) => Ok(parsed),
            Err(e) => {
                eprintln!(
                    "  {} JSON parse: {} — falling back",
                    ui::theme::paint_warning(&ui::theme::active(), "!"),
                    e
                );
                Ok(crate::ModelReply::fallback(text.trim()))
            }
        }
    }

    async fn complete_chat_stream_anthropic(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let key = self.api_key.as_deref().unwrap_or("");
        let url = "https://api.anthropic.com/v1/messages";

        let mut messages: Vec<serde_json::Value> = vec![];
        if !history.is_empty() {
            messages.push(json!({"role": "user", "content": history}));
            messages.push(json!({"role": "assistant", "content": "I understand. Continue."}));
        }
        messages.push(json!({"role": "user", "content": user_prompt}));

        let payload = json!({
            "model": self.model,
            "system": system_prompt,
            "messages": messages,
            "max_tokens": 4096,
            "stream": true
        });

        let resp = self
            .client
            .post(url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .context("failed to call Anthropic API")?;

        if !resp.status().is_success() {
            return Err(self.parse_api_error("Anthropic", resp).await);
        }

        self.stream_anthropic_sse(resp).await
    }

    // ── OpenAI SSE streaming ────────────────────────────────────────

    async fn stream_sse_response(&self, resp: reqwest::Response) -> Result<String> {
        let mut stream = resp.bytes_stream();
        let mut full = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("stream read error")?;
            let text = String::from_utf8_lossy(&chunk);
            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        return Ok(full);
                    }
                    if let Ok(chunk) = serde_json::from_str::<OpenAIStreamChunk>(data) {
                        if let Some(content) = chunk
                            .choices
                            .first()
                            .and_then(|c| c.delta.content.as_deref())
                        {
                            full.push_str(content);
                        }
                    }
                }
            }
        }
        Ok(full)
    }

    // ── Anthropic SSE streaming ─────────────────────────────────────

    async fn stream_anthropic_sse(&self, resp: reqwest::Response) -> Result<String> {
        let mut stream = resp.bytes_stream();
        let mut full = String::new();
        let mut event_type = String::new();
        let mut buf = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("stream read error")?;
            buf.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].to_string();
                buf = buf[pos + 1..].to_string();
                let trimmed = line.trim();

                if trimmed.is_empty() {
                    event_type.clear();
                    continue;
                }

                if let Some(event_val) = trimmed.strip_prefix("event: ") {
                    event_type = event_val.to_string();
                    continue;
                }

                if let Some(data) = trimmed.strip_prefix("data: ") {
                    if let Ok(chunk) =
                        serde_json::from_str::<AnthropicStreamChunk>(data)
                    {
                        if chunk.chunk_type.as_deref() == Some("content_block_delta") {
                            if let Some(text) = chunk.delta.and_then(|d| d.text) {
                                full.push_str(&text);
                            }
                        }
                    }
                }
            }
        }

        Ok(full)
    }

    // ── Error parsing ────────────────────────────────────────────────

    async fn parse_api_error(&self, provider: &str, resp: reqwest::Response) -> anyhow::Error {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let err_msg = serde_json::from_str::<LlmErrorResponse>(&body)
            .map(|v| v.error)
            .unwrap_or_else(|_| {
                body.chars().take(300).collect::<String>()
            });
        anyhow!("{} API failed ({}): {}", provider, status, err_msg)
    }
}
