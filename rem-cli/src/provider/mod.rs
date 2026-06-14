//! LLM provider abstraction and API dispatch.
//! Defines [`Provider`], [`ProviderKind`], and shared streaming/error-handling
//! logic. Provider-specific response types and API methods live in submodules.

use std::sync::atomic::{AtomicBool, Ordering};

use std::future::Future;

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use tokio::time::{sleep, timeout, Duration};

pub mod anthropic;
pub mod gemini;
pub mod ollama;
pub mod openai;

#[cfg(test)]
mod tests;

#[derive(Debug, Deserialize)]
struct LlmErrorResponse {
    #[serde(default)]
    error: LlmErrorBody,
}

#[derive(Debug, Deserialize, Default)]
#[serde(untagged)]
enum LlmErrorBody {
    #[default]
    Empty,
    String(String),
    Object {
        message: Option<String>,
        r#type: Option<String>,
    },
}

impl std::fmt::Display for LlmErrorBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmErrorBody::Empty => write!(f, "unknown error"),
            LlmErrorBody::String(s) => write!(f, "{}", s),
            LlmErrorBody::Object { message, r#type } => {
                if let Some(msg) = message {
                    write!(f, "{}", msg)?;
                }
                if let Some(t) = r#type {
                    if message.is_some() {
                        write!(f, " ({})", t)?;
                    } else {
                        write!(f, "{}", t)?;
                    }
                }
                Ok(())
            }
        }
    }
}

/// Set to `true` by the global Ctrl+C handler to cancel an in-flight stream.
pub(crate) static STREAM_CANCELLED: AtomicBool = AtomicBool::new(false);

/// Builds a full API URL by combining base URL and endpoint path.
pub fn api_url(base_url: &str, endpoint: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let ep = endpoint.trim_start_matches('/');
    if base.ends_with("/api") {
        format!("{}/{}", base, ep)
    } else {
        format!("{}/api/{}", base, ep)
    }
}

/// Supported LLM provider backends.
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderKind {
    Ollama,
    OpenAI,
    Gemini,
    Anthropic,
}

impl ProviderKind {
    /// Returns the string label for this provider kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Ollama => "ollama",
            ProviderKind::OpenAI => "openai",
            ProviderKind::Gemini => "gemini",
            ProviderKind::Anthropic => "anthropic",
        }
    }

    /// Parses a provider kind from a string (case-insensitive).
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "openai" => ProviderKind::OpenAI,
            "gemini" | "google" => ProviderKind::Gemini,
            "anthropic" | "claude" => ProviderKind::Anthropic,
            _ => ProviderKind::Ollama,
        }
    }
}

/// An LLM provider client with routing to the appropriate backend.
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
    /// Builds a reqwest client with the given timeout.
    fn build_client(timeout_s: u64) -> Client {
        Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_s))
            .build()
            .unwrap_or_else(|_| Client::new())
    }

    /// Creates a new Ollama provider instance.
    pub fn new_ollama(
        base_url: String,
        model: String,
        timeout_s: u64,
        system_prompt: String,
        model_ctx: usize,
    ) -> Self {
        Self {
            kind: ProviderKind::Ollama,
            client: Self::build_client(timeout_s),
            base_url,
            model,
            system_prompt,
            api_key: None,
            model_ctx,
        }
    }

    /// Creates a new OpenAI-compatible provider instance.
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
            client: Self::build_client(timeout_s),
            base_url,
            model,
            system_prompt,
            api_key: Some(api_key),
            model_ctx,
        }
    }

    /// Creates a new Google Gemini provider instance.
    pub fn new_gemini(
        api_key: String,
        model: String,
        timeout_s: u64,
        system_prompt: String,
        model_ctx: usize,
    ) -> Self {
        Self {
            kind: ProviderKind::Gemini,
            client: Self::build_client(timeout_s),
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            model,
            system_prompt,
            api_key: Some(api_key),
            model_ctx,
        }
    }

    /// Creates a new Anthropic (Claude) provider instance.
    pub fn new_anthropic(
        api_key: String,
        model: String,
        timeout_s: u64,
        system_prompt: String,
        model_ctx: usize,
    ) -> Self {
        Self {
            kind: ProviderKind::Anthropic,
            client: Self::build_client(timeout_s),
            base_url: "https://api.anthropic.com".to_string(),
            model,
            system_prompt,
            api_key: Some(api_key),
            model_ctx,
        }
    }

    /// Overrides the model name.
    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    /// Returns a display label like `ollama/rem-coder`.
    pub fn provider_label(&self) -> String {
        match self.kind {
            ProviderKind::Ollama => format!("ollama/{}", self.model),
            ProviderKind::OpenAI => format!("openai/{}", self.model),
            ProviderKind::Gemini => format!("gemini/{}", self.model),
            ProviderKind::Anthropic => format!("anthropic/{}", self.model),
        }
    }

    async fn with_retry<F, Fut, T>(f: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let mut last_err = None;
        for attempt in 0..3 {
            match f().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    let err_str = e.to_string();
                    let is_transient = err_str.contains("timeout")
                        || err_str.contains("timed out")
                        || err_str.contains("429")
                        || err_str.contains("500")
                        || err_str.contains("502")
                        || err_str.contains("503")
                        || err_str.contains("504")
                        || err_str.contains("connection refused")
                        || err_str.contains("connection reset");
                    if !is_transient || attempt == 2 {
                        return Err(e);
                    }
                    last_err = Some(e);
                    sleep(Duration::from_millis(500 * 2u64.pow(attempt))).await;
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("retry exhausted")))
    }

    /// Lists available models from the provider.
    pub async fn list_models(&self) -> Result<Vec<String>> {
        Self::with_retry(|| async {
            match self.kind {
                ProviderKind::Ollama => self.list_models_ollama().await,
                ProviderKind::OpenAI => self.list_models_openai().await,
                ProviderKind::Gemini => self.list_models_gemini().await,
                ProviderKind::Anthropic => self.list_models_anthropic().await,
            }
        })
        .await
    }

    /// Checks if the provider is reachable and the API is healthy.
    pub async fn healthcheck(&self) -> Result<()> {
        Self::with_retry(|| async {
            match self.kind {
                ProviderKind::Ollama => self.healthcheck_ollama().await,
                ProviderKind::OpenAI => self.healthcheck_openai().await,
                ProviderKind::Gemini => self.healthcheck_gemini().await,
                ProviderKind::Anthropic => self.healthcheck_anthropic().await,
            }
        })
        .await
    }

    /// Sends a prompt and expects a structured JSON response (parsed into [`ModelReply`]).
    pub async fn complete_json(&self, user_prompt: &str) -> Result<crate::ModelReply> {
        Self::with_retry(|| async {
            match self.kind {
                ProviderKind::Ollama => self.complete_json_ollama(user_prompt).await,
                ProviderKind::OpenAI => self.complete_json_openai(user_prompt).await,
                ProviderKind::Gemini => self.complete_json_gemini(user_prompt).await,
                ProviderKind::Anthropic => self.complete_json_anthropic(user_prompt).await,
            }
        })
        .await
    }

    /// Sends a prompt with streaming response, printing tokens as they arrive.
    pub async fn complete_chat_stream(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        Self::with_retry(|| async {
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
        })
        .await
    }

    fn api_key_str(&self) -> &str {
        self.api_key.as_deref().unwrap_or("")
    }

    fn parse_json_fallback(text: &str) -> Result<crate::ModelReply> {
        match serde_json::from_str::<crate::ModelReply>(text.trim()) {
            Ok(parsed) => Ok(parsed),
            Err(e) => {
                eprintln!(
                    "  {} JSON parse: {} — falling back",
                    crate::ui::theme::paint_warning(&crate::ui::theme::active(), "!"),
                    e
                );
                Ok(crate::ModelReply::fallback(text.trim()))
            }
        }
    }

    async fn parse_api_error(&self, provider: &str, resp: reqwest::Response) -> anyhow::Error {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let err_msg = serde_json::from_str::<LlmErrorResponse>(&body)
            .map(|v| v.error.to_string())
            .unwrap_or_else(|_| body.chars().take(300).collect::<String>());
        anyhow!("{} API failed ({}): {}", provider, status, err_msg)
    }

    const STREAM_CHUNK_TIMEOUT: Duration = Duration::from_secs(60);
    const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

    async fn stream_sse_response(&self, resp: reqwest::Response) -> Result<String> {
        let mut stream = resp.bytes_stream();
        let mut full = String::with_capacity(4096);

        loop {
            if STREAM_CANCELLED.load(Ordering::SeqCst) {
                break;
            }
            let chunk = match timeout(Self::STREAM_CHUNK_TIMEOUT, stream.next()).await {
                Ok(Some(Ok(c))) => c,
                Ok(Some(Err(e))) => return Err(anyhow!("stream read error: {}", e)),
                Ok(None) => break,
                Err(_) => return Err(anyhow!("stream timed out (no data for 60s)")),
            };
            let text = String::from_utf8_lossy(&chunk);
            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        return Ok(full);
                    }
                    if let Ok(chunk) = serde_json::from_str::<openai::OpenAIStreamChunk>(data) {
                        if let Some(content) = chunk
                            .choices
                            .first()
                            .and_then(|c| c.delta.content.as_deref())
                        {
                            full.push_str(content);
                            if full.len() > Self::MAX_RESPONSE_BYTES {
                                return Err(anyhow!(
                                    "response too large ({} bytes)",
                                    Self::MAX_RESPONSE_BYTES
                                ));
                            }
                        }
                    }
                }
            }
        }
        Ok(full)
    }

    async fn stream_anthropic_sse(&self, resp: reqwest::Response) -> Result<String> {
        let mut stream = resp.bytes_stream();
        let mut full = String::with_capacity(4096);
        let mut buf = String::with_capacity(4096);
        let mut cursor = 0usize;

        loop {
            if STREAM_CANCELLED.load(Ordering::SeqCst) {
                break;
            }
            let chunk = match timeout(Self::STREAM_CHUNK_TIMEOUT, stream.next()).await {
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

                        if trimmed.is_empty() {
                            continue;
                        }

                        if trimmed.starts_with("event: ") {
                            continue;
                        }

                        if let Some(data) = trimmed.strip_prefix("data: ") {
                            if let Ok(chunk) =
                                serde_json::from_str::<anthropic::AnthropicStreamChunk>(data)
                            {
                                if chunk.chunk_type.as_deref() == Some("content_block_delta") {
                                    if let Some(text) = chunk.delta.and_then(|d| d.text) {
                                        full.push_str(&text);
                                        if full.len() > Self::MAX_RESPONSE_BYTES {
                                            return Err(anyhow!(
                                                "response too large ({} bytes)",
                                                Self::MAX_RESPONSE_BYTES
                                            ));
                                        }
                                    }
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

    async fn handle_ollama_error(&self, resp: reqwest::Response, url: &str) -> Result<String> {
        let status = resp.status();
        let body = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("(read error: {})", e));
        let err_msg = serde_json::from_str::<LlmErrorResponse>(&body)
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
        Err(anyhow!("Ollama failed: {} — {}", status, err_msg))
    }

    async fn stream_ollama_response(&self, resp: reqwest::Response) -> Result<String> {
        const MAX_BUF: usize = 10 * 1024 * 1024;

        let mut stream = resp.bytes_stream();
        let mut full = String::new();
        let mut buf = String::new();
        let mut cursor = 0usize;

        loop {
            if STREAM_CANCELLED.load(Ordering::SeqCst) {
                break;
            }
            let chunk = match timeout(Self::STREAM_CHUNK_TIMEOUT, stream.next()).await {
                Ok(Some(Ok(c))) => c,
                Ok(Some(Err(e))) => return Err(anyhow!("stream read error: {}", e)),
                Ok(None) => break,
                Err(_) => return Err(anyhow!("stream timed out (no data for 60s)")),
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));
            if buf.len() > MAX_BUF {
                return Err(anyhow!("response too large (>{MAX_BUF} bytes)"));
            }
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
                        if let Ok(obj) = serde_json::from_str::<ollama::OllamaStreamLine>(trimmed) {
                            if let Some(token) = obj.response {
                                full.push_str(&token);
                            }
                            if obj.done == Some(true) {
                                return Ok(full);
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
