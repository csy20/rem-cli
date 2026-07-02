use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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
            LlmErrorBody::String(s) => write!(f, "{s}"),
            LlmErrorBody::Object { message, r#type } => {
                if let Some(msg) = message {
                    write!(f, "{msg}")?;
                }
                if let Some(t) = r#type {
                    if message.is_some() {
                        write!(f, " ({t})")?;
                    } else {
                        write!(f, "{t}")?;
                    }
                }
                Ok(())
            }
        }
    }
}

pub(crate) static STREAM_CANCELLED: AtomicBool = AtomicBool::new(false);
pub(crate) static STREAM_TOKENS: AtomicBool = AtomicBool::new(false);

/// Reusable HTTP client with connection pooling (no hard timeout — individual
/// requests use their own per-call timeouts via tokio::time::timeout).
pub(crate) static HTTP_CLIENT: std::sync::LazyLock<Client> = std::sync::LazyLock::new(|| {
    Client::builder()
        .pool_max_idle_per_host(4)
        .build()
        .unwrap_or_else(|_| Client::new())
});

pub(crate) use crate::constants::{MAX_RESPONSE_BYTES, STREAM_CHUNK_TIMEOUT};

// ── ProviderContext: immutable shared state extracted from Provider ──────

#[derive(Clone)]
pub(crate) struct ProviderContext {
    pub client: Client,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub model_ctx: usize,
    pub kind: ProviderKind,
    pub reasoning_config: crate::reasoning::ReasoningConfig,
}

impl ProviderContext {
    pub fn new(
        base_url: String,
        model: String,
        api_key: Option<String>,
        model_ctx: usize,
        kind: ProviderKind,
        reasoning_config: crate::reasoning::ReasoningConfig,
        client: Client,
    ) -> Self {
        Self {
            client,
            base_url,
            model,
            api_key,
            model_ctx,
            kind,
            reasoning_config,
        }
    }

    pub fn api_key_str(&self) -> &str {
        self.api_key.as_deref().unwrap_or("")
    }
}

impl std::fmt::Debug for ProviderContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderContext")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("model_ctx", &self.model_ctx)
            .finish()
    }
}

// ── Provider ─────────────────────────────────────────────────────────────

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

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "openai" | "vllm" => ProviderKind::OpenAI,
            "gemini" | "google" => ProviderKind::Gemini,
            "anthropic" | "claude" => ProviderKind::Anthropic,
            "azure" => ProviderKind::Azure,
            "bedrock" | "aws" => ProviderKind::Bedrock,
            "openrouter" => ProviderKind::OpenRouter,
            _ => ProviderKind::Ollama,
        }
    }
}

#[async_trait]
pub(crate) trait ProviderBackend: Send + Sync {
    async fn list_models(&self, ctx: &ProviderContext) -> Result<Vec<String>>;
    async fn complete_json(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<crate::ModelReply>;
    async fn complete_chat_stream(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
    ) -> Result<String>;
    async fn complete_chat_stream_with_vision(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
        mime_type: &str,
        base64_data: &str,
    ) -> Result<String>;
    async fn complete_chat_stream_with_tools(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
        tool_specs: &[tools::ToolSpec],
    ) -> Result<tools::ToolResponse>;
}

pub struct Provider {
    pub kind: ProviderKind,
    pub ctx: ProviderContext,
    pub reasoning_config: crate::reasoning::ReasoningConfig,
    pub system_prompt: String,
    pub(crate) last_usage: Arc<Mutex<anthropic::AnthropicUsage>>,
    backend: Box<dyn ProviderBackend>,
}

impl Provider {
    /// Builds a ProviderContext with the current reasoning_config synced.
    fn build_ctx(&self) -> ProviderContext {
        let mut ctx = self.ctx.clone();
        ctx.reasoning_config = self.reasoning_config;
        ctx
    }
}

// ── Default base URLs and models ─────────────────────────────────────────

const DEFAULT_BASE_URLS: &[(ProviderKind, &str)] = &[
    (ProviderKind::Ollama, "http://localhost:11434"),
    (ProviderKind::OpenAI, "https://api.openai.com/v1"),
    (ProviderKind::Gemini, "https://generativelanguage.googleapis.com"),
    (ProviderKind::Anthropic, "https://api.anthropic.com"),
    (ProviderKind::Azure, ""),
    (ProviderKind::Bedrock, ""),
    (ProviderKind::OpenRouter, "https://openrouter.ai/api/v1"),
];

pub(crate) const DEFAULT_MODELS: &[(ProviderKind, &str)] = &[
    (ProviderKind::Gemini, "gemini-2.0-flash"),
    (ProviderKind::Anthropic, "claude-sonnet-4-20250514"),
    (ProviderKind::Bedrock, "anthropic.claude-sonnet-4-20250514"),
    (ProviderKind::OpenRouter, "openai/gpt-4o"),
];

pub(crate) fn default_base_url(kind: ProviderKind) -> String {
    DEFAULT_BASE_URLS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, url)| url.to_string())
        .unwrap_or_default()
}

pub(crate) fn default_model(kind: ProviderKind) -> Option<&'static str> {
    DEFAULT_MODELS.iter().find(|(k, _)| *k == kind).map(|(_, m)| *m)
}

pub(crate) const API_KEY_ENV_VARS: &[(ProviderKind, &str)] = &[
    (ProviderKind::OpenAI, "OPENAI_API_KEY"),
    (ProviderKind::Gemini, "GEMINI_API_KEY"),
    (ProviderKind::Anthropic, "ANTHROPIC_API_KEY"),
    (ProviderKind::Azure, "AZURE_OPENAI_API_KEY"),
    (ProviderKind::Bedrock, "AWS_ACCESS_KEY_ID"),
    (ProviderKind::OpenRouter, "OPENROUTER_API_KEY"),
];

pub(crate) fn api_key_env_var(kind: ProviderKind) -> Option<&'static str> {
    API_KEY_ENV_VARS.iter().find(|(k, _)| *k == kind).map(|(_, e)| *e)
}

pub fn ollama_api_url(base_url: &str, endpoint: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let ep = endpoint.trim_start_matches('/');
    if base.ends_with("/api") {
        format!("{base}/{ep}")
    } else {
        format!("{base}/api/{ep}")
    }
}

// ─── Fallback backend for feature-gated providers ────────────────────────

#[cfg(not(feature = "bedrock"))]
struct UnsupportedBackend;

#[cfg(not(feature = "bedrock"))]
#[async_trait]
impl ProviderBackend for UnsupportedBackend {
    async fn list_models(&self, _ctx: &ProviderContext) -> Result<Vec<String>> {
        Err(anyhow!("AWS Bedrock not compiled (enable 'bedrock' feature)"))
    }
    async fn complete_json(&self, _ctx: &ProviderContext, _sp: &str, _up: &str) -> Result<crate::ModelReply> {
        Err(anyhow!("AWS Bedrock not compiled (enable 'bedrock' feature)"))
    }
    async fn complete_chat_stream(&self, _ctx: &ProviderContext, _sp: &str, _up: &str, _hist: &str) -> Result<String> {
        Err(anyhow!("AWS Bedrock not compiled (enable 'bedrock' feature)"))
    }
    async fn complete_chat_stream_with_vision(
        &self,
        _ctx: &ProviderContext,
        _sp: &str,
        _up: &str,
        _hist: &str,
        _mime: &str,
        _b64: &str,
    ) -> Result<String> {
        Err(anyhow!("AWS Bedrock not compiled (enable 'bedrock' feature)"))
    }
    async fn complete_chat_stream_with_tools(
        &self,
        _ctx: &ProviderContext,
        _sp: &str,
        _up: &str,
        _hist: &str,
        _tools: &[tools::ToolSpec],
    ) -> Result<tools::ToolResponse> {
        Err(anyhow!("AWS Bedrock not compiled (enable 'bedrock' feature)"))
    }
}

// ─── Provider implementation ─────────────────────────────────────────────

fn build_client(_timeout_s: u64) -> Client {
    HTTP_CLIENT.clone()
}

impl Provider {
    pub fn new(
        kind: ProviderKind,
        base_url: String,
        model: String,
        timeout_s: u64,
        system_prompt: String,
        api_key: Option<String>,
        model_ctx: usize,
    ) -> Self {
        let client = build_client(timeout_s);
        let reasoning_config = crate::reasoning::ReasoningConfig::default();
        let ctx = ProviderContext::new(base_url, model, api_key, model_ctx, kind, reasoning_config, client);
        let last_usage = Arc::new(Mutex::new(anthropic::AnthropicUsage::default()));
        let backend: Box<dyn ProviderBackend> = match kind {
            ProviderKind::Ollama => Box::new(ollama::OllamaBackend),
            ProviderKind::OpenAI => Box::new(openai::OpenAIBackend),
            ProviderKind::Gemini => Box::new(gemini::GeminiBackend),
            ProviderKind::Anthropic => Box::new(anthropic::AnthropicBackend::new(Arc::clone(&last_usage))),
            ProviderKind::Azure => Box::new(azure::AzureBackend),
            #[cfg(feature = "bedrock")]
            ProviderKind::Bedrock => Box::new(bedrock::BedrockBackend),
            #[cfg(not(feature = "bedrock"))]
            ProviderKind::Bedrock => Box::new(UnsupportedBackend),
            ProviderKind::OpenRouter => Box::new(openrouter::OpenRouterBackend),
        };
        Self {
            kind,
            ctx,
            system_prompt,
            reasoning_config,
            last_usage,
            backend,
        }
    }

    pub fn supports_tools(&self) -> bool {
        tools::provider_supports_tools(&self.kind)
    }

    pub fn set_model(&mut self, model: String) {
        self.ctx.model = model;
    }

    pub fn provider_label(&self) -> String {
        match self.kind {
            ProviderKind::Ollama => format!("ollama/{}", self.ctx.model),
            ProviderKind::OpenAI => format!("openai/{}", self.ctx.model),
            ProviderKind::Gemini => format!("gemini/{}", self.ctx.model),
            ProviderKind::Anthropic => format!("anthropic/{}", self.ctx.model),
            ProviderKind::Azure => format!("azure/{}", self.ctx.model),
            ProviderKind::Bedrock => format!("bedrock/{}", self.ctx.model),
            ProviderKind::OpenRouter => format!("openrouter/{}", self.ctx.model),
        }
    }

    // ─── Retry logic ────────────────────────────────────────────────────

    fn is_transient_error(e: &anyhow::Error) -> bool {
        let err_str = e.to_string();
        if e.downcast_ref::<tokio::time::error::Elapsed>().is_some() {
            return true;
        }
        if let Some(req_err) = e.downcast_ref::<reqwest::Error>() {
            if req_err.is_timeout() || req_err.is_connect() {
                return true;
            }
            if let Some(status) = req_err.status() {
                let code = status.as_u16();
                if code == 429 || (500..=504).contains(&code) {
                    return true;
                }
            }
        }
        err_str.contains("connection refused") || err_str.contains("connection reset") || err_str.contains("timed out")
    }

    async fn with_retry<F, Fut, T>(f: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        // If stream was cancelled (e.g. Ctrl+C), don't retry
        if STREAM_CANCELLED.load(Ordering::SeqCst) {
            return Err(anyhow!("request cancelled by user"));
        }
        let mut last_err = None;
        for attempt in 0..crate::constants::LLM_RETRY_MAX_ATTEMPTS as usize {
            // Check cancellation before each retry attempt
            if STREAM_CANCELLED.load(Ordering::SeqCst) {
                return Err(anyhow!("request cancelled by user"));
            }
            match f().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    if !Self::is_transient_error(&e) || attempt == crate::constants::LLM_RETRY_MAX_ATTEMPTS as usize - 1
                    {
                        return Err(e);
                    }
                    last_err = Some(e);
                    let delay =
                        Duration::from_millis(crate::constants::LLM_RETRY_BASE_DELAY_MS * 2u64.pow(attempt as u32));
                    // Poll STREAM_CANCELLED during backoff sleep (avoids redundant signal listener)
                    let poll_interval = Duration::from_millis(100);
                    let mut remaining = delay;
                    while remaining > Duration::ZERO {
                        if STREAM_CANCELLED.load(Ordering::SeqCst) {
                            return Err(anyhow!("cancelled during retry backoff"));
                        }
                        let step = remaining.min(poll_interval);
                        sleep(step).await;
                        remaining = remaining.saturating_sub(step);
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("retry exhausted")))
    }

    // ─── Public API ─────────────────────────────────────────────────────

    pub async fn list_models(&self) -> Result<Vec<String>> {
        Self::with_retry(|| self.backend.list_models(&self.ctx)).await
    }

    pub async fn complete_json(&self, user_prompt: &str) -> Result<crate::ModelReply> {
        Self::with_retry(|| self.backend.complete_json(&self.ctx, &self.system_prompt, user_prompt)).await
    }

    pub async fn complete_chat_stream(&self, user_prompt: &str, system_prompt: &str, history: &str) -> Result<String> {
        let ctx = self.build_ctx();
        Self::with_retry(|| {
            self.backend
                .complete_chat_stream(&ctx, system_prompt, user_prompt, history)
        })
        .await
    }

    pub async fn complete_chat_stream_with_vision(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        mime_type: &str,
        base64_data: &str,
    ) -> Result<String> {
        let ctx = self.build_ctx();
        Self::with_retry(|| {
            self.backend.complete_chat_stream_with_vision(
                &ctx,
                system_prompt,
                user_prompt,
                history,
                mime_type,
                base64_data,
            )
        })
        .await
    }

    pub async fn complete_chat_stream_with_tools(
        &self,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        tool_specs: &[tools::ToolSpec],
    ) -> Result<tools::ToolResponse> {
        let ctx = self.build_ctx();
        Self::with_retry(|| {
            self.backend
                .complete_chat_stream_with_tools(&ctx, system_prompt, user_prompt, history, tool_specs)
        })
        .await
    }

    pub(crate) fn anthropic_usage(&self) -> anthropic::AnthropicUsage {
        self.last_usage.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Parses the history string from [`build_chat_history`] into
/// `(user_content, assistant_content)` pairs so providers can construct
/// proper message arrays instead of lumping everything into a single user message.
/// Within each turn, internal `\n` characters are escaped as `\\n` and unescaped here.
fn parse_history_turns(history: &str) -> Vec<(String, String)> {
    if history.is_empty() {
        return Vec::new();
    }
    let mut turns = Vec::new();
    for block in history.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        // Find the REM: boundary within the block (allows multi-line messages)
        let (user_part, assistant_part) = if let Some(rem_idx) = block.find("\nREM: ") {
            let user = &block[..rem_idx];
            let assistant = &block[rem_idx + 6..]; // skip "\nREM: "
            (user, assistant)
        } else if let Some(rem_idx) = block.strip_prefix("REM: ") {
            ("", rem_idx)
        } else {
            (block, "")
        };
        let user_content = user_part.strip_prefix("User: ").unwrap_or(user_part).trim();
        let assistant_content = assistant_part.trim();
        if !user_content.is_empty() || !assistant_content.is_empty() {
            turns.push((
                user_content.replace("\\n", "\n"),
                assistant_content.replace("\\n", "\n"),
            ));
        }
    }
    turns
}

// ─── Shared streaming / utility functions ────────────────────────────────

pub fn openai_chat_url(base_url: &str, kind: ProviderKind, model: &str) -> String {
    let base = base_url.trim_end_matches('/');
    match kind {
        ProviderKind::Azure => {
            format!("{base}/openai/deployments/{model}/chat/completions?api-version=2024-02-15-preview")
        }
        _ => format!("{base}/chat/completions"),
    }
}

pub fn openai_models_url(base_url: &str) -> String {
    format!("{}/models", base_url.trim_end_matches('/'))
}

pub fn add_openai_auth(req: reqwest::RequestBuilder, api_key: &str, kind: ProviderKind) -> reqwest::RequestBuilder {
    match kind {
        ProviderKind::Azure => req.header("api-key", api_key),
        _ => req.header("Authorization", format!("Bearer {api_key}")),
    }
}

pub fn emit_token(text: &str) {
    if STREAM_TOKENS.load(Ordering::SeqCst) {
        use std::io::Write;
        let _ = std::io::stdout().write(text.as_bytes());
        if text.contains('\n') {
            let _ = std::io::stdout().flush();
        }
    }
}

pub(crate) async fn stream_buf<F>(resp: reqwest::Response, mut on_line: F) -> Result<()>
where
    F: FnMut(&str) -> Result<bool>,
{
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::with_capacity(crate::constants::INITIAL_BUF_CAPACITY);
    let mut offset = 0usize;

    loop {
        if STREAM_CANCELLED.load(Ordering::SeqCst) {
            return Err(anyhow!("stream cancelled by user"));
        }
        let chunk = match timeout(STREAM_CHUNK_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(c))) => c,
            Ok(Some(Err(e))) => return Err(anyhow!("stream read error: {e}")),
            Ok(None) => break,
            Err(_) => return Err(anyhow!("stream timed out (no data for 60s)")),
        };
        if offset > 0 && buf.len() > crate::constants::INITIAL_BUF_CAPACITY * 4 {
            buf = buf[offset..].to_vec();
            offset = 0;
        }
        if buf.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(anyhow!(
                "stream buffer exceeded max size ({} bytes)",
                MAX_RESPONSE_BYTES
            ));
        }
        buf.extend_from_slice(&chunk);
        loop {
            let tail = &buf[offset..];
            match tail.iter().position(|&b| b == b'\n') {
                Some(pos) => {
                    let trimmed = std::str::from_utf8(&tail[..pos]).map(|s| s.trim()).unwrap_or("");
                    offset += pos + 1;
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

async fn stream_lines<F>(resp: reqwest::Response, mut on_line: F) -> Result<String>
where
    F: FnMut(&str, &mut String) -> Result<bool>,
{
    let mut full = String::with_capacity(crate::constants::INITIAL_BUF_CAPACITY);
    stream_buf(resp, |trimmed| {
        if !on_line(trimmed, &mut full)? {
            return Ok(false);
        }
        Ok(true)
    })
    .await?;
    Ok(full)
}

pub(super) async fn stream_sse_response(resp: reqwest::Response) -> Result<String> {
    stream_lines(resp, |trimmed, full| {
        if let Some(data) = trimmed.strip_prefix("data: ") {
            if data == "[DONE]" {
                return Ok(false);
            }
            if let Ok(chunk) = serde_json::from_str::<openai::OpenAIStreamChunk>(data) {
                if let Some(content) = chunk.choices.first().and_then(|c| c.delta.content.as_deref()) {
                    if !content.is_empty() {
                        full.push_str(content);
                        emit_token(content);
                        if full.len() > MAX_RESPONSE_BYTES {
                            return Err(anyhow!("response too large ({} bytes)", MAX_RESPONSE_BYTES));
                        }
                    }
                }
            }
        }
        Ok(true)
    })
    .await
}

pub(super) async fn stream_anthropic_sse(
    resp: reqwest::Response,
    last_usage: &Mutex<anthropic::AnthropicUsage>,
    show_reasoning: bool,
) -> Result<String> {
    stream_lines(resp, |trimmed, full| {
        if trimmed.starts_with("event: ") {
            return Ok(true);
        }
        if let Some(data) = trimmed.strip_prefix("data: ") {
            if let Ok(chunk) = serde_json::from_str::<anthropic::AnthropicStreamChunk>(data) {
                match chunk.chunk_type.as_deref() {
                    Some("content_block_delta") => {
                        if let Some(ref delta) = chunk.delta {
                            let is_thinking = delta.delta_type.as_deref() == Some("thinking_delta");
                            if is_thinking && !show_reasoning {
                                return Ok(true);
                            }
                            if let Some(ref text) = delta.text {
                                full.push_str(text);
                                emit_token(text);
                            }
                        }
                    }
                    Some("content_block_start") => {
                        if let Some(ref block) = chunk.content_block {
                            let is_thinking = block.block_type.as_deref() == Some("thinking");
                            if is_thinking && !show_reasoning {
                                return Ok(true);
                            }
                            if let Some(ref text) = block.text {
                                full.push_str(text);
                                emit_token(text);
                            }
                        }
                    }
                    Some("message_start") => {
                        if let Some(usage) = chunk.message.and_then(|m| m.usage) {
                            let mut last = last_usage.lock().unwrap_or_else(|e| e.into_inner());
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
                            let mut last = last_usage.lock().unwrap_or_else(|e| e.into_inner());
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
    .await
}

pub(super) async fn handle_ollama_error(resp: reqwest::Response, url: &str, model: &str) -> Result<String> {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_else(|e| format!("(read error: {e})"));
    let err_msg = serde_json::from_str::<LlmErrorResponse>(&body)
        .map(|v| v.error.to_string())
        .unwrap_or_else(|_| body.clone());
    if status.as_u16() == 404 && err_msg.to_lowercase().contains("model") {
        return Err(anyhow!("Model '{model}' not found. Pull it: `ollama pull {model}`"));
    }
    if status.as_u16() == 404 {
        return Err(anyhow!("Endpoint not found (404 at {url}). Check --ollama-url"));
    }
    Err(anyhow!("Ollama failed: {status} — {err_msg}"))
}

#[allow(dead_code)]
/// Stream handler for the legacy `/api/generate` endpoint (vs `/api/chat`).
/// Kept for reference in case non-chat Ollama endpoints are needed later.
pub(super) async fn stream_ollama_response(resp: reqwest::Response) -> Result<String> {
    stream_lines(resp, |trimmed, full| {
        if let Ok(obj) = serde_json::from_str::<ollama::OllamaStreamLine>(trimmed) {
            if let Some(token) = obj.response {
                full.push_str(&token);
                emit_token(&token);
            }
            if obj.done == Some(true) {
                return Ok(false);
            }
        }
        Ok(true)
    })
    .await
}

pub(super) async fn stream_gemini_sse(resp: reqwest::Response) -> Result<String> {
    stream_lines(resp, |trimmed, full| {
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
                    emit_token(&text);
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

pub(super) fn parse_json_fallback(text: &str) -> Result<crate::ModelReply> {
    match serde_json::from_str::<crate::ModelReply>(text.trim()) {
        Ok(parsed) => Ok(parsed),
        Err(e) => {
            tracing::warn!("JSON parse failed — falling back: {e}");
            Ok(crate::ModelReply::fallback(text.trim()))
        }
    }
}

pub(super) async fn parse_api_error(provider_name: &str, resp: reqwest::Response) -> anyhow::Error {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let err_msg = serde_json::from_str::<LlmErrorResponse>(&body)
        .map(|v| v.error.to_string())
        .unwrap_or_else(|_| body.chars().take(crate::constants::API_ERROR_BODY_MAX_CHARS).collect());
    anyhow!("{provider_name} API failed ({status}): {err_msg}")
}

pub(super) async fn openai_compat_complete_json(
    ctx: &ProviderContext,
    kind: ProviderKind,
    provider_name: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<crate::ModelReply> {
    let url = openai_chat_url(&ctx.base_url, kind, &ctx.model);
    let resp = add_openai_auth(ctx.client.post(&url), ctx.api_key_str(), kind)
        .json(&json!({
            "model": ctx.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": format!("{}\n\nReturn JSON only.", user_prompt)}
            ],
            "temperature": crate::constants::JSON_TEMPERATURE,
            "max_tokens": crate::constants::JSON_MAX_TOKENS,
            "response_format": {"type": "json_object"}
        }))
        .send()
        .await
        .with_context(|| format!("failed to call {provider_name} API"))?;
    if !resp.status().is_success() {
        return Err(parse_api_error(provider_name, resp).await);
    }
    let parsed: openai::OpenAIResponse = resp
        .json()
        .await
        .with_context(|| format!("invalid {provider_name} response"))?;
    let content = parsed
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or("");
    parse_json_fallback(content)
}

pub(super) async fn openai_compat_chat_stream(
    ctx: &ProviderContext,
    kind: ProviderKind,
    provider_name: &str,
    user_prompt: &str,
    system_prompt: &str,
    history: &str,
) -> Result<String> {
    let url = openai_chat_url(&ctx.base_url, kind, &ctx.model);
    let mut messages: Vec<serde_json::Value> = vec![json!({"role": "system", "content": system_prompt})];
    if !history.is_empty() {
        for (user_msg, assistant_msg) in parse_history_turns(history) {
            messages.push(json!({"role": "user", "content": user_msg}));
            if !assistant_msg.is_empty() {
                messages.push(json!({"role": "assistant", "content": assistant_msg}));
            }
        }
    }
    messages.push(json!({"role": "user", "content": user_prompt}));
    let resp = add_openai_auth(ctx.client.post(&url), ctx.api_key_str(), kind)
        .json(&json!({
            "model": ctx.model,
            "messages": messages,
            "stream": true,
            "temperature": crate::constants::DEFAULT_TEMPERATURE,
            "max_tokens": crate::constants::DEFAULT_MAX_TOKENS,
        }))
        .send()
        .await
        .with_context(|| format!("failed to call {provider_name} API"))?;
    if !resp.status().is_success() {
        return Err(parse_api_error(provider_name, resp).await);
    }
    stream_sse_response(resp).await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn openai_compat_chat_stream_with_vision(
    ctx: &ProviderContext,
    kind: ProviderKind,
    provider_name: &str,
    user_prompt: &str,
    system_prompt: &str,
    history: &str,
    mime_type: &str,
    base64_data: &str,
) -> Result<String> {
    let url = openai_chat_url(&ctx.base_url, kind, &ctx.model);
    let data_uri = format!("data:{mime_type};base64,{base64_data}");
    let mut messages: Vec<serde_json::Value> = vec![];
    messages.push(json!({"role": "system", "content": system_prompt}));
    if !history.is_empty() {
        for (user_msg, assistant_msg) in parse_history_turns(history) {
            messages.push(json!({"role": "user", "content": user_msg}));
            if !assistant_msg.is_empty() {
                messages.push(json!({"role": "assistant", "content": assistant_msg}));
            }
        }
    }
    messages.push(json!({
        "role": "user",
        "content": [
            {"type": "text", "text": user_prompt},
            {"type": "image_url", "image_url": {"url": data_uri}}
        ]
    }));
    let resp = add_openai_auth(ctx.client.post(&url), ctx.api_key_str(), kind)
        .json(&json!({
            "model": ctx.model,
            "messages": messages,
            "stream": true,
            "max_tokens": crate::constants::DEFAULT_MAX_TOKENS,
        }))
        .send()
        .await
        .with_context(|| format!("failed to call {provider_name} vision API"))?;
    if !resp.status().is_success() {
        return Err(parse_api_error(provider_name, resp).await);
    }
    stream_sse_response(resp).await
}

pub(super) async fn openai_compat_chat_stream_with_tools(
    ctx: &ProviderContext,
    kind: ProviderKind,
    provider_name: &str,
    user_prompt: &str,
    system_prompt: &str,
    history: &str,
    tool_specs: &[tools::ToolSpec],
) -> Result<tools::ToolResponse> {
    let url = openai_chat_url(&ctx.base_url, kind, &ctx.model);
    let mut messages: Vec<serde_json::Value> = vec![json!({"role": "system", "content": system_prompt})];
    if !history.is_empty() {
        for (user_msg, assistant_msg) in parse_history_turns(history) {
            messages.push(json!({"role": "user", "content": user_msg}));
            if !assistant_msg.is_empty() {
                messages.push(json!({"role": "assistant", "content": assistant_msg}));
            }
        }
    }
    messages.push(json!({"role": "user", "content": user_prompt}));
    let tools_json: Vec<serde_json::Value> = tool_specs.iter().map(|t| t.to_openai_tool()).collect();
    let mut payload = json!({
        "model": ctx.model,
        "messages": messages,
        "stream": true,
        "temperature": crate::constants::DEFAULT_TEMPERATURE,
        "max_tokens": crate::constants::DEFAULT_MAX_TOKENS,
    });
    if !tools_json.is_empty() {
        payload["tools"] = json!(tools_json);
        payload["tool_choice"] = json!("auto");
    }
    let resp = add_openai_auth(ctx.client.post(&url), ctx.api_key_str(), kind)
        .json(&payload)
        .send()
        .await
        .with_context(|| format!("failed to call {provider_name} API"))?;
    if !resp.status().is_success() {
        return Err(parse_api_error(provider_name, resp).await);
    }
    stream_openai_tool_response(resp).await
}

pub(crate) async fn stream_openai_tool_response(resp: reqwest::Response) -> Result<tools::ToolResponse> {
    let mut full_text = String::with_capacity(crate::constants::INITIAL_BUF_CAPACITY);
    let mut tool_acc = openai::AccumulatedToolCalls::default();

    stream_buf(resp, |trimmed| {
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
                emit_token(content);
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
