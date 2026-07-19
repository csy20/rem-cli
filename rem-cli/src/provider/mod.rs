//! LLM provider infrastructure.
//!
//! This module defines the provider abstraction layer including:
//! - [`ProviderKind`] — enum over all supported LLM backends
//! - [`ProviderContext`] — immutable shared state (URL, credentials, model)
//! - [`ProviderBackend`] — async trait each provider implements
//! - [`Provider`] — the public-facing client with retry & circuit-breaker
//!
//! Sub-modules group the remaining functionality:
//! - [`provider_error`] — `ProviderError`, API error parsing, JSON fallback
//! - [`provider_stream`] — streaming helpers, SSE handlers, inline markdown
//! - [`provider_http`] — HTTP client, URL builders, OpenAI-compat wrappers
//! - [`provider_retry`] — circuit breaker, transient-error detection, retry loop

use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;

// ── Sub-modules ───────────────────────────────────────────────────────────

pub mod anthropic;
pub mod bedrock;
pub mod deepseek;
pub mod gemini;
pub mod ollama;
pub mod openai;
pub(crate) mod openai_compat;
pub(crate) mod provider_error;
pub mod provider_http;
pub(crate) mod provider_retry;
pub(crate) mod provider_stream;
pub mod tools;

#[cfg(test)]
mod tests;

// ── Re-exports from sub-modules ──────────────────────────────────────────

pub(crate) use crate::constants::MAX_RESPONSE_BYTES;
pub(crate) use provider_error::{handle_ollama_error, parse_api_error, parse_json_fallback, ProviderError};
#[allow(unused_imports)]
pub(crate) use provider_http::{
    add_openai_auth, build_messages_from_history, openai_chat_url, openai_compat_chat_stream,
    openai_compat_chat_stream_with_tools, openai_compat_chat_stream_with_vision, openai_compat_complete_json,
    openai_compat_list_models, openai_models_url, parse_history_turns,
};
pub(crate) use provider_retry::with_retry_and_circuit_breaker;
pub(crate) use provider_stream::{
    emit_token, stream_anthropic_sse, stream_buf, stream_gemini_sse, stream_sse_response, HAD_STREAMING_OUTPUT,
    HTTP_CLIENT, STREAM_CANCELLED, STREAM_TOKENS,
};

// ── ProviderKind ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(clippy::upper_case_acronyms)]
#[non_exhaustive]
pub enum ProviderKind {
    Ollama,
    OpenAI,
    Gemini,
    Anthropic,
    Azure,
    Bedrock,
    OpenRouter,
    DeepSeek,
    GitHub,
    Groq,
    XAI,
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
            ProviderKind::DeepSeek => "deepseek",
            ProviderKind::GitHub => "github",
            ProviderKind::Groq => "groq",
            ProviderKind::XAI => "xai",
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
            "deepseek" => ProviderKind::DeepSeek,
            "github" | "githubmodels" => ProviderKind::GitHub,
            "groq" | "groqcloud" => ProviderKind::Groq,
            "xai" | "grok" => ProviderKind::XAI,
            _ => ProviderKind::Ollama,
        }
    }
}

// ── ProviderContext ───────────────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct ProviderContext {
    pub client: reqwest::Client,
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
        client: reqwest::Client,
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

// ── ProviderBackend trait ─────────────────────────────────────────────────

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

// ── Provider struct ───────────────────────────────────────────────────────

pub struct Provider {
    pub kind: ProviderKind,
    pub ctx: ProviderContext,
    pub reasoning_config: crate::reasoning::ReasoningConfig,
    pub system_prompt: String,
    pub(crate) last_usage: Arc<Mutex<anthropic::AnthropicUsage>>,
    backend: Box<dyn ProviderBackend>,
}

// ── Default config tables ─────────────────────────────────────────────────

const DEFAULT_BASE_URLS: &[(ProviderKind, &str)] = &[
    (ProviderKind::Ollama, "http://localhost:11434"),
    (ProviderKind::OpenAI, "https://api.openai.com/v1"),
    (ProviderKind::Gemini, "https://generativelanguage.googleapis.com"),
    (ProviderKind::Anthropic, "https://api.anthropic.com"),
    (ProviderKind::Azure, ""),
    (ProviderKind::Bedrock, ""),
    (ProviderKind::OpenRouter, "https://openrouter.ai/api/v1"),
    (ProviderKind::DeepSeek, "https://api.deepseek.com"),
    (ProviderKind::GitHub, "https://models.inference.ai.azure.com"),
    (ProviderKind::Groq, "https://api.groq.com/openai/v1"),
    (ProviderKind::XAI, "https://api.x.ai/v1"),
];

pub(crate) const DEFAULT_MODELS: &[(ProviderKind, &str)] = &[
    (ProviderKind::Gemini, "gemini-2.0-flash"),
    (ProviderKind::Anthropic, "claude-sonnet-4-20250514"),
    (ProviderKind::Bedrock, "anthropic.claude-sonnet-4-20250514"),
    (ProviderKind::OpenRouter, "openai/gpt-4o"),
    (ProviderKind::DeepSeek, "deepseek-chat"),
    (ProviderKind::GitHub, "gpt-4o"),
    (ProviderKind::Groq, "llama-3.3-70b-versatile"),
    (ProviderKind::XAI, "grok-2"),
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
    (ProviderKind::DeepSeek, "DEEPSEEK_API_KEY"),
    (ProviderKind::GitHub, "GITHUB_TOKEN"),
    (ProviderKind::Groq, "GROQ_API_KEY"),
    (ProviderKind::XAI, "XAI_API_KEY"),
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
        Err(anyhow::anyhow!("AWS Bedrock not compiled (enable 'bedrock' feature)"))
    }
    async fn complete_json(&self, _ctx: &ProviderContext, _sp: &str, _up: &str) -> Result<crate::ModelReply> {
        Err(anyhow::anyhow!("AWS Bedrock not compiled (enable 'bedrock' feature)"))
    }
    async fn complete_chat_stream(&self, _ctx: &ProviderContext, _sp: &str, _up: &str, _hist: &str) -> Result<String> {
        Err(anyhow::anyhow!("AWS Bedrock not compiled (enable 'bedrock' feature)"))
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
        Err(anyhow::anyhow!("AWS Bedrock not compiled (enable 'bedrock' feature)"))
    }
    async fn complete_chat_stream_with_tools(
        &self,
        _ctx: &ProviderContext,
        _sp: &str,
        _up: &str,
        _hist: &str,
        _tools: &[tools::ToolSpec],
    ) -> Result<tools::ToolResponse> {
        Err(anyhow::anyhow!("AWS Bedrock not compiled (enable 'bedrock' feature)"))
    }
}

// ─── Provider implementation ─────────────────────────────────────────────

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
        let client = provider_http::build_client(timeout_s);
        let reasoning_config = crate::reasoning::ReasoningConfig::default();
        let ctx = ProviderContext::new(base_url, model, api_key, model_ctx, kind, reasoning_config, client);
        let last_usage = Arc::new(Mutex::new(anthropic::AnthropicUsage::default()));
        let backend: Box<dyn ProviderBackend> = match kind {
            ProviderKind::Ollama => Box::new(ollama::OllamaBackend),
            ProviderKind::OpenAI => Box::new(openai::OpenAIBackend),
            ProviderKind::Gemini => Box::new(gemini::GeminiBackend),
            ProviderKind::Anthropic => Box::new(anthropic::AnthropicBackend::new(Arc::clone(&last_usage))),
            ProviderKind::Azure => Box::new(openai_compat::OpenAICompatBackend {
                display_name: "Azure OpenAI",
                supports_list_models: false,
            }),
            #[cfg(feature = "bedrock")]
            ProviderKind::Bedrock => Box::new(bedrock::BedrockBackend),
            #[cfg(not(feature = "bedrock"))]
            ProviderKind::Bedrock => Box::new(UnsupportedBackend),
            ProviderKind::OpenRouter => Box::new(openai_compat::OpenAICompatBackend {
                display_name: "OpenRouter",
                supports_list_models: true,
            }),
            ProviderKind::DeepSeek => Box::new(deepseek::DeepSeekBackend),
            ProviderKind::GitHub => Box::new(openai_compat::OpenAICompatBackend {
                display_name: "GitHub Models",
                supports_list_models: true,
            }),
            ProviderKind::Groq => Box::new(openai_compat::OpenAICompatBackend {
                display_name: "Groq",
                supports_list_models: true,
            }),
            ProviderKind::XAI => Box::new(openai_compat::OpenAICompatBackend {
                display_name: "xAI",
                supports_list_models: true,
            }),
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

    fn build_ctx(&self) -> ProviderContext {
        let mut ctx = self.ctx.clone();
        ctx.reasoning_config = self.reasoning_config;
        ctx
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
            ProviderKind::DeepSeek => format!("deepseek/{}", self.ctx.model),
            ProviderKind::GitHub => format!("github/{}", self.ctx.model),
            ProviderKind::Groq => format!("groq/{}", self.ctx.model),
            ProviderKind::XAI => format!("xai/{}", self.ctx.model),
        }
    }

    // ─── Public API (with retry + circuit breaker) ──────────────────────

    pub async fn list_models(&self) -> Result<Vec<String>> {
        STREAM_CANCELLED.store(false, std::sync::atomic::Ordering::Relaxed);
        with_retry_and_circuit_breaker(self.kind, || self.backend.list_models(&self.ctx)).await
    }

    pub async fn complete_json(&self, user_prompt: &str) -> Result<crate::ModelReply> {
        STREAM_CANCELLED.store(false, std::sync::atomic::Ordering::Relaxed);
        with_retry_and_circuit_breaker(self.kind, || {
            self.backend.complete_json(&self.ctx, &self.system_prompt, user_prompt)
        })
        .await
    }

    pub async fn complete_chat_stream(&self, user_prompt: &str, system_prompt: &str, history: &str) -> Result<String> {
        STREAM_CANCELLED.store(false, std::sync::atomic::Ordering::Relaxed);
        let ctx = self.build_ctx();
        with_retry_and_circuit_breaker(self.kind, || {
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
        STREAM_CANCELLED.store(false, std::sync::atomic::Ordering::Relaxed);
        let ctx = self.build_ctx();
        with_retry_and_circuit_breaker(self.kind, || {
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
        STREAM_CANCELLED.store(false, std::sync::atomic::Ordering::Relaxed);
        let ctx = self.build_ctx();
        with_retry_and_circuit_breaker(self.kind, || {
            self.backend
                .complete_chat_stream_with_tools(&ctx, system_prompt, user_prompt, history, tool_specs)
        })
        .await
    }

    pub(crate) fn anthropic_usage(&self) -> anthropic::AnthropicUsage {
        self.last_usage.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    #[allow(dead_code)]
    pub fn is_transient_error(e: &anyhow::Error) -> bool {
        provider_retry::is_transient_error(e)
    }
}
