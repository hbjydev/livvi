use anyhow::Result;
use async_trait::async_trait;

use crate::model::Message;

/// Strategy for summarizing a conversation segment into a compact string.
///
/// Implementers receive a complete prompt (system instruction plus context
/// messages) and return a non-streaming summary.
#[async_trait]
pub trait Summarizer: Send + Sync + 'static {
    /// Summarize the provided prompt into a string.
    async fn summarize(&self, prompt: Vec<Message>) -> Result<String>;
}
