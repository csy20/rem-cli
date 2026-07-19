use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use reqwest::Client;
use tokio::time::timeout;

use crate::constants::{INITIAL_BUF_CAPACITY, MAX_RESPONSE_BYTES, STREAM_CHUNK_TIMEOUT};
use crate::provider::provider_error::ProviderError;

pub(crate) static STREAM_CANCELLED: AtomicBool = AtomicBool::new(false);
pub(crate) static STREAM_TOKENS: AtomicBool = AtomicBool::new(false);
pub(crate) static HAD_STREAMING_OUTPUT: AtomicBool = AtomicBool::new(false);

pub(crate) static HTTP_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .pool_max_idle_per_host(4)
        .build()
        .unwrap_or_else(|_| Client::new())
});

pub(crate) async fn stream_buf<F>(resp: reqwest::Response, mut on_line: F) -> Result<()>
where
    F: FnMut(&str) -> Result<bool>,
{
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::with_capacity(INITIAL_BUF_CAPACITY);
    let mut offset = 0usize;
    loop {
        if STREAM_CANCELLED.load(Ordering::Relaxed) {
            return Err(anyhow!(ProviderError::Cancelled));
        }
        let chunk = match timeout(STREAM_CHUNK_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(c))) => c,
            Ok(Some(Err(e))) => {
                return Err(anyhow!(ProviderError::Other(format!("stream read error: {e}"))));
            }
            Ok(None) => break,
            Err(_) => {
                return Err(anyhow!(ProviderError::Timeout(
                    "stream timed out (no data for 60s)".into()
                )));
            }
        };
        if offset > 0 && buf.len() > INITIAL_BUF_CAPACITY * 2 {
            buf = buf.split_off(offset);
            offset = 0;
        }
        if buf.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(anyhow!(ProviderError::ResponseTooLarge(MAX_RESPONSE_BYTES as u64)));
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
    let mut full = String::with_capacity(INITIAL_BUF_CAPACITY);
    stream_buf(resp, |trimmed| {
        if !on_line(trimmed, &mut full)? {
            return Ok(false);
        }
        Ok(true)
    })
    .await?;
    Ok(full)
}

pub(crate) async fn stream_sse_response(resp: reqwest::Response) -> Result<String> {
    stream_lines(resp, |trimmed, full| {
        if trimmed.as_bytes().starts_with(b"data: ") {
            let data = &trimmed[6..];
            if data == "[DONE]" {
                return Ok(false);
            }
            if let Some(content) = extract_sse_delta_content(data) {
                if !content.is_empty() {
                    full.push_str(content);
                    emit_token(content);
                    if full.len() > MAX_RESPONSE_BYTES {
                        return Err(anyhow!(ProviderError::ResponseTooLarge(MAX_RESPONSE_BYTES as u64)));
                    }
                }
            }
        }
        Ok(true)
    })
    .await
}

pub(crate) async fn stream_anthropic_sse(
    resp: reqwest::Response,
    last_usage: &std::sync::Mutex<crate::provider::anthropic::AnthropicUsage>,
    show_reasoning: bool,
) -> Result<String> {
    stream_lines(resp, |trimmed, full| {
        if trimmed.starts_with("event: ") {
            return Ok(true);
        }
        if let Some(data) = trimmed.strip_prefix("data: ") {
            if let Ok(chunk) = serde_json::from_str::<crate::provider::anthropic::AnthropicStreamChunk>(data) {
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
                            } else if let Some(ref thinking) = delta.thinking {
                                full.push_str(thinking);
                                emit_token(thinking);
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
                    return Err(anyhow!(ProviderError::ResponseTooLarge(MAX_RESPONSE_BYTES as u64)));
                }
            }
        }
        Ok(true)
    })
    .await
}

pub(crate) async fn stream_gemini_sse(resp: reqwest::Response) -> Result<String> {
    stream_lines(resp, |trimmed, full| {
        if trimmed.is_empty() || trimmed.starts_with(':') {
            return Ok(true);
        }
        if let Some(data) = trimmed.strip_prefix("data: ") {
            if let Ok(chunk) = serde_json::from_str::<crate::provider::gemini::GeminiStreamChunk>(data) {
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
                        return Err(anyhow!(ProviderError::ResponseTooLarge(MAX_RESPONSE_BYTES as u64)));
                    }
                }
            }
        }
        Ok(true)
    })
    .await
}

pub(crate) async fn stream_openai_tool_response(
    resp: reqwest::Response,
) -> Result<crate::provider::tools::ToolResponse> {
    let mut full_text = String::with_capacity(INITIAL_BUF_CAPACITY);
    let mut tool_acc = crate::provider::openai::AccumulatedToolCalls::default();
    stream_buf(resp, |trimmed| {
        if !trimmed.starts_with("data: ") {
            return Ok(true);
        }
        let data = &trimmed[6..];
        if data == "[DONE]" {
            return Ok(false);
        }

        // Lightweight path for content-only chunks (most common)
        if let Some(content) = extract_sse_delta_content(data) {
            full_text.push_str(content);
            emit_token(content);
        }

        // Only do full deserialization when tool calls may be present
        if data.contains("\"tool_calls\"") {
            if let Ok(chunk) = serde_json::from_str::<crate::provider::openai::OpenAIStreamChunk>(data) {
                if let Some(tool_calls) = chunk.choices.first().and_then(|c| c.delta.tool_calls.as_ref()) {
                    tool_acc.absorb_chunk(tool_calls);
                }
            }
        }

        if full_text.len() > MAX_RESPONSE_BYTES {
            return Err(anyhow!(ProviderError::ResponseTooLarge(MAX_RESPONSE_BYTES as u64)));
        }
        Ok(true)
    })
    .await?;
    if !tool_acc.is_empty() {
        Ok(crate::provider::tools::ToolResponse::ToolCalls(
            tool_acc.to_tool_calls(),
        ))
    } else {
        Ok(crate::provider::tools::ToolResponse::Text(full_text))
    }
}

/// Lightweight extraction of `choices[0].delta.content` from an SSE JSON line.
/// Avoids full `serde_json::from_str` deserialization overhead in the streaming hot path.
/// Falls back to `None` for edge cases (escaped quotes, non-string content).
fn extract_sse_delta_content(data: &str) -> Option<&str> {
    // Scan for `"content":"` to extract text content without full JSON parse
    let needle = "\"content\":\"";
    let start = data.find(needle)?;
    let content_start = start + needle.len();
    let mut escape = false;
    for (i, c) in data[content_start..].char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' {
            escape = true;
            continue;
        }
        if c == '"' {
            return Some(&data[content_start..content_start + i]);
        }
    }
    None
}

/// Applies lightweight inline Markdown formatting (code, bold, italic) using ANSI codes.
/// Writes directly to a `Write` sink to avoid per-token String allocation.
fn write_inline_markdown(text: &str, w: &mut impl Write) {
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '`' => {
                let mut code = String::new();
                loop {
                    match chars.next() {
                        Some('`') => break,
                        Some(c) => code.push(c),
                        None => break,
                    }
                }
                if code.is_empty() {
                    let _ = w.write_all(b"\x1b[32m``\x1b[0m");
                } else {
                    let _ = w.write_all(b"\x1b[32m");
                    let _ = w.write_all(code.as_bytes());
                    let _ = w.write_all(b"\x1b[0m");
                }
            }
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    let mut bold = String::new();
                    loop {
                        match chars.next() {
                            Some('*') if chars.peek() == Some(&'*') => {
                                chars.next();
                                let _ = w.write_all(b"\x1b[1m");
                                let _ = w.write_all(bold.as_bytes());
                                let _ = w.write_all(b"\x1b[0m");
                                break;
                            }
                            Some(c) => bold.push(c),
                            None => {
                                let _ = w.write_all(b"**");
                                let _ = w.write_all(bold.as_bytes());
                                break;
                            }
                        }
                    }
                } else {
                    let mut italic = String::new();
                    loop {
                        match chars.next() {
                            Some('*') => {
                                let _ = w.write_all(b"\x1b[3m");
                                let _ = w.write_all(italic.as_bytes());
                                let _ = w.write_all(b"\x1b[0m");
                                break;
                            }
                            Some(c) => italic.push(c),
                            None => {
                                let _ = w.write_all(b"*");
                                let _ = w.write_all(italic.as_bytes());
                                break;
                            }
                        }
                    }
                }
            }
            _ => {
                let mut buf = [0u8; 4];
                let encoded = c.encode_utf8(&mut buf);
                let _ = w.write_all(encoded.as_bytes());
            }
        }
    }
}

pub fn emit_token(text: &str) {
    if STREAM_TOKENS.load(Ordering::Relaxed) {
        HAD_STREAMING_OUTPUT.store(true, Ordering::Relaxed);
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        write_inline_markdown(text, &mut handle);
        if text.contains('\n') {
            let _ = handle.flush();
        }
    }
}
