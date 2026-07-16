use anyhow::Result;
use async_trait::async_trait;

use super::tools::{ToolResponse, ToolSpec};
use super::{ProviderBackend, ProviderContext};

pub(super) struct OpenRouterBackend;

#[async_trait]
impl ProviderBackend for OpenRouterBackend {
    async fn list_models(&self, ctx: &ProviderContext) -> Result<Vec<String>> {
        super::openai_compat_list_models(ctx, "OpenRouter").await
    }

    async fn complete_json(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<crate::ModelReply> {
        super::openai_compat_complete_json(ctx, ctx.kind, "OpenRouter", system_prompt, user_prompt).await
    }

    async fn complete_chat_stream(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
    ) -> Result<String> {
        super::openai_compat_chat_stream(ctx, ctx.kind, "OpenRouter", user_prompt, system_prompt, history).await
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
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
        tool_specs: &[ToolSpec],
    ) -> Result<ToolResponse> {
        super::openai_compat_chat_stream_with_tools(
            ctx,
            ctx.kind,
            "OpenRouter",
            user_prompt,
            system_prompt,
            history,
            tool_specs,
        )
        .await
    }
}
