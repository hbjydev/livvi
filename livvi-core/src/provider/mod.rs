use anyhow::Result;
use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::{model::Transcript, tool::Tools};

mod mock;
pub use mock::MockProvider;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    EndTurn,
    ToolCalls,
    MaxTokens,
    ContentFilter,
    Incomplete,
    Other(String),
}

#[derive(Debug, Clone)]
pub enum ProviderEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallDelta {
        id: String,
        arguments: String,
    },
    ToolCallDone {
        id: String,
    },
    Usage {
        input_tokens: usize,
        output_tokens: usize,
        reasoning_tokens: usize,
    },
    Done {
        reason: FinishReason,
    },
}

pub type ProviderStream = BoxStream<'static, Result<ProviderEvent>>;

#[async_trait]
pub trait Provider<S: Send + Sync + 'static>: Send + Sync + 'static {
    async fn stream(&mut self, transcript: Transcript, tools: Tools<S>) -> Result<ProviderStream>;
}
