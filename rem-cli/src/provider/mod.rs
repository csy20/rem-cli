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
pub mod azure;
pub mod bedrock;
pub mod gemini;
pub mod ollama;
pub mod openai;
pub mod openrouter;
pub mod tools;

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

pub(crate) const STREAM_CHUNK_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

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
    Azure,
    Bedrock,
    OpenRouter,
}

impl ProviderKind {
    /// Returns the string label for this provider kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Ollama => "ollama",
            ProviderKind::OpenAI => "openai",
            ProviderKind::Gemini => "gemini",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Azure => "azure",
            ProviderKind::Bedrock => "bedrock",
            ProviderKind::OpenRouter => "openrouter",
        }
    }

    /// Parses a provider kind from a string (case-insensitive).
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "openai" => ProviderKind::OpenAI,
            "gemini" | "google" => ProviderKind::Gemini,
            "anthropic" | "claude" => ProviderKind::Anthropic,
            "azure" => ProviderKind::Azure,
            "bedrock" | "aws" => ProviderKind::Bedrock,
            "openrouter" => ProviderKind::OpenRouter,
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
    pub reasoning_config: crate::reasoning::ReasoningConfig,
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
            reasoning_config: Default::default(),
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
            reasoning_config: Default::default(),
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
            reasoning_config: Default::default(),
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
            reasoning_config: Default::default(),
        }
    }

    /// Creates a new Azure OpenAI provider instance.
    pub fn new_azure(
        base_url: String,
        model: String,
        timeout_s: u64,
        system_prompt: String,
        api_key: String,
        model_ctx: usize,
    ) -> Self {
        Self {
            kind: ProviderKind::Azure,
            client: Self::build_client(timeout_s),
            base_url,
            model,
            system_prompt,
            api_key: Some(api_key),
            model_ctx,
            reasoning_config: Default::default(),
        }
    }

    /// Creates a new AWS Bedrock provider instance.
    pub fn new_bedrock(
        model: String,
        timeout_s: u64,
        system_prompt: String,
        api_key: String,
        model_ctx: usize,
    ) -> Self {
        Self {
            kind: ProviderKind::Bedrock,
            client: Self::build_client(timeout_s),
            base_url: String::new(),
            model,
            system_prompt,
            api_key: Some(api_key),
            model_ctx,
            reasoning_config: Default::default(),
        }
    }

    /// Creates a new OpenRouter provider instance.
    pub fn new_openrouter(
        model: String,
        timeout_s: u64,
        system_prompt: String,
        api_key: String,
        model_ctx: usize,
    ) -> Self {
        Self {
            kind: ProviderKind::OpenRouter,
            client: Self::build_client(timeout_s),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            model,
            system_prompt,
            api_key: Some(api_key),
            model_ctx,
            reasoning_config: Default::default(),
        }
    }

    /// Returns true if this provider supports native tool/function calling.
    pub fn supports_tools(&self) -> bool {
        tools::provider_supports_tools(&self.kind)
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
            ProviderKind::Azure => format!("azure/{}", self.model),
            ProviderKind::Bedrock => format!("bedrock/{}", self.model),
            ProviderKind::OpenRouter => format!("openrouter/{}", self.model),
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
                ProviderKind::Azure => self.list_models_azure().await,
                ProviderKind::Bedrock => self.list_models_bedrock().await,
                ProviderKind::OpenRouter => self.list_models_openrouter().await,
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
                ProviderKind::Azure => self.healthcheck_azure().await,
                ProviderKind::Bedrock => self.healthcheck_bedrock().await,
                ProviderKind::OpenRouter => self.healthcheck_openrouter().await,
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
                ProviderKind::Azure => self.complete_json_azure(user_prompt).await,
                ProviderKind::Bedrock => self.complete_json_bedrock(user_prompt).await,
                ProviderKind::OpenRouter => self.complete_json_openrouter(user_prompt).await,
            }
        })
        .await
    }

    /// Sends a prompt with an image attached, streaming response.
    /// Supported by OpenAI, Anthropic, Gemini, and Ollama.
    pub async fn complete_chat_stream_with_vision(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        mime_type: &str,
        base64_data: &str,
    ) -> Result<String> {
        Self::with_retry(|| async {
            match self.kind {
                ProviderKind::OpenAI => {
                    self.complete_chat_vision_openai(
                        user_prompt,
                        system_prompt,
                        history,
                        mime_type,
                        base64_data,
                    )
                    .await
                }
                ProviderKind::Anthropic => {
                    self.complete_chat_vision_anthropic(
                        user_prompt,
                        system_prompt,
                        history,
                        mime_type,
                        base64_data,
                    )
                    .await
                }
                ProviderKind::Gemini => {
                    self.complete_chat_vision_gemini(
                        user_prompt,
                        system_prompt,
                        history,
                        mime_type,
                        base64_data,
                    )
                    .await
                }
                ProviderKind::Ollama => {
                    self.complete_chat_vision_ollama(
                        user_prompt,
                        system_prompt,
                        history,
                        mime_type,
                        base64_data,
                    )
                    .await
                }
                ProviderKind::Azure => {
                    self.complete_chat_vision_azure(
                        user_prompt,
                        system_prompt,
                        history,
                        mime_type,
                        base64_data,
                    )
                    .await
                }
                ProviderKind::Bedrock => {
                    self.complete_chat_vision_bedrock(
                        user_prompt,
                        system_prompt,
                        history,
                        mime_type,
                        base64_data,
                    )
                    .await
                }
                ProviderKind::OpenRouter => {
                    self.complete_chat_vision_openrouter(
                        user_prompt,
                        system_prompt,
                        history,
                        mime_type,
                        base64_data,
                    )
                    .await
                }
            }
        })
        .await
    }

    /// Sends a prompt with streaming response, printing tokens as they arrive.
    /// Supports tool calls: returns ToolResponse which may contain text or tool calls.
    pub async fn complete_chat_stream_with_tools(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        tool_specs: &[tools::ToolSpec],
    ) -> Result<tools::ToolResponse> {
        Self::with_retry(|| async {
            match self.kind {
                ProviderKind::Ollama => {
                    self.complete_chat_stream_tools_ollama(
                        user_prompt,
                        system_prompt,
                        history,
                        tool_specs,
                    )
                    .await
                }
                ProviderKind::OpenAI => {
                    self.complete_chat_stream_tools_openai(
                        user_prompt,
                        system_prompt,
                        history,
                        tool_specs,
                    )
                    .await
                }
                ProviderKind::Gemini => {
                    self.complete_chat_stream_tools_gemini(
                        user_prompt,
                        system_prompt,
                        history,
                        tool_specs,
                    )
                    .await
                }
                ProviderKind::Anthropic => {
                    self.complete_chat_stream_tools_anthropic(
                        user_prompt,
                        system_prompt,
                        history,
                        tool_specs,
                    )
                    .await
                }
                ProviderKind::Azure => {
                    self.complete_chat_stream_tools_azure(
                        user_prompt,
                        system_prompt,
                        history,
                        tool_specs,
                    )
                    .await
                }
                ProviderKind::Bedrock => {
                    self.complete_chat_stream_tools_bedrock(
                        user_prompt,
                        system_prompt,
                        history,
                        tool_specs,
                    )
                    .await
                }
                ProviderKind::OpenRouter => {
                    self.complete_chat_stream_tools_openrouter(
                        user_prompt,
                        system_prompt,
                        history,
                        tool_specs,
                    )
                    .await
                }
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
                ProviderKind::Azure => {
                    self.complete_chat_stream_azure(user_prompt, system_prompt, history)
                        .await
                }
                ProviderKind::Bedrock => {
                    self.complete_chat_stream_bedrock(user_prompt, system_prompt, history)
                        .await
                }
                ProviderKind::OpenRouter => {
                    self.complete_chat_stream_openrouter(user_prompt, system_prompt, history)
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

    /// Core streaming loop: reads SSE bytes, buffers partial lines, yields each line
    /// to the provided callback. The callback receives the trimmed line and a mutable
    /// output buffer. Return `Ok(true)` from the callback to continue, `Ok(false)` to
    /// stop early with the current output, or `Err` to abort.
    async fn stream_lines<F>(resp: reqwest::Response, mut on_line: F) -> Result<String>
    where
        F: FnMut(&str, &mut String) -> Result<bool>,
    {
        let mut stream = resp.bytes_stream();
        let mut full = String::with_capacity(4096);
        let mut buf = String::with_capacity(4096);
        let mut cursor = 0usize;

        loop {
            if STREAM_CANCELLED.load(Ordering::SeqCst) {
                break;
            }
            let chunk = match timeout(STREAM_CHUNK_TIMEOUT, stream.next()).await {
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
                        if !on_line(trimmed, &mut full)? {
                            return Ok(full);
                        }
                    }
                    None => break,
                }
            }
        }
        Ok(full)
    }

    async fn stream_sse_response(&self, resp: reqwest::Response) -> Result<String> {
        Self::stream_lines(resp, |trimmed, full| {
            if let Some(data) = trimmed.strip_prefix("data: ") {
                if data == "[DONE]" {
                    return Ok(false);
                }
                if let Ok(chunk) = serde_json::from_str::<openai::OpenAIStreamChunk>(data) {
                    if let Some(content) = chunk
                        .choices
                        .first()
                        .and_then(|c| c.delta.content.as_deref())
                    {
                        full.push_str(content);
                        if full.len() > MAX_RESPONSE_BYTES {
                            return Err(anyhow!(
                                "response too large ({} bytes)",
                                MAX_RESPONSE_BYTES
                            ));
                        }
                    }
                }
            }
            Ok(true)
        })
        .await
    }

    async fn stream_anthropic_sse(&self, resp: reqwest::Response) -> Result<String> {
        let result = Self::stream_lines(resp, |trimmed, full| {
            if trimmed.starts_with("event: ") {
                return Ok(true);
            }
            if let Some(data) = trimmed.strip_prefix("data: ") {
                if let Ok(chunk) = serde_json::from_str::<anthropic::AnthropicStreamChunk>(data) {
                    match chunk.chunk_type.as_deref() {
                        Some("content_block_delta") => {
                            if let Some(text) = chunk.delta.and_then(|d| d.text) {
                                full.push_str(&text);
                            }
                        }
                        Some("content_block_start") => {
                            if let Some(text) = chunk.content_block.and_then(|b| b.text) {
                                full.push_str(&text);
                            }
                        }
                        Some("message_start") => {
                            if let Some(usage) = chunk.message.and_then(|m| m.usage) {
                                let mut last = crate::provider::anthropic::LAST_USAGE
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner());
                                if let Some(t) = usage.input_tokens {
                                    last.input_tokens = t;
                                }
                                if let Some(t) = usage.cache_creation_input_tokens {
                                    last.cache_creation_input_tokens = t;
                                }
                                if let Some(t) = usage.cache_read_input_tokens {
                                    last.cache_read_input_tokens = t;
                                }
                            }
                        }
                        Some("message_delta") => {
                            if let Some(usage) = chunk.usage {
                                let mut last = crate::provider::anthropic::LAST_USAGE
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner());
                                if let Some(t) = usage.output_tokens {
                                    last.output_tokens = t;
                                }
                            }
                        }
                        _ => {}
                    }
                    if full.len() > MAX_RESPONSE_BYTES {
                        return Err(anyhow!("response too large ({} bytes)", MAX_RESPONSE_BYTES));
                    }
                }
            }
            Ok(true)
        })
        .await;
        result
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
        Self::stream_lines(resp, |trimmed, full| {
            if let Ok(obj) = serde_json::from_str::<ollama::OllamaStreamLine>(trimmed) {
                if let Some(token) = obj.response {
                    full.push_str(&token);
                }
                if obj.done == Some(true) {
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .await
    }

    async fn stream_gemini_sse(&self, resp: reqwest::Response) -> Result<String> {
        Self::stream_lines(resp, |trimmed, full| {
            if trimmed.is_empty() || trimmed.starts_with(':') {
                return Ok(true);
            }
            if let Some(data) = trimmed.strip_prefix("data: ") {
                if let Ok(chunk) = serde_json::from_str::<gemini::GeminiStreamChunk>(data) {
                    if let Some(text) = chunk
                        .candidates
                        .and_then(|c| c.into_iter().next())
                        .and_then(|c| c.content)
                        .and_then(|c| c.parts)
                        .and_then(|p| p.into_iter().next())
                        .and_then(|p| p.text)
                    {
                        full.push_str(&text);
                        if full.len() > MAX_RESPONSE_BYTES {
                            return Err(anyhow!(
                                "response too large ({} bytes)",
                                MAX_RESPONSE_BYTES
                            ));
                        }
                    }
                }
            }
            Ok(true)
        })
        .await
    }
}
