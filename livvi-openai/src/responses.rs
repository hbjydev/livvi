use std::collections::VecDeque;

use anyhow::Result;
use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use livvi_core::{
    model::{Transcript, TranscriptItem},
    provider::{FinishReason, Provider, ProviderEvent, ProviderStream},
    tool::Tools,
};
use openai_api_rs::v1::{
    api::OpenAIClient,
    responses::responses_stream::{
        CreateResponseStreamRequest, ResponseStreamEvent, ResponseStreamResponse,
    },
};
use serde_json::{Value, json};

use crate::common::tool_to_function;

pub struct OpenAIResponsesProvider {
    client: OpenAIClient,
    model_name: String,
}

impl OpenAIResponsesProvider {
    pub fn new(api_key: &str, api_url: &str, model_name: &str) -> Result<Self> {
        let client = OpenAIClient::builder()
            .with_endpoint(api_url)
            .with_api_key(api_key)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create OpenAI client: {}", e))?;

        Ok(OpenAIResponsesProvider {
            client,
            model_name: model_name.to_string(),
        })
    }
}

#[async_trait]
impl<S: Send + Sync + 'static> Provider<S> for OpenAIResponsesProvider {
    async fn stream(&mut self, transcript: Transcript, tools: Tools<S>) -> Result<ProviderStream> {
        let mut input_items = vec![];
        for item in transcript.items() {
            input_items.extend(into_openai(item)?);
        }

        let mut tool_items = vec![];
        for tool in tools.schemas() {
            tool_items.push(tool_to_responses(tool));
        }

        let mut req = CreateResponseStreamRequest::new();
        req.model = Some(self.model_name.clone());
        req.input = Some(input_items.into());
        req.tools = Some(tool_items);

        let openai_stream: BoxStream<'static, ResponseStreamResponse> = Box::pin(
            self.client
                .create_response_stream(req)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to create response stream: {}", e))?,
        );

        let events = stream::unfold(
            (openai_stream, VecDeque::new()),
            |(mut openai_stream, mut buffer)| async move {
                loop {
                    if let Some(event) = buffer.pop_front() {
                        return Some((event, (openai_stream, buffer)));
                    }

                    match openai_stream.next().await {
                        Some(ResponseStreamResponse::Event(evt)) => {
                            match provider_events_from_openai(evt) {
                                Ok(events) => {
                                    for event in events {
                                        buffer.push_back(Ok(event));
                                    }
                                }
                                Err(e) => return Some((Err(e), (openai_stream, buffer))),
                            }
                        }
                        Some(ResponseStreamResponse::Done) | None => return None,
                    }
                }
            },
        );

        Ok(Box::pin(events))
    }
}

fn tool_to_responses(tool: livvi_core::tool::ToolDefinition) -> openai_api_rs::v1::types::Tools {
    openai_api_rs::v1::types::Tools::Function(openai_api_rs::v1::types::ToolsFunction {
        function: tool_to_function(tool),
    })
}

fn provider_events_from_openai(evt: ResponseStreamEvent) -> Result<Vec<ProviderEvent>> {
    let mut events = vec![];

    match evt.event.as_deref() {
        Some("response.output_text.delta") => {
            if let Some(delta) = evt.data.get("delta").and_then(|v| v.as_str()) {
                events.push(ProviderEvent::TextDelta(delta.to_string()));
            }
        }
        Some("response.reasoning.summary_text.delta") => {
            if let Some(delta) = evt.data.get("delta").and_then(|v| v.as_str()) {
                events.push(ProviderEvent::ReasoningDelta(delta.to_string()));
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
                events.push(ProviderEvent::ToolCallStart { id, name });
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
            events.push(ProviderEvent::ToolCallDelta {
                id,
                arguments: delta.to_string(),
            });
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
                events.push(ProviderEvent::ToolCallDone { id });
            }
        }
        Some("response.completed") => {
            if let Some(usage) = evt.data.get("usage") {
                let (input_tokens, output_tokens, reasoning_tokens) = parse_usage(usage);
                events.push(ProviderEvent::Usage {
                    input_tokens,
                    output_tokens,
                    reasoning_tokens,
                });
            }
            let reason = evt
                .data
                .get("stop_reason")
                .and_then(|v| v.as_str())
                .map(finish_reason_from_openai)
                .unwrap_or(FinishReason::EndTurn);
            events.push(ProviderEvent::Done { reason });
        }
        Some("response.incomplete") => {
            events.push(ProviderEvent::Done {
                reason: FinishReason::Incomplete,
            });
        }
        Some("response.failed") => {
            return Err(anyhow::anyhow!(
                "Response failed: {:?}",
                evt.data.get("error")
            ));
        }
        _ => {}
    }

    Ok(events)
}

fn finish_reason_from_openai(reason: &str) -> FinishReason {
    match reason {
        "end_turn" | "stop" => FinishReason::EndTurn,
        "tool_calls" => FinishReason::ToolCalls,
        "max_tokens" => FinishReason::MaxTokens,
        "content_filter" => FinishReason::ContentFilter,
        "incomplete" => FinishReason::Incomplete,
        other => FinishReason::Other(other.to_string()),
    }
}

fn parse_usage(value: &Value) -> (usize, usize, usize) {
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
    (input_tokens, output_tokens, reasoning_tokens)
}

fn into_openai(ti: TranscriptItem) -> Result<Vec<serde_json::Value>> {
    use livvi_core::model::{ToolCall, ToolResult, TranscriptContent};

    let mut items = vec![];
    let mut text_parts = vec![];

    for block in ti.blocks {
        match block {
            TranscriptContent::ToolResult(ToolResult { id, content, .. }) => {
                items.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": id,
                    "output": content,
                }));
            }
            TranscriptContent::Reasoning { metadata, .. } => {
                for spec in metadata
                    .as_object()
                    .unwrap_or(&serde_json::Map::new())
                    .get("openai_items")
                    .unwrap_or(&json!([]))
                    .as_array()
                    .unwrap_or(&vec![])
                {
                    let mut item = json!({
                        "type": "reasoning",
                        "summary": spec.get("summary").unwrap_or(&json!([]))
                    });

                    if let Some(rid) = spec.get("id") {
                        item["id"] = rid.clone();
                    }

                    if let Some(enc) = spec.get("encrypted_content") {
                        item["encrypted_content"] = enc.clone();
                    }

                    items.push(item);
                }
            }
            TranscriptContent::ToolCall(ToolCall { id, name, input }) => {
                items.push(json!({
                    "type": "function_call",
                    "id": id,
                    "name": name,
                    "arguments": input,
                }));
            }
            TranscriptContent::Text(text) => {
                text_parts.push(text);
            }
        }
    }

    if !text_parts.is_empty() {
        items.push(serde_json::json!({
            "role": ti.role.to_string(),
            "content": text_parts.join("\n"),
        }));
    }

    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use livvi_core::model::{ToolCall, ToolResult};
    use serde_json::json;

    fn stream_event(event: &str, data: Value) -> ResponseStreamEvent {
        ResponseStreamEvent {
            event: Some(event.to_string()),
            data,
        }
    }

    #[test]
    fn into_openai_user_message_is_not_empty() {
        let item = TranscriptItem::user_message("Hello, world!");
        let items = into_openai(item).unwrap();
        assert_eq!(
            items,
            vec![json!({"role": "user", "content": "Hello, world!"})]
        );
    }

    #[test]
    fn into_openai_system_message() {
        let item = TranscriptItem::system_message("Be helpful.");
        let items = into_openai(item).unwrap();
        assert_eq!(
            items,
            vec![json!({"role": "system", "content": "Be helpful."})]
        );
    }

    #[test]
    fn into_openai_assistant_text() {
        let item = TranscriptItem::assistant_message("Hi there.");
        let items = into_openai(item).unwrap();
        assert_eq!(
            items,
            vec![json!({"role": "assistant", "content": "Hi there."})]
        );
    }

    #[test]
    fn into_openai_function_call_uses_id() {
        let item = TranscriptItem::assistant_tool_call(ToolCall {
            name: "calc".to_string(),
            id: "call-1".to_string(),
            input: json!({"a": 2, "b": 2}),
        });
        let items = into_openai(item).unwrap();
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
        let item = TranscriptItem::tool_result(ToolResult {
            id: "call-1".to_string(),
            content: "4".to_string(),
            is_error: false,
        });
        let items = into_openai(item).unwrap();
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
        let events = provider_events_from_openai(stream_event(
            "response.output_text.delta",
            json!({"delta": "Hello back!"}),
        ))
        .unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            ProviderEvent::TextDelta(ref text) if text == "Hello back!"
        ));

        let done = provider_events_from_openai(stream_event(
            "response.completed",
            json!({
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 10, "output_tokens": 5}
            }),
        ))
        .unwrap();
        assert_eq!(done.len(), 2);
        assert!(matches!(
            done[0],
            ProviderEvent::Usage {
                input_tokens: 10,
                output_tokens: 5,
                ..
            }
        ));
        assert!(matches!(
            done[1],
            ProviderEvent::Done {
                reason: FinishReason::EndTurn
            }
        ));
    }

    #[test]
    fn from_openai_tool_call_response() {
        let start = provider_events_from_openai(stream_event(
            "response.output_item.added",
            json!({
                "type": "function_call",
                "call_id": "call-1",
                "name": "calc",
                "arguments": ""
            }),
        ))
        .unwrap();
        assert!(matches!(
            start[0],
            ProviderEvent::ToolCallStart { ref id, ref name } if id == "call-1" && name == "calc"
        ));

        let delta = provider_events_from_openai(stream_event(
            "response.function_call_arguments.delta",
            json!({"call_id": "call-1", "delta": "{\"a\":2,\"b\":2}"}),
        ))
        .unwrap();
        assert!(matches!(
            delta[0],
            ProviderEvent::ToolCallDelta { ref id, ref arguments } if id == "call-1" && arguments == "{\"a\":2,\"b\":2}"
        ));

        let done = provider_events_from_openai(stream_event(
            "response.output_item.done",
            json!({"type": "function_call", "call_id": "call-1"}),
        ))
        .unwrap();
        assert!(matches!(
            done[0],
            ProviderEvent::ToolCallDone { ref id } if id == "call-1"
        ));

        let completed = provider_events_from_openai(stream_event(
            "response.completed",
            json!({"stop_reason": "tool_calls"}),
        ))
        .unwrap();
        assert!(matches!(
            completed[0],
            ProviderEvent::Done {
                reason: FinishReason::ToolCalls
            }
        ));
    }

    #[test]
    fn from_openai_tool_call_falls_back_to_id() {
        let start = provider_events_from_openai(stream_event(
            "response.output_item.added",
            json!({
                "type": "function_call",
                "id": "call-1",
                "name": "calc",
                "arguments": ""
            }),
        ))
        .unwrap();
        assert!(matches!(
            start[0],
            ProviderEvent::ToolCallStart { ref id, .. } if id == "call-1"
        ));
    }

    #[test]
    fn from_openai_reasoning_response() {
        let events = provider_events_from_openai(stream_event(
            "response.reasoning.summary_text.delta",
            json!({"delta": "Thinking..."}),
        ))
        .unwrap();
        assert!(matches!(
            events[0],
            ProviderEvent::ReasoningDelta(ref text) if text == "Thinking..."
        ));

        let done = provider_events_from_openai(stream_event(
            "response.completed",
            json!({
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 5,
                    "output_tokens_details": {"reasoning": 3}
                }
            }),
        ))
        .unwrap();
        assert!(matches!(
            done[0],
            ProviderEvent::Usage {
                input_tokens: 10,
                output_tokens: 5,
                reasoning_tokens: 3
            }
        ));
    }
}
