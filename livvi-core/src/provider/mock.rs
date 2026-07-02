use std::collections::VecDeque;

use anyhow::Result;
use async_trait::async_trait;
use futures::stream::{self};

use crate::{
    model::Transcript,
    provider::{Provider, ProviderEvent, ProviderStream},
    tool::Tools,
};

#[derive(Debug, Clone, Default)]
pub struct MockProvider {
    turns: VecDeque<Vec<ProviderEvent>>,
}

impl MockProvider {
    pub fn new(turns: Vec<Vec<ProviderEvent>>) -> Self {
        MockProvider {
            turns: turns.into_iter().collect(),
        }
    }
}

#[async_trait]
impl<S: Send + Sync + 'static> Provider<S> for MockProvider {
    async fn stream(
        &mut self,
        _transcript: Transcript,
        _tools: Tools<S>,
    ) -> Result<ProviderStream> {
        let events = self.turns.pop_front().unwrap_or_default();
        Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
    }
}

#[cfg(test)]
mod tests {
    use futures::stream::StreamExt;

    use crate::provider::FinishReason;

    use super::*;

    #[tokio::test]
    async fn test_mock_provider() {
        let turns = vec![
            vec![
                ProviderEvent::TextDelta("Hello".to_string()),
                ProviderEvent::Done {
                    reason: FinishReason::EndTurn,
                },
            ],
            vec![
                ProviderEvent::ToolCallStart {
                    id: "id1".to_string(),
                    name: "tool1".to_string(),
                },
                ProviderEvent::ToolCallDelta {
                    id: "id1".to_string(),
                    arguments: "{\"expr\":\"hello\"}".to_string(),
                },
                ProviderEvent::ToolCallDone {
                    id: "id1".to_string(),
                },
                ProviderEvent::Done {
                    reason: FinishReason::ToolCalls,
                },
            ],
        ];

        let mut provider = MockProvider::new(turns);
        let transcript = Transcript::new();
        let tools = Tools::<()>::new();

        let response1: Vec<_> = provider
            .stream(transcript.clone(), tools.clone())
            .await
            .unwrap()
            .collect()
            .await;
        assert_eq!(response1.len(), 2);
        assert!(matches!(
            response1[0].as_ref().unwrap(),
            ProviderEvent::TextDelta(_)
        ));

        let response2: Vec<_> = provider
            .stream(transcript, tools.clone())
            .await
            .unwrap()
            .collect()
            .await;
        assert_eq!(response2.len(), 4);
        assert!(matches!(
            response2.last().unwrap().as_ref().unwrap(),
            ProviderEvent::Done {
                reason: FinishReason::ToolCalls,
            }
        ));

        let empty: Vec<_> = provider
            .stream(Transcript::new(), tools)
            .await
            .unwrap()
            .collect()
            .await;
        assert!(empty.is_empty());
    }
}
