use anyhow::Result;
use async_trait::async_trait;

use crate::{model::Transcript, tool::Tools};

mod mock;
pub use mock::MockProvider;

#[derive(Debug, Clone)]
pub struct ProviderResponseToolCall {
    pub tool_name: String,
    pub tool_args: serde_json::Value,
    pub tool_call_id: String,
}

#[derive(Debug, Clone)]
pub enum ProviderResponseValue {
    ToolCalls(Vec<ProviderResponseToolCall>),
    Text(String),
    Reasoning(String),
}

#[derive(Debug, Clone)]
pub struct ProviderResponse {
    pub value: ProviderResponseValue,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub reasoning_tokens: usize,
}

#[async_trait]
pub trait Provider<S: Send + Sync + 'static>: Send + Sync + 'static {
    async fn complete(
        &mut self,
        transcript: Transcript,
        tools: Tools<S>,
    ) -> Result<ProviderResponse>;
}
