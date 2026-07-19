use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::json;

use crate::provider::provider_error::{self, ProviderError};
use crate::provider::provider_stream;
use crate::provider::{ProviderContext, ProviderKind};

pub(crate) fn build_client(timeout_s: u64) -> Client {
    Client::builder()
        .pool_max_idle_per_host(4)
        .timeout(std::time::Duration::from_secs(timeout_s))
        .build()
        .unwrap_or_else(|_| provider_stream::HTTP_CLIENT.clone())
}

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

pub(crate) async fn openai_compat_list_models(ctx: &ProviderContext, provider_name: &str) -> Result<Vec<String>> {
    let url = openai_models_url(&ctx.base_url);
    let resp = add_openai_auth(ctx.client.get(&url), ctx.api_key_str(), ctx.kind)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!(ProviderError::Other(format!(
            "{provider_name} API unreachable at {}",
            ctx.base_url
        ))));
    }
    let parsed: crate::provider::openai::OpenAIModelsResponse = resp
        .json()
        .await
        .with_context(|| format!("invalid {provider_name} models response"))?;
    Ok(parsed.data.into_iter().map(|m| m.id).collect())
}

pub(crate) async fn openai_compat_complete_json(
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
        return Err(provider_error::parse_api_error(provider_name, resp, Some(ctx.api_key_str())).await);
    }
    let parsed: crate::provider::openai::OpenAIResponse = resp
        .json()
        .await
        .with_context(|| format!("invalid {provider_name} response"))?;
    let content = parsed
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or("");
    provider_error::parse_json_fallback(content)
}

pub(crate) async fn openai_compat_chat_stream(
    ctx: &ProviderContext,
    kind: ProviderKind,
    provider_name: &str,
    user_prompt: &str,
    system_prompt: &str,
    history: &str,
) -> Result<String> {
    let url = openai_chat_url(&ctx.base_url, kind, &ctx.model);
    let messages = build_messages_from_history(history, user_prompt, Some(system_prompt));
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
        return Err(provider_error::parse_api_error(provider_name, resp, Some(ctx.api_key_str())).await);
    }
    provider_stream::stream_sse_response(resp).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn openai_compat_chat_stream_with_vision(
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
    let mut messages = build_messages_from_history(history, "", Some(system_prompt));
    messages.pop();
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
        return Err(provider_error::parse_api_error(provider_name, resp, Some(ctx.api_key_str())).await);
    }
    provider_stream::stream_sse_response(resp).await
}

pub(crate) async fn openai_compat_chat_stream_with_tools(
    ctx: &ProviderContext,
    kind: ProviderKind,
    provider_name: &str,
    user_prompt: &str,
    system_prompt: &str,
    history: &str,
    tool_specs: &[crate::provider::tools::ToolSpec],
) -> Result<crate::provider::tools::ToolResponse> {
    let url = openai_chat_url(&ctx.base_url, kind, &ctx.model);
    let messages = build_messages_from_history(history, user_prompt, Some(system_prompt));
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
        return Err(provider_error::parse_api_error(provider_name, resp, Some(ctx.api_key_str())).await);
    }
    provider_stream::stream_openai_tool_response(resp).await
}

pub(crate) fn parse_history_turns(history: &str) -> Vec<(String, String)> {
    if history.is_empty() {
        return Vec::new();
    }
    let mut turns = Vec::new();
    for block in history.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        const BOUNDARY: &str = "\n<<<REM:BOUNDARY>>>\n";
        const BOUNDARY_STRIP: &str = "<<<REM:BOUNDARY>>>\n";
        let (user_part, assistant_part) = if let Some(rem_idx) = block.find(BOUNDARY) {
            let user = &block[..rem_idx];
            let assistant = &block[rem_idx + BOUNDARY.len()..];
            (user, assistant)
        } else if let Some(rem_idx) = block.strip_prefix(BOUNDARY_STRIP) {
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

pub(crate) fn build_messages_from_history(
    history: &str,
    user_prompt: &str,
    system_prompt: Option<&str>,
) -> Vec<serde_json::Value> {
    let mut messages: Vec<serde_json::Value> = Vec::new();
    if let Some(sp) = system_prompt {
        messages.push(json!({"role": "system", "content": sp}));
    }
    if !history.is_empty() {
        for (user_msg, assistant_msg) in parse_history_turns(history) {
            messages.push(json!({"role": "user", "content": user_msg}));
            if !assistant_msg.is_empty() {
                messages.push(json!({"role": "assistant", "content": assistant_msg}));
            }
        }
    }
    messages.push(json!({"role": "user", "content": user_prompt}));
    messages
}
