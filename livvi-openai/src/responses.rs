use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use livvi_core::{
    model::{Message, Role, ToolCall, Usage},
    provider::{Provider, ProviderEvent},
    tool::ToolDefinition,
};
use openai_api_rs::v1::{
    api::OpenAIClient,
    responses::responses_stream::{
        CreateResponseStreamRequest, ResponseStreamEvent, ResponseStreamResponse,
    },
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::common::tool_to_function;

pub struct OpenAIResponsesProvider {
    api_key: String,
    api_url: String,

    client: OpenAIClient,
    model_name: String,
}

impl Clone for OpenAIResponsesProvider {
    fn clone(&self) -> Self {
        OpenAIResponsesProvider::new(&self.api_key, &self.api_url, &self.model_name)
            .expect("Failed to clone OpenAIResponsesProvider")
    }
}

impl OpenAIResponsesProvider {
    pub fn new(api_key: &str, api_url: &str, model_name: &str) -> Result<Self> {
        let client = OpenAIClient::builder()
            .with_endpoint(api_url.to_string())
            .with_api_key(api_key.to_string())
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create OpenAI client: {}", e))?;

        Ok(OpenAIResponsesProvider {
            client,
            api_url: api_url.to_string(),
            api_key: api_key.to_string(),
            model_name: model_name.to_string(),
        })
    }
}

#[async_trait]
impl Provider for OpenAIResponsesProvider {
    fn clone_dyn(&self) -> Box<dyn Provider> {
        Box::new(self.clone())
    }

    #[tracing::instrument(
        skip(self, tx, messages, tools),
        fields(
            otel.name = tracing::field::Empty,
            gen_ai.operation.name = "chat",
            gen_ai.request.model = self.model_name,
            gen_ai.request.stream = true,
        )
    )]
    async fn stream(
        &mut self,
        tx: mpsc::Sender<ProviderEvent>,
        messages: Vec<Message>,
        tools: HashMap<String, ToolDefinition>,
    ) -> Result<()> {
        let mut input_items = vec![];
        for item in messages {
            input_items.extend(into_openai(item)?);
        }

        let mut tool_items = vec![];
        for tool in tools.into_values() {
            tool_items.push(tool_to_responses(tool));
        }

        let mut req = CreateResponseStreamRequest::new();
        req.model = Some(self.model_name.clone());
        req.input = Some(input_items.into());
        req.tools = Some(tool_items);

        let mut openai_stream: BoxStream<'static, ResponseStreamResponse> = Box::pin(
            self.client
                .create_response_stream(req)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to create response stream: {}", e))?,
        );

        let mut accumulators: HashMap<String, ToolCallAccumulator> = HashMap::new();

        while let Some(response) = openai_stream.next().await {
            match response {
                ResponseStreamResponse::Event(evt) => {
                    let events = provider_events_from_openai(evt, &mut accumulators)?;
                    for event in events {
                        if tx.send(event).await.is_err() {
                            return Ok(());
                        }
                    }
                }
                ResponseStreamResponse::Done => break,
            }
        }

        // Emit any tool calls that were completed during the stream.
        let mut completed_calls = vec![];
        for (id, acc) in accumulators {
            if let Some(name) = acc.name {
                let input = serde_json::from_str(&acc.arguments).unwrap_or_else(|_| json!({}));
                completed_calls.push(ToolCall { id, name, input });
            }
        }
        if !completed_calls.is_empty() {
            let _ = tx.send(ProviderEvent::ToolCalls(completed_calls)).await;
        }

        Ok(())
    }
}

#[derive(Debug, Default)]
struct ToolCallAccumulator {
    name: Option<String>,
    arguments: String,
}

fn tool_to_responses(tool: livvi_core::tool::ToolDefinition) -> openai_api_rs::v1::types::Tools {
    openai_api_rs::v1::types::Tools::Function(openai_api_rs::v1::types::ToolsFunction {
        function: tool_to_function(tool),
    })
}

#[tracing::instrument]
fn provider_events_from_openai(
    evt: ResponseStreamEvent,
    accumulators: &mut HashMap<String, ToolCallAccumulator>,
) -> Result<Vec<ProviderEvent>> {
    let mut events = vec![];

    match evt.event.as_deref() {
        Some("response.output_text.delta") => {
            if let Some(delta) = evt.data.get("delta").and_then(|v| v.as_str()) {
                events.push(ProviderEvent::Token(delta.to_string()));
            }
        }
        Some("response.reasoning.summary_text.delta") => {
            if let Some(delta) = evt.data.get("delta").and_then(|v| v.as_str()) {
                events.push(ProviderEvent::ThinkingToken(delta.to_string()));
            }
        }
        Some("response.output_item.added") => {
            if evt.data.get("type").and_then(|v| v.as_str()) == Some("function_call") {
                let id = evt
                    .data
                    .get("call_id")
                    .or_else(|| evt.data.get("id"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("function_call missing call_id/id"))?
                    .to_string();
                let name = evt
                    .data
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("function_call missing name"))?
                    .to_string();

                let acc = accumulators.entry(id).or_default();
                acc.name = Some(name);

                events.push(ProviderEvent::ToolCallStarted);
            }
        }
        Some("response.function_call_arguments.delta") => {
            let id = evt
                .data
                .get("call_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("function_call_arguments.delta missing call_id"))?
                .to_string();
            let delta = evt.data.get("delta").and_then(|v| v.as_str()).unwrap_or("");

            if let Some(acc) = accumulators.get_mut(&id) {
                acc.arguments.push_str(delta);
            }
        }
        Some("response.output_item.done") => {
            if evt.data.get("type").and_then(|v| v.as_str()) == Some("function_call") {
                let id = evt
                    .data
                    .get("call_id")
                    .or_else(|| evt.data.get("id"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("function_call done missing call_id/id"))?
                    .to_string();

                if let Some(acc) = accumulators.remove(&id)
                    && let Some(name) = acc.name
                {
                    let input = serde_json::from_str(&acc.arguments).unwrap_or_else(|_| json!({}));
                    events.push(ProviderEvent::ToolCalls(vec![ToolCall { id, name, input }]));
                }
            }
        }
        Some("response.completed") => {
            if let Some(usage) = evt.data.get("usage") {
                let (input_tokens, output_tokens, reasoning_tokens, cached_tokens, uncached_tokens) =
                    parse_usage(usage);
                events.push(ProviderEvent::Usage(Usage {
                    input_tokens,
                    output_tokens,
                    reasoning_tokens,
                    cached_tokens,
                    uncached_tokens,
                    prompt_processing_ms: 0,
                    generation_ms: 0,
                }));
            }
        }
        Some("response.incomplete") | Some("response.failed") => {
            // Stream ends; the caller observes completion via the stream closing.
        }
        _ => {}
    }

    Ok(events)
}

fn parse_usage(value: &Value) -> (usize, usize, usize, usize, usize) {
    let input_tokens = value
        .get("input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let output_tokens = value
        .get("output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let reasoning_tokens = value
        .get("output_tokens_details")
        .and_then(|d| d.as_object())
        .and_then(|d| d.get("reasoning"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let cached_tokens = value
        .get("input_tokens_details")
        .and_then(|d| d.as_object())
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let uncached_tokens = input_tokens.saturating_sub(cached_tokens);
    (
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cached_tokens,
        uncached_tokens,
    )
}

fn into_openai(msg: Message) -> Result<Vec<Value>> {
    let mut items = vec![];

    match msg.role {
        Role::Tool => {
            if let (Some(content), Some(tool_call_id)) = (msg.content, msg.tool_call_id) {
                items.push(json!({
                    "type": "function_call_output",
                    "call_id": tool_call_id,
                    "output": content,
                }));
            }
        }
        Role::Assistant => {
            if let Some(calls) = msg.tool_calls {
                for call in calls {
                    items.push(json!({
                        "type": "function_call",
                        "id": call.id,
                        "name": call.name,
                        "arguments": call.input,
                    }));
                }
            }
            if let Some(content) = msg.content {
                items.push(json!({
                    "role": "assistant",
                    "content": content,
                }));
            }
        }
        Role::User | Role::System => {
            if let Some(content) = msg.content {
                items.push(json!({
                    "role": msg.role.to_string(),
                    "content": content,
                }));
            }
        }
    }

    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use livvi_core::model::ToolCall;
    use serde_json::json;

    fn stream_event(event: &str, data: Value) -> ResponseStreamEvent {
        ResponseStreamEvent {
            event: Some(event.to_string()),
            data,
        }
    }

    #[test]
    fn into_openai_user_message_is_not_empty() {
        let msg = Message::user("Hello, world!", None);
        let items = into_openai(msg).unwrap();
        assert_eq!(
            items,
            vec![json!({"role": "user", "content": "Hello, world!"})]
        );
    }

    #[test]
    fn into_openai_system_message() {
        let msg = Message::system("Be helpful.");
        let items = into_openai(msg).unwrap();
        assert_eq!(
            items,
            vec![json!({"role": "system", "content": "Be helpful."})]
        );
    }

    #[test]
    fn into_openai_assistant_text() {
        let msg = Message::assistant("Hi there.", None::<&str>);
        let items = into_openai(msg).unwrap();
        assert_eq!(
            items,
            vec![json!({"role": "assistant", "content": "Hi there."})]
        );
    }

    #[test]
    fn into_openai_function_call_uses_id() {
        let msg = Message::with_tool_calls(
            vec![ToolCall {
                name: "calc".to_string(),
                id: "call-1".to_string(),
                input: json!({"a": 2, "b": 2}),
            }],
            None::<&str>,
            None::<&str>,
        );
        let items = into_openai(msg).unwrap();
        assert_eq!(
            items,
            vec![json!({
                "type": "function_call",
                "id": "call-1",
                "name": "calc",
                "arguments": {"a": 2, "b": 2}
            })]
        );
    }

    #[test]
    fn into_openai_function_call_output() {
        let msg = Message::tool_result("call-1", "4");
        let items = into_openai(msg).unwrap();
        assert_eq!(
            items,
            vec![json!({
                "type": "function_call_output",
                "call_id": "call-1",
                "output": "4"
            })]
        );
    }

    #[test]
    fn from_openai_text_response() {
        let events = provider_events_from_openai(
            stream_event(
                "response.output_text.delta",
                json!({"delta": "Hello back!"}),
            ),
            &mut HashMap::new(),
        )
        .unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            ProviderEvent::Token(text) if text == "Hello back!"
        ));

        let done = provider_events_from_openai(
            stream_event(
                "response.completed",
                json!({
                    "stop_reason": "end_turn",
                    "usage": {"input_tokens": 10, "output_tokens": 5}
                }),
            ),
            &mut HashMap::new(),
        )
        .unwrap();
        assert_eq!(done.len(), 1);
        assert!(matches!(
            &done[0],
            ProviderEvent::Usage(usage) if usage.input_tokens == 10 && usage.output_tokens == 5
        ));
    }

    #[test]
    fn from_openai_tool_call_response() {
        let mut accumulators = HashMap::new();

        let start = provider_events_from_openai(
            stream_event(
                "response.output_item.added",
                json!({
                    "type": "function_call",
                    "call_id": "call-1",
                    "name": "calc",
                    "arguments": ""
                }),
            ),
            &mut accumulators,
        )
        .unwrap();
        assert!(matches!(&start[0], ProviderEvent::ToolCallStarted));

        let delta = provider_events_from_openai(
            stream_event(
                "response.function_call_arguments.delta",
                json!({"call_id": "call-1", "delta": "{\"a\":2,\"b\":2}"}),
            ),
            &mut accumulators,
        )
        .unwrap();
        assert!(delta.is_empty());

        let done = provider_events_from_openai(
            stream_event(
                "response.output_item.done",
                json!({"type": "function_call", "call_id": "call-1"}),
            ),
            &mut accumulators,
        )
        .unwrap();
        assert!(matches!(
            &done[0],
            ProviderEvent::ToolCalls(calls) if calls.len() == 1 && calls[0].id == "call-1" && calls[0].name == "calc"
        ));
    }

    #[test]
    fn from_openai_tool_call_falls_back_to_id() {
        let mut accumulators = HashMap::new();
        let start = provider_events_from_openai(
            stream_event(
                "response.output_item.added",
                json!({
                    "type": "function_call",
                    "id": "call-1",
                    "name": "calc",
                    "arguments": ""
                }),
            ),
            &mut accumulators,
        )
        .unwrap();
        assert!(matches!(&start[0], ProviderEvent::ToolCallStarted));
    }

    #[test]
    fn from_openai_reasoning_response() {
        let events = provider_events_from_openai(
            stream_event(
                "response.reasoning.summary_text.delta",
                json!({"delta": "Thinking..."}),
            ),
            &mut HashMap::new(),
        )
        .unwrap();
        assert!(matches!(
            &events[0],
            ProviderEvent::ThinkingToken(text) if text == "Thinking..."
        ));

        let done = provider_events_from_openai(
            stream_event(
                "response.completed",
                json!({
                    "stop_reason": "end_turn",
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 5,
                        "output_tokens_details": {"reasoning": 3}
                    }
                }),
            ),
            &mut HashMap::new(),
        )
        .unwrap();
        assert!(matches!(
            &done[0],
            ProviderEvent::Usage(usage) if usage.input_tokens == 10 && usage.output_tokens == 5 && usage.reasoning_tokens == 3
        ));
    }
}
