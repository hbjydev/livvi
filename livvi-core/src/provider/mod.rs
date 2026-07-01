use anyhow::Result;
use async_trait::async_trait;

use crate::model::Transcript;

mod mock;
pub use mock::MockProvider;

#[derive(Debug, Clone)]
pub enum ProviderResponse {
    ToolCall {
        tool_name: String,
        tool_args: String,
        tool_call_id: String,
    },
    Text(String),
}

#[async_trait]
pub trait Provider: Send + Sync + 'static {
    async fn complete(&mut self, transcript: Transcript) -> Result<ProviderResponse>;
}
