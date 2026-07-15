use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;

use super::tools::{ToolResponse, ToolSpec};
use super::{ProviderBackend, ProviderContext};

pub(super) struct GitHubBackend;

#[async_trait]
impl ProviderBackend for GitHubBackend {
    async fn list_models(&self, ctx: &ProviderContext) -> Result<Vec<String>> {
        let url = super::openai_models_url(&ctx.base_url);
        let resp = super::add_openai_auth(ctx.client.get(&url), ctx.api_key_str(), ctx.kind)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(anyhow!(super::ProviderError::Other(format!(
                "GitHub Models API unreachable at {}",
                ctx.base_url
            ))));
        }
        let parsed: super::openai::OpenAIModelsResponse =
            resp.json().await.context("invalid GitHub Models response")?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }

    async fn complete_json(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<crate::ModelReply> {
        super::openai_compat_complete_json(ctx, ctx.kind, "GitHub Models", system_prompt, user_prompt).await
    }

    async fn complete_chat_stream(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
    ) -> Result<String> {
        super::openai_compat_chat_stream(ctx, ctx.kind, "GitHub Models", user_prompt, system_prompt, history).await
    }

    async fn complete_chat_stream_with_vision(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
        mime_type: &str,
        base64_data: &str,
    ) -> Result<String> {
        super::openai_compat_chat_stream_with_vision(
            ctx,
            ctx.kind,
            "GitHub Models",
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
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
        tool_specs: &[ToolSpec],
    ) -> Result<ToolResponse> {
        super::openai_compat_chat_stream_with_tools(
            ctx,
            ctx.kind,
            "GitHub Models",
            user_prompt,
            system_prompt,
            history,
            tool_specs,
        )
        .await
    }
}
