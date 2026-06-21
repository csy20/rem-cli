use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;

use super::tools::{ToolResponse, ToolSpec};
use super::{Provider, ProviderBackend};

pub(super) struct BedrockBackend;

impl BedrockBackend {
    async fn bedrock_client(&self) -> Result<aws_sdk_bedrockruntime::Client> {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        Ok(aws_sdk_bedrockruntime::Client::new(&config))
    }
}

#[async_trait]
impl ProviderBackend for BedrockBackend {
    async fn list_models(&self, provider: &Provider) -> Result<Vec<String>> {
        Ok(vec![provider.model.clone()])
    }

    async fn complete_json(&self, provider: &Provider, user_prompt: &str) -> Result<crate::ModelReply> {
        let client = self.bedrock_client().await?;
        let system_content = aws_sdk_bedrockruntime::types::SystemContentBlock::Text(provider.system_prompt.clone());
        let content = aws_sdk_bedrockruntime::types::ContentBlock::Text(format!(
            "{}\n\nReturn JSON only. Respond with a valid JSON object.",
            user_prompt
        ));

        let msg = aws_sdk_bedrockruntime::types::Message::builder()
            .role(aws_sdk_bedrockruntime::types::ConversationRole::User)
            .content(content)
            .build()
            .map_err(|e| anyhow!("failed to build Bedrock message: {}", e))?;

        let resp = client
            .converse()
            .model_id(&provider.model)
            .system(system_content)
            .messages(msg)
            .inference_config(
                aws_sdk_bedrockruntime::types::InferenceConfiguration::builder()
                    .max_tokens(512)
                    .temperature(0.3)
                    .build(),
            )
            .send()
            .await
            .context("failed to call Bedrock Converse API")?;

        let text = resp
            .output()
            .and_then(|o| {
                if let aws_sdk_bedrockruntime::types::ConverseOutput::Message(msg) = o {
                    Some(msg)
                } else {
                    None
                }
            })
            .map(|m| m.content())
            .and_then(|c| c.first())
            .and_then(|b| {
                if let aws_sdk_bedrockruntime::types::ContentBlock::Text(t) = b {
                    Some(t.as_str())
                } else {
                    None
                }
            })
            .unwrap_or("")
            .to_string();

        Provider::parse_json_fallback(&text)
    }

    async fn complete_chat_stream(
        &self,
        provider: &Provider,
        user_prompt: &str,
        system_prompt: &str,
        history: &str,
    ) -> Result<String> {
        let client = self.bedrock_client().await?;
        let system_content = aws_sdk_bedrockruntime::types::SystemContentBlock::Text(system_prompt.to_string());

        let mut prompt = String::new();
        if !history.is_empty() {
            prompt.push_str(&format!("[Previous conversation]:\n{}\n\n", history));
        }
        prompt.push_str(user_prompt);
        let content = aws_sdk_bedrockruntime::types::ContentBlock::Text(prompt);

        let msg = aws_sdk_bedrockruntime::types::Message::builder()
            .role(aws_sdk_bedrockruntime::types::ConversationRole::User)
            .content(content)
            .build()
            .map_err(|e| anyhow!("failed to build Bedrock message: {}", e))?;

        let output = client
            .converse_stream()
            .model_id(&provider.model)
            .system(system_content)
            .messages(msg)
            .inference_config(
                aws_sdk_bedrockruntime::types::InferenceConfiguration::builder()
                    .max_tokens(4096)
                    .temperature(0.7)
                    .build(),
            )
            .send()
            .await
            .context("failed to call Bedrock ConverseStream API")?;

        let mut full_text = String::with_capacity(4096);
        let mut event_stream = output.stream;

        use aws_sdk_bedrockruntime::types::ConverseStreamOutput;
        loop {
            if super::STREAM_CANCELLED.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            match event_stream.recv().await {
                Ok(Some(ConverseStreamOutput::ContentBlockDelta(delta))) => {
                    if let Some(aws_sdk_bedrockruntime::types::ContentBlockDelta::Text(t)) = delta.delta() {
                        full_text.push_str(t);
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(e) => return Err(anyhow!("Bedrock stream error: {}", e)),
            }
            if full_text.len() > super::MAX_RESPONSE_BYTES {
                return Err(anyhow!("response too large ({} bytes)", super::MAX_RESPONSE_BYTES));
            }
        }
        Ok(full_text)
    }

    async fn complete_chat_stream_with_vision(
        &self,
        _provider: &Provider,
        _user_prompt: &str,
        _system_prompt: &str,
        _history: &str,
        _mime_type: &str,
        _base64_data: &str,
    ) -> Result<String> {
        Err(anyhow!("Vision not yet supported for AWS Bedrock"))
    }

    async fn complete_chat_stream_with_tools(
        &self,
        _provider: &Provider,
        _user_prompt: &str,
        _system_prompt: &str,
        _history: &str,
        _tool_specs: &[ToolSpec],
    ) -> Result<ToolResponse> {
        Err(anyhow!("Tool calling not yet supported for AWS Bedrock"))
    }
}
