use anyhow::Result;
use async_trait::async_trait;

use crate::{model::Transcript, provider::{Provider, ProviderResponse}};

#[derive(Debug, Clone)]
pub struct MockProvider {
    responses: Vec<ProviderResponse>,
    index: usize,
}

impl MockProvider {
    pub fn new(responses: Vec<ProviderResponse>) -> Self {
        MockProvider { responses, index: 0 }
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        MockProvider {
            responses: vec![],
            index: 0,
        }
    }
}

#[async_trait]
impl Provider for MockProvider {
    async fn complete(&mut self, _transcript: Transcript) -> Result<ProviderResponse> {
        if self.index >= self.responses.len() {
            anyhow::bail!("Mock ran out of responses");
        }

        let response = self.responses[self.index].clone();
        self.index += 1;

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_mock_provider() {
        use super::*;

        let responses = vec![
            ProviderResponse::Text("Hello".to_string()),
            ProviderResponse::ToolCall {
                tool_name: "tool1".to_string(),
                tool_args: "arg1".to_string(),
                tool_call_id: "id1".to_string(),
            },
        ];

        let mut provider = MockProvider::new(responses);

        // Test that it returns the expected responses in order
        let transcript = Transcript::new();
        let response1 = provider.complete(transcript.clone()).await.unwrap();
        assert!(matches!(response1, ProviderResponse::Text(_)));

        let response2 = provider.complete(transcript).await.unwrap();
        assert!(matches!(response2, ProviderResponse::ToolCall { .. }));

        // Test that it runs out of responses
        let result = provider.complete(Transcript::new()).await;
        assert!(result.is_err());
    }
}
