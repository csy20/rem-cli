//! LLM provider abstraction and API dispatch.
//! Defines [`Provider`], [`ProviderKind`], and shared streaming/error-handling
//! logic. Provider-specific response types and API methods live in submodules.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use std::future::Future;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use tokio::time::{sleep, timeout, Duration};

pub mod anthropic;
pub mod azure;
#[cfg(feature = "bedrock")]
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

/// Set to `true` during LLM calls to stream tokens to stdout as they arrive.
pub(crate) static STREAM_TOKENS: AtomicBool = AtomicBool::new(false);

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
#[derive(Debug, Clone, Copy, PartialEq)]
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

/// Backend trait implemented by each provider submodule.
/// Each method receives the full `Provider` so implementations can access
/// shared infrastructure (client, auth helpers, SSE parsers, etc.).
#[async_trait]
pub(crate) trait ProviderBackend: Send + Sync {
    async fn list_models(&self, provider: &Provider) -> Result<Vec<String>>;
    async fn complete_json(&self, provider: &Provider, user_prompt: &str) -> Result<crate::ModelReply>;
    async fn complete_chat_stream(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String>;
    async fn complete_chat_stream_with_vision(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        mime_type: &str,
        base64_data: &str,
    ) -> Result<String>;
    async fn complete_chat_stream_with_tools(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        tool_specs: &[tools::ToolSpec],
    ) -> Result<tools::ToolResponse>;
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
    /// Tracks token usage from the last Anthropic API call.
    pub(crate) last_usage: Mutex<crate::provider::anthropic::AnthropicUsage>,
    backend: Box<dyn ProviderBackend>,
}

/// Default base URLs for providers that have well-known endpoints.
const DEFAULT_BASE_URLS: &[(ProviderKind, &str)] = &[
    (ProviderKind::Ollama, "http://localhost:11434"),
    (ProviderKind::OpenAI, "https://api.openai.com/v1"),
    (ProviderKind::Gemini, "https://generativelanguage.googleapis.com"),
    (ProviderKind::Anthropic, "https://api.anthropic.com"),
    (ProviderKind::Azure, ""),
    (ProviderKind::Bedrock, ""),
    (ProviderKind::OpenRouter, "https://openrouter.ai/api/v1"),
];

/// Default model fallback for providers that have well-known models.
pub(crate) const DEFAULT_MODELS: &[(ProviderKind, &str)] = &[
    (ProviderKind::Gemini, "gemini-2.0-flash"),
    (ProviderKind::Anthropic, "claude-sonnet-4-20250514"),
    (ProviderKind::Bedrock, "anthropic.claude-sonnet-4-20250514"),
    (ProviderKind::OpenRouter, "openai/gpt-4o"),
];

/// Returns the default base URL for a provider kind.
pub(crate) fn default_base_url(kind: ProviderKind) -> String {
    DEFAULT_BASE_URLS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, url)| url.to_string())
        .unwrap_or_default()
}

/// Returns the default model for a provider kind, if any.
pub(crate) fn default_model(kind: ProviderKind) -> Option<&'static str> {
    DEFAULT_MODELS.iter().find(|(k, _)| *k == kind).map(|(_, m)| *m)
}

/// API key environment variable names per provider kind.
pub(crate) const API_KEY_ENV_VARS: &[(ProviderKind, &str)] = &[
    (ProviderKind::OpenAI, "OPENAI_API_KEY"),
    (ProviderKind::Gemini, "GEMINI_API_KEY"),
    (ProviderKind::Anthropic, "ANTHROPIC_API_KEY"),
    (ProviderKind::Azure, "AZURE_OPENAI_API_KEY"),
    (ProviderKind::Bedrock, "AWS_ACCESS_KEY_ID"),
    (ProviderKind::OpenRouter, "OPENROUTER_API_KEY"),
];

/// Returns the env var name for a provider's API key, if known.
pub(crate) fn api_key_env_var(kind: ProviderKind) -> Option<&'static str> {
    API_KEY_ENV_VARS.iter().find(|(k, _)| *k == kind).map(|(_, e)| *e)
}

/// Fallback backend returned when a feature-gated provider is not compiled.
#[cfg(not(feature = "bedrock"))]
struct UnsupportedBackend;

#[cfg(not(feature = "bedrock"))]
#[async_trait]
impl ProviderBackend for UnsupportedBackend {
    async fn list_models(&self, _provider: &Provider) -> Result<Vec<String>> {
        Err(anyhow!("AWS Bedrock not compiled (enable 'bedrock' feature)"))
    }
    async fn complete_json(&self, _provider: &Provider, _user_prompt: &str) -> Result<crate::ModelReply> {
        Err(anyhow!("AWS Bedrock not compiled (enable 'bedrock' feature)"))
    }
    async fn complete_chat_stream(
        &self,
        _provider: &Provider,
        _user_prompt: &str,
        _system_prompt: &str,
        _history: &str,
    ) -> Result<String> {
        Err(anyhow!("AWS Bedrock not compiled (enable 'bedrock' feature)"))
    }
    async fn complete_chat_stream_with_vision(
        &self,
        _provider: &Provider,
        _user_prompt: &str,
        _system_prompt: &str,
        _history: &str,
        _mime_type: &str,
        _base64_data: &str,
    ) -> Result<String> {
        Err(anyhow!("AWS Bedrock not compiled (enable 'bedrock' feature)"))
    }
    async fn complete_chat_stream_with_tools(
        &self,
        _provider: &Provider,
        _user_prompt: &str,
        _system_prompt: &str,
        _history: &str,
        _tool_specs: &[tools::ToolSpec],
    ) -> Result<tools::ToolResponse> {
        Err(anyhow!("AWS Bedrock not compiled (enable 'bedrock' feature)"))
    }
}

impl Provider {
    /// Builds a reqwest client with the given timeout.
    fn build_client(timeout_s: u64) -> Client {
        Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_s))
            .build()
            .unwrap_or_else(|_| Client::new())
    }

    /// Creates a new provider instance with the given kind and configuration.
    pub fn new(
        kind: ProviderKind,
        base_url: String,
        model: String,
        timeout_s: u64,
        system_prompt: String,
        api_key: Option<String>,
        model_ctx: usize,
    ) -> Self {
        let client = Self::build_client(timeout_s);
        let backend: Box<dyn ProviderBackend> = match kind {
            ProviderKind::Ollama => Box::new(ollama::OllamaBackend),
            ProviderKind::OpenAI => Box::new(openai::OpenAIBackend),
            ProviderKind::Gemini => Box::new(gemini::GeminiBackend),
            ProviderKind::Anthropic => Box::new(anthropic::AnthropicBackend),
            ProviderKind::Azure => Box::new(azure::AzureBackend),
            #[cfg(feature = "bedrock")]
            ProviderKind::Bedrock => Box::new(bedrock::BedrockBackend),
            #[cfg(not(feature = "bedrock"))]
            ProviderKind::Bedrock => Box::new(UnsupportedBackend),
            ProviderKind::OpenRouter => Box::new(openrouter::OpenRouterBackend),
        };
        Self {
            kind,
            client,
            base_url,
            model,
            system_prompt,
            api_key,
            model_ctx,
            reasoning_config: Default::default(),
            last_usage: Mutex::new(anthropic::AnthropicUsage::default()),
            backend,
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

    /// Returns true if the error looks transient (network issues, server errors, timeouts).
    fn is_transient_error(e: &anyhow::Error) -> bool {
        let err_str = e.to_string();
        // Check for tokio timeout
        if e.downcast_ref::<tokio::time::error::Elapsed>().is_some() {
            return true;
        }
        // Check for reqwest error kinds
        if let Some(req_err) = e.downcast_ref::<reqwest::Error>() {
            if req_err.is_timeout() || req_err.is_connect() || req_err.is_request() {
                return true;
            }
            if let Some(status) = req_err.status() {
                let code = status.as_u16();
                if code == 429 || (500..=504).contains(&code) {
                    return true;
                }
            }
        }
        // Fallback: string-based detection for wrapped errors
        err_str.contains("connection refused") || err_str.contains("connection reset") || err_str.contains("timed out")
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
                    if !Self::is_transient_error(&e) || attempt == 2 {
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
        Self::with_retry(|| self.backend.list_models(self)).await
    }

    /// Sends a prompt and expects a structured JSON response (parsed into [`ModelReply`]).
    pub async fn complete_json(&self, user_prompt: &str) -> Result<crate::ModelReply> {
        Self::with_retry(|| self.backend.complete_json(self, user_prompt)).await
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
        Self::with_retry(|| {
            self.backend.complete_chat_stream_with_vision(
                self,
                user_prompt,
                system_prompt,
                history,
                mime_type,
                base64_data,
            )
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
        Self::with_retry(|| {
            self.backend
                .complete_chat_stream_with_tools(self, user_prompt, system_prompt, history, tool_specs)
        })
        .await
    }

    /// Sends a prompt with streaming response, printing tokens as they arrive.
    pub async fn complete_chat_stream(&self, user_prompt: &str, system_prompt: &str, history: &str) -> Result<String> {
        Self::with_retry(|| {
            self.backend
                .complete_chat_stream(self, user_prompt, system_prompt, history)
        })
        .await
    }

    pub(crate) fn anthropic_usage(&self) -> anthropic::AnthropicUsage {
        self.last_usage.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(super) fn api_key_str(&self) -> &str {
        self.api_key.as_deref().unwrap_or("")
    }

    /// Builds the chat completions URL, applying Azure's api-version suffix when needed.
    pub(super) fn openai_chat_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        match self.kind {
            ProviderKind::Azure => {
                format!("{}/chat/completions?api-version=2024-02-15-preview", base)
            }
            _ => format!("{}/chat/completions", base),
        }
    }

    /// Builds the models listing URL.
    pub(super) fn openai_models_url(&self) -> String {
        format!("{}/models", self.base_url.trim_end_matches('/'))
    }

    /// Adds the appropriate auth header to an OpenAI-compatible request.
    pub(super) fn add_openai_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.kind {
            ProviderKind::Azure => req.header("api-key", self.api_key.as_deref().unwrap_or_default()),
            _ => req.header(
                "Authorization",
                format!("Bearer {}", self.api_key.as_deref().unwrap_or_default()),
            ),
        }
    }

    pub(super) fn parse_json_fallback(text: &str) -> Result<crate::ModelReply> {
        match serde_json::from_str::<crate::ModelReply>(text.trim()) {
            Ok(parsed) => Ok(parsed),
            Err(e) => {
                tracing::warn!("JSON parse failed — falling back: {}", e);
                Ok(crate::ModelReply::fallback(text.trim()))
            }
        }
    }

    pub(super) async fn parse_api_error(&self, provider: &str, resp: reqwest::Response) -> anyhow::Error {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let err_msg = serde_json::from_str::<LlmErrorResponse>(&body)
            .map(|v| v.error.to_string())
            .unwrap_or_else(|_| body.chars().take(300).collect::<String>());
        anyhow!("{} API failed ({}): {}", provider, status, err_msg)
    }

    /// Prints a token to stdout when live streaming is enabled.
    fn emit_token(text: &str) {
        if STREAM_TOKENS.load(Ordering::SeqCst) {
            use std::io::Write;
            let _ = std::io::stdout().write(text.as_bytes());
            let _ = std::io::stdout().flush();
        }
    }

    /// Core byte-buffering loop: reads chunks from the response stream, reassembles
    /// complete lines, and calls `on_line` for each non-empty trimmed line.
    /// The callback returns `Ok(true)` to continue or `Ok(false)` to stop early.
    pub(crate) async fn stream_buf<F>(resp: reqwest::Response, mut on_line: F) -> Result<()>
    where
        F: FnMut(&str) -> Result<bool>,
    {
        let mut stream = resp.bytes_stream();
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
                        if !on_line(trimmed)? {
                            return Ok(());
                        }
                    }
                    None => break,
                }
            }
        }
        Ok(())
    }

    /// Accumulates lines into a full response string. Wraps [`stream_buf`] with
    /// a text-accumulation callback.
    async fn stream_lines<F>(resp: reqwest::Response, mut on_line: F) -> Result<String>
    where
        F: FnMut(&str, &mut String) -> Result<bool>,
    {
        let mut full = String::with_capacity(4096);
        Self::stream_buf(resp, |trimmed| {
            if !on_line(trimmed, &mut full)? {
                return Ok(false);
            }
            Ok(true)
        })
        .await?;
        Ok(full)
    }

    pub(super) async fn stream_sse_response(&self, resp: reqwest::Response) -> Result<String> {
        Self::stream_lines(resp, |trimmed, full| {
            if let Some(data) = trimmed.strip_prefix("data: ") {
                if data == "[DONE]" {
                    return Ok(false);
                }
                if let Ok(chunk) = serde_json::from_str::<openai::OpenAIStreamChunk>(data) {
                    if let Some(content) = chunk.choices.first().and_then(|c| c.delta.content.as_deref()) {
                        full.push_str(content);
                        Self::emit_token(content);
                        if full.len() > MAX_RESPONSE_BYTES {
                            return Err(anyhow!("response too large ({} bytes)", MAX_RESPONSE_BYTES));
                        }
                    }
                }
            }
            Ok(true)
        })
        .await
    }

    pub(super) async fn stream_anthropic_sse(&self, resp: reqwest::Response) -> Result<String> {
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
                                Self::emit_token(&text);
                            }
                        }
                        Some("content_block_start") => {
                            if let Some(text) = chunk.content_block.and_then(|b| b.text) {
                                full.push_str(&text);
                                Self::emit_token(&text);
                            }
                        }
                        Some("message_start") => {
                            if let Some(usage) = chunk.message.and_then(|m| m.usage) {
                                let mut last_usage = self.last_usage.lock().unwrap_or_else(|e| e.into_inner());
                                if let Some(t) = usage.input_tokens {
                                    last_usage.input_tokens = t;
                                }
                                if let Some(t) = usage.cache_creation_input_tokens {
                                    last_usage.cache_creation_input_tokens = t;
                                }
                                if let Some(t) = usage.cache_read_input_tokens {
                                    last_usage.cache_read_input_tokens = t;
                                }
                            }
                        }
                        Some("message_delta") => {
                            if let Some(usage) = chunk.usage {
                                let mut last_usage = self.last_usage.lock().unwrap_or_else(|e| e.into_inner());
                                if let Some(t) = usage.output_tokens {
                                    last_usage.output_tokens = t;
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

    pub(super) async fn handle_ollama_error(&self, resp: reqwest::Response, url: &str) -> Result<String> {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_else(|e| format!("(read error: {})", e));
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
            return Err(anyhow!("Endpoint not found (404 at {}). Check --ollama-url", url));
        }
        Err(anyhow!("Ollama failed: {} — {}", status, err_msg))
    }

    pub(super) async fn stream_ollama_response(&self, resp: reqwest::Response) -> Result<String> {
        Self::stream_lines(resp, |trimmed, full| {
            if let Ok(obj) = serde_json::from_str::<ollama::OllamaStreamLine>(trimmed) {
                if let Some(token) = obj.response {
                    full.push_str(&token);
                    Self::emit_token(&token);
                }
                if obj.done == Some(true) {
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .await
    }

    pub(super) async fn stream_gemini_sse(&self, resp: reqwest::Response) -> Result<String> {
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
                        Self::emit_token(&text);
                        if full.len() > MAX_RESPONSE_BYTES {
                            return Err(anyhow!("response too large ({} bytes)", MAX_RESPONSE_BYTES));
                        }
                    }
                }
            }
            Ok(true)
        })
        .await
    }

    /// Shared tool streaming loop for OpenAI-compatible providers (OpenAI, Azure, OpenRouter).
    /// Handles SSE buffering, timeout, cancellation, text + tool call accumulation.
    /// Builds an OpenAI-compatible messages vec from system/history/user parts.
    pub(super) fn openai_compat_messages(
        system_prompt: &str,
        history: &str,
        user_prompt: &str,
    ) -> Vec<serde_json::Value> {
        let mut messages = vec![json!({"role": "system", "content": system_prompt})];
        if !history.is_empty() {
            messages.push(json!({"role": "user", "content": history}));
        }
        messages.push(json!({"role": "user", "content": user_prompt}));
        messages
    }

    /// OpenAI-compatible chat stream for Azure/OpenRouter (no reasoning extras).
    pub(super) async fn openai_compat_chat_stream(
        &self,
        provider_name: &str,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let url = self.openai_chat_url();
        let messages = Self::openai_compat_messages(system_prompt, history, user_prompt);
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
            .with_context(|| format!("failed to call {} API", provider_name))?;
        if !resp.status().is_success() {
            return Err(self.parse_api_error(provider_name, resp).await);
        }
        self.stream_sse_response(resp).await
    }

    /// OpenAI-compatible vision stream for Azure/OpenRouter.
    pub(super) async fn openai_compat_chat_stream_with_vision(
        &self,
        provider_name: &str,
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
            .with_context(|| format!("failed to call {} vision API", provider_name))?;
        if !resp.status().is_success() {
            return Err(self.parse_api_error(provider_name, resp).await);
        }
        self.stream_sse_response(resp).await
    }

    /// OpenAI-compatible tool streaming for Azure/OpenRouter.
    pub(super) async fn openai_compat_chat_stream_with_tools(
        &self,
        provider_name: &str,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        tool_specs: &[tools::ToolSpec],
    ) -> Result<tools::ToolResponse> {
        let url = self.openai_chat_url();
        let messages = Self::openai_compat_messages(system_prompt, history, user_prompt);
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
            .with_context(|| format!("failed to call {} API", provider_name))?;
        if !resp.status().is_success() {
            return Err(self.parse_api_error(provider_name, resp).await);
        }
        Self::stream_openai_tool_response(resp).await
    }

    pub(crate) async fn stream_openai_tool_response(resp: reqwest::Response) -> Result<tools::ToolResponse> {
        let mut full_text = String::with_capacity(4096);
        let mut tool_acc = openai::AccumulatedToolCalls::default();

        Self::stream_buf(resp, |trimmed| {
            if !trimmed.starts_with("data: ") {
                return Ok(true);
            }
            let data = &trimmed[6..];
            if data == "[DONE]" {
                return Ok(false);
            }
            if let Ok(chunk) = serde_json::from_str::<openai::OpenAIStreamChunk>(data) {
                if let Some(content) = chunk.choices.first().and_then(|c| c.delta.content.as_deref()) {
                    full_text.push_str(content);
                }
                if let Some(tool_calls) = chunk.choices.first().and_then(|c| c.delta.tool_calls.as_ref()) {
                    tool_acc.absorb_chunk(tool_calls);
                }
            }
            if full_text.len() > MAX_RESPONSE_BYTES {
                return Err(anyhow!("response too large ({} bytes)", MAX_RESPONSE_BYTES));
            }
            Ok(true)
        })
        .await?;

        if !tool_acc.is_empty() {
            Ok(tools::ToolResponse::ToolCalls(tool_acc.to_tool_calls()))
        } else {
            Ok(tools::ToolResponse::Text(full_text))
        }
    }
}
