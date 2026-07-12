use std::collections::{HashMap, VecDeque};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::model::Message;
use crate::provider::{Provider, ProviderEvent};
use crate::tool::ToolDefinition;

#[derive(Debug, Clone, Default)]
pub struct MockProvider {
    events: VecDeque<ProviderEvent>,
}

impl MockProvider {
    pub fn new(events: Vec<ProviderEvent>) -> Self {
        MockProvider {
            events: events.into_iter().collect(),
        }
    }
}

#[async_trait]
impl Provider for MockProvider {
    async fn stream(
        &mut self,
        tx: mpsc::Sender<ProviderEvent>,
        _ctx: Vec<Message>,
        _tool_schemas: HashMap<String, ToolDefinition>,
    ) -> Result<()> {
        for event in self.events.drain(..) {
            tx.send(event).await?;
        }
        Ok(())
    }

    fn clone_dyn(&self) -> Box<dyn Provider> {
        Box::new(self.clone()) // Forward to the derive(Clone) impl
    }
}

#[cfg(test)]
mod tests {
    use crate::{context::Context, model::ToolCall, tool::Toolbox};

    use super::*;

    #[tokio::test]
    async fn test_mock_provider() {
        let turns = vec![
            ProviderEvent::Token("Hello".to_string()),
            ProviderEvent::ToolCalls(vec![ToolCall {
                id: "id1".to_string(),
                name: "tool1".to_string(),
                input: serde_json::Value::Null,
            }]),
        ];

        let mut provider = MockProvider::new(turns);
        let ctx = Context::new("", None);
        let tools = Toolbox::<()>::new();
        let (tx, mut rx) = mpsc::channel(256);

        tokio::spawn(async move {
            provider
                .stream(tx, ctx.as_messages(), tools.schemas())
                .await
                .unwrap();
        });

        let received = rx.recv().await;
        assert_eq!(received, Some(ProviderEvent::Token("Hello".into())));

        let received = rx.recv().await;
        assert_eq!(
            received,
            Some(ProviderEvent::ToolCalls(vec![ToolCall {
                id: "id1".to_string(),
                name: "tool1".to_string(),
                input: serde_json::Value::Null,
            }]))
        );

        let closed_msg = rx.recv().await;
        assert!(closed_msg.is_none());
    }
}
