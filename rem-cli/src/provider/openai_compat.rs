use anyhow::Result;
use async_trait::async_trait;

use super::tools::{ToolResponse, ToolSpec};
use super::{ProviderBackend, ProviderContext};

/// A single backend for all OpenAI-compatible providers (Azure, OpenRouter, GitHub, xAI).
/// Eliminates 4 nearly-identical backend files by parameterizing the display name.
pub(super) struct OpenAICompatBackend {
    pub(super) display_name: &'static str,
    pub(super) supports_list_models: bool,
}

#[async_trait]
impl ProviderBackend for OpenAICompatBackend {
    async fn list_models(&self, ctx: &ProviderContext) -> Result<Vec<String>> {
        if self.supports_list_models {
            super::openai_compat_list_models(ctx, self.display_name).await
        } else {
            Ok(vec![ctx.model.clone()])
        }
    }

    async fn complete_json(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<crate::ModelReply> {
        super::openai_compat_complete_json(ctx, ctx.kind, self.display_name, system_prompt, user_prompt).await
    }

    async fn complete_chat_stream(
        &self,
        ctx: &ProviderContext,
        system_prompt: &str,
        user_prompt: &str,
        history: &str,
    ) -> Result<String> {
        super::openai_compat_chat_stream(ctx, ctx.kind, self.display_name, user_prompt, system_prompt, history).await
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
            self.display_name,
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
            self.display_name,
            user_prompt,
            system_prompt,
            history,
            tool_specs,
        )
        .await
    }
}
