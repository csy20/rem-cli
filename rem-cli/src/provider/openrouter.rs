use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;

use super::tools::{ToolResponse, ToolSpec};
use super::{Provider, ProviderBackend};

pub(super) struct OpenRouterBackend;

#[async_trait]
impl ProviderBackend for OpenRouterBackend {
    async fn list_models(&self, provider: &Provider) -> Result<Vec<String>> {
        let url = provider.openai_models_url();
        let resp = provider.add_openai_auth(provider.client.get(&url)).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("OpenRouter API unreachable at {}", provider.base_url));
        }
        let parsed: super::openai::OpenAIModelsResponse =
            resp.json().await.context("invalid OpenRouter models response")?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }

    async fn complete_json(&self, provider: &Provider, user_prompt: &str) -> Result<crate::ModelReply> {
        provider.openai_compat_complete_json("OpenRouter", user_prompt).await
    }

    async fn complete_chat_stream(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        provider
            .openai_compat_chat_stream("OpenRouter", user_prompt, system_prompt, history)
            .await
    }

    async fn complete_chat_stream_with_vision(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        mime_type: &str,
        base64_data: &str,
    ) -> Result<String> {
        provider
            .openai_compat_chat_stream_with_vision(
                "OpenRouter",
                user_prompt,
                system_prompt,
                history,
                mime_type,
                base64_data,
            )
            .await
    }

    async fn complete_chat_stream_with_tools(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
        tool_specs: &[ToolSpec],
    ) -> Result<ToolResponse> {
        provider
            .openai_compat_chat_stream_with_tools("OpenRouter", user_prompt, system_prompt, history, tool_specs)
            .await
    }
}
