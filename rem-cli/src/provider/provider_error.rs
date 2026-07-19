use anyhow::{anyhow, Result};
use serde::Deserialize;

/// Structured error type for LLM provider operations.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ProviderError {
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("rate limited: {0}")]
    RateLimit(String),
    #[error("request timed out: {0}")]
    Timeout(String),
    #[error("server error: {0}")]
    ServerError(String),
    #[error("cancelled by user")]
    Cancelled,
    #[error("response too large ({0} bytes)")]
    ResponseTooLarge(u64),
    #[error("{0}")]
    Other(String),
}

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

pub(crate) async fn parse_api_error(
    provider_name: &str,
    resp: reqwest::Response,
    api_key: Option<&str>,
) -> anyhow::Error {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let redacted_body = redact_api_key(&body, api_key);
    let err_msg = serde_json::from_str::<LlmErrorResponse>(&redacted_body)
        .map(|v| v.error.to_string())
        .unwrap_or_else(|_| {
            redacted_body
                .chars()
                .take(crate::constants::API_ERROR_BODY_MAX_CHARS)
                .collect()
        });
    let code = status.as_u16();
    let text = status_code_text(code);
    let code_str = if text.is_empty() {
        format!("{code}")
    } else {
        format!("{code} {text}")
    };
    match code {
        429 => {
            let hint = " — reduce request rate or wait before retrying";
            anyhow!(ProviderError::RateLimit(format!(
                "{provider_name} rate limited ({code_str}): {err_msg}{hint}"
            )))
        }
        401 | 403 => {
            let hint = " — check your API key in ~/.config/rem-cli/config.toml or the REMOTE_API_KEY env var";
            anyhow!(ProviderError::Auth(format!(
                "{provider_name} auth failed ({code_str}): {err_msg}{hint}"
            )))
        }
        500..=504 => anyhow!(ProviderError::ServerError(format!(
            "{provider_name} server error ({code_str}): {err_msg}"
        ))),
        _ => anyhow!(ProviderError::Other(format!(
            "{provider_name} API failed ({code_str}): {err_msg}"
        ))),
    }
}

pub(crate) async fn handle_ollama_error(resp: reqwest::Response, url: &str, model: &str) -> anyhow::Error {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_else(|e| format!("(read error: {e})"));
    let err_msg = serde_json::from_str::<LlmErrorResponse>(&body)
        .map(|v| v.error.to_string())
        .unwrap_or_else(|_| body.clone());
    let code = status.as_u16();
    if code == 404 && err_msg.to_lowercase().contains("model") {
        return anyhow!(ProviderError::Other(format!(
            "Model '{model}' not found. Pull it: `ollama pull {model}`"
        )));
    }
    if code == 404 {
        return anyhow!(ProviderError::Other(format!(
            "Endpoint not found (404 at {url}). Check --ollama-url"
        )));
    }
    if code == 429 {
        return anyhow!(ProviderError::RateLimit(format!(
            "Ollama rate limited ({code}): {err_msg}"
        )));
    }
    if (500..=504).contains(&code) {
        return anyhow!(ProviderError::ServerError(format!(
            "Ollama server error ({code}): {err_msg}"
        )));
    }
    anyhow!(ProviderError::Other(format!("Ollama failed: {status} — {err_msg}")))
}

pub(crate) fn parse_json_fallback(text: &str) -> Result<crate::ModelReply> {
    match serde_json::from_str::<crate::ModelReply>(text.trim()) {
        Ok(parsed) => Ok(parsed),
        Err(e) => {
            tracing::warn!("JSON parse failed — falling back: {e}");
            Ok(crate::ModelReply::fallback(text.trim()))
        }
    }
}

fn redact_api_key(msg: &str, api_key: Option<&str>) -> String {
    let mut s = msg.to_string();
    if let Some(key) = api_key {
        if !key.is_empty() && s.contains(key) {
            s = s.replace(key, "***");
        }
    }
    s
}

fn status_code_text(code: u16) -> &'static str {
    match code {
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}
