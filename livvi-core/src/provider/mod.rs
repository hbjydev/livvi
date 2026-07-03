use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::context::Context;
use crate::model::{ToolCall, Usage};
use crate::tool::ToolDefinition;

mod mock;
pub use mock::MockProvider;

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderEvent {
    Token(String),
    ThinkingToken(String),
    Usage(Usage),
    ToolCalls(Vec<ToolCall>),
    ToolCallStarted,
}

#[async_trait]
pub trait Provider: Send + Sync + 'static {
    async fn stream(
        &mut self,
        tx: mpsc::Sender<ProviderEvent>,
        ctx: Context,
        tool_schemas: HashMap<String, ToolDefinition>,
    ) -> Result<()>;
}
