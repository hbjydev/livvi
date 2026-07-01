use anyhow::Result;
use async_trait::async_trait;

use crate::{
    model::Transcript,
    provider::{Provider, ProviderResponse},
};

#[derive(Debug, Clone, Default)]
pub struct MockProvider {
    responses: Vec<ProviderResponse>,
    index: usize,
}

impl MockProvider {
    pub fn new(responses: Vec<ProviderResponse>) -> Self {
        MockProvider {
            responses,
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
    use crate::provider::{ProviderResponseToolCall, ProviderResponseValue};

    #[tokio::test]
    async fn test_mock_provider() {
        use super::*;

        let responses = vec![
            ProviderResponse {
                value: ProviderResponseValue::Text("Hello".to_string()),
                input_tokens: 0,
                output_tokens: 0,
                reasoning_tokens: 0,
            },
            ProviderResponse {
                value: ProviderResponseValue::ToolCalls(vec![ProviderResponseToolCall {
                    tool_name: "tool1".to_string(),
                    tool_args: serde_json::json!({"expr": "hello"}),
                    tool_call_id: "id1".to_string(),
                }]),
                input_tokens: 0,
                output_tokens: 0,
                reasoning_tokens: 0,
            },
        ];

        let mut provider = MockProvider::new(responses);

        // Test that it returns the expected responses in order
        let transcript = Transcript::new();
        let response1 = provider.complete(transcript.clone()).await.unwrap();
        assert!(matches!(response1.value, ProviderResponseValue::Text(_)));

        let response2 = provider.complete(transcript).await.unwrap();
        assert!(matches!(
            response2.value,
            ProviderResponseValue::ToolCalls(..)
        ));

        // Test that it runs out of responses
        let result = provider.complete(Transcript::new()).await;
        assert!(result.is_err());
    }
}
