use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

use crate::ui;

#[derive(Debug, Deserialize)]
struct LlmErrorResponse {
    error: String,
}

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
}

pub struct Provider {
    pub kind: ProviderKind,
    pub client: Client,
    pub base_url: String,
    pub model: String,
    pub system_prompt: String,
    api_key: Option<String>,
    /// num_ctx passed to Ollama (or max_tokens guidance for OpenAI compat).
    /// Configurable via rem config (model_ctx). Key scaling knob: higher => model can use
    /// project memory + retrieved relevant code chunks from indexer.
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

    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        match self.kind {
            ProviderKind::Ollama => self.list_models_ollama().await,
            ProviderKind::OpenAI => self.list_models_openai().await,
        }
    }

    pub async fn healthcheck(&self) -> Result<()> {
        match self.kind {
            ProviderKind::Ollama => self.healthcheck_ollama().await,
            ProviderKind::OpenAI => self.healthcheck_openai().await,
        }
    }

    pub async fn complete_json(&self, user_prompt: &str) -> Result<crate::ModelReply> {
        match self.kind {
            ProviderKind::Ollama => self.complete_json_ollama(user_prompt).await,
            ProviderKind::OpenAI => self.complete_json_openai(user_prompt).await,
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
        }
    }

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
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let err_msg = serde_json::from_str::<LlmErrorResponse>(&body)
                .map(|v| v.error)
                .unwrap_or_else(|_| body.clone());
            return Err(anyhow!("OpenAI API failed: {} — {}", status, err_msg));
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
            return Err(anyhow!("Ollama failed: {} — {}", status, err_msg));
        }

        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_clone = cancelled.clone();
        let ctrlc_task = tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            cancelled_clone.store(true, Ordering::SeqCst);
        });

        let result = self
            .stream_response_ollama(resp, cancelled.clone(), ctrlc_task)
            .await;
        if cancelled.load(Ordering::SeqCst) {
            println!();
        }
        result
    }

    async fn stream_response_ollama(
        &self,
        resp: reqwest::Response,
        cancelled: Arc<AtomicBool>,
        ctrlc_task: tokio::task::JoinHandle<()>,
    ) -> Result<String> {
        let t = ui::theme::active();
        let mut stream = resp.bytes_stream();
        let mut full = String::new();
        let mut buf = String::new();
        let start = std::time::Instant::now();
        let mut token_count = 0u32;

        'stream_loop: while let Some(chunk) = stream.next().await {
            if cancelled.load(Ordering::SeqCst) {
                ctrlc_task.abort();
                break 'stream_loop;
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
                        if io::stdout().write_all(token.as_bytes()).is_err() {
                            ctrlc_task.abort();
                            break 'stream_loop;
                        }
                        let _ = io::stdout().flush();
                        full.push_str(token);
                        token_count += 1;
                    }
                    if obj["done"].as_bool() == Some(true) {
                        if cancelled.load(Ordering::SeqCst) {
                            ctrlc_task.abort();
                            break 'stream_loop;
                        }
                        let elapsed = start.elapsed();
                        let tps = if elapsed.as_secs_f64() > 0.0 {
                            token_count as f64 / elapsed.as_secs_f64()
                        } else {
                            0.0
                        };
                        println!(
                            "\n  {} {:.0} tokens/s in {:.1}s",
                            ui::theme::paint_dim(&t, "\u{2502}"),
                            tps,
                            elapsed.as_secs_f64()
                        );
                        ctrlc_task.abort();
                        return Ok(full);
                    }
                }
            }
        }
        ctrlc_task.abort();
        if cancelled.load(Ordering::SeqCst) {
            println!(
                "\n  {} stream cancelled — {} tokens received (Ctrl+C again to exit)",
                ui::theme::paint_warning(&t, "\u{2502}"),
                token_count
            );
        }
        println!();
        Ok(full)
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
            "max_tokens": 512
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
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let err_msg = serde_json::from_str::<LlmErrorResponse>(&body)
                .map(|v| v.error)
                .unwrap_or_else(|_| body.clone());
            return Err(anyhow!("OpenAI API failed: {} — {}", status, err_msg));
        }

        self.stream_response_openai(resp).await
    }

    async fn stream_response_openai(&self, resp: reqwest::Response) -> Result<String> {
        let mut stream = resp.bytes_stream();
        let mut full = String::new();
        let mut token_count = 0u32;
        let start = std::time::Instant::now();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("stream read error")?;
            let text = String::from_utf8_lossy(&chunk);
            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        let elapsed = start.elapsed();
                        let tps = if elapsed.as_secs_f64() > 0.0 {
                            token_count as f64 / elapsed.as_secs_f64()
                        } else {
                            0.0
                        };
                        println!(
                            "\n  {} {:.0} tokens/s in {:.1}s",
                            ui::theme::paint_dim(&ui::theme::active(), "\u{2502}"),
                            tps,
                            elapsed.as_secs_f64()
                        );
                        return Ok(full);
                    }
                    if let Ok(chunk) = serde_json::from_str::<OpenAIStreamChunk>(data) {
                        if let Some(content) = chunk
                            .choices
                            .first()
                            .and_then(|c| c.delta.content.as_deref())
                        {
                            if io::stdout().write_all(content.as_bytes()).is_err() {
                                return Ok(full);
                            }
                            let _ = io::stdout().flush();
                            full.push_str(content);
                            token_count += 1;
                        }
                    }
                }
            }
        }
        println!();
        Ok(full)
    }
}
