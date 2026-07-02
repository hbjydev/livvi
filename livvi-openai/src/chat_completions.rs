use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, BoxStream, StreamExt};
use livvi_core::{
    model::{Role, ToolCall, ToolResult, Transcript, TranscriptContent, TranscriptItem},
    provider::{FinishReason, Provider, ProviderEvent, ProviderStream},
    tool::Tools,
};
use openai_api_rs::v1::chat_completion::{ChatCompletionMessage, Content, MessageRole};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::common::tool_to_function;

pub struct OpenAIChatCompletionsProvider {
    client: reqwest::Client,
    api_url: String,
    api_key: String,
    model_name: String,
}

impl OpenAIChatCompletionsProvider {
    pub fn new(api_key: &str, api_url: &str, model_name: &str) -> Result<Self> {
        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

        Ok(OpenAIChatCompletionsProvider {
            client,
            api_url: api_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model_name: model_name.to_string(),
        })
    }
}

#[async_trait]
impl<S: Send + Sync + 'static> Provider<S> for OpenAIChatCompletionsProvider {
    async fn stream(&mut self, transcript: Transcript, tools: Tools<S>) -> Result<ProviderStream> {
        let mut messages = vec![];
        for item in transcript.items() {
            messages.extend(into_openai_chat_completion(item));
        }

        let tool_items: Vec<openai_api_rs::v1::chat_completion::Tool> = tools
            .schemas()
            .into_iter()
            .map(|tool| openai_api_rs::v1::chat_completion::Tool {
                r#type: openai_api_rs::v1::chat_completion::ToolType::Function,
                function: tool_to_function(tool),
            })
            .collect();

        let body = ChatCompletionRequestBody {
            model: self.model_name.clone(),
            messages,
            tools: if tool_items.is_empty() {
                None
            } else {
                Some(tool_items)
            },
            stream: true,
            stream_options: Some(json!({ "include_usage": true })),
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.api_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send chat completion request: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Chat completion request failed ({}): {}",
                status,
                text
            ));
        }

        let bytes_stream: BoxStream<'static, Result<Bytes, reqwest::Error>> =
            Box::pin(response.bytes_stream());
        let chunk_stream = parse_sse_stream(bytes_stream);

        let state = StreamState {
            chunk_stream,
            accumulators: HashMap::new(),
            started: HashSet::new(),
            finish_reason: None,
            buffer: VecDeque::new(),
            finalized: false,
        };

        let events = stream::unfold(state, |mut state| async move {
            loop {
                if let Some(event) = state.buffer.pop_front() {
                    return Some((event, state));
                }

                if state.finalized {
                    return None;
                }

                match state.chunk_stream.next().await {
                    Some(Ok(chunk)) => handle_chunk(&mut state, &chunk),
                    Some(Err(e)) => return Some((Err(e), state)),
                    None => {
                        finalize_stream(&mut state);
                        state.finalized = true;
                    }
                }
            }
        });

        Ok(Box::pin(events))
    }
}

#[derive(Serialize)]
struct ChatCompletionRequestBody {
    model: String,
    messages: Vec<ChatCompletionMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<openai_api_rs::v1::chat_completion::Tool>>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChunk {
    #[serde(default)]
    choices: Vec<ChatCompletionChunkChoice>,
    usage: Option<ChatCompletionChunkUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChunkChoice {
    delta: ChatCompletionChunkDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatCompletionChunkDelta {
    content: Option<String>,
    reasoning: Option<String>,
    #[serde(rename = "reasoning_content")]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ChatCompletionChunkToolCall>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChunkToolCall {
    index: Option<usize>,
    id: Option<String>,
    function: Option<ChatCompletionChunkToolCallFunction>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChunkToolCallFunction {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChunkUsage {
    prompt_tokens: i64,
    completion_tokens: i64,
    #[serde(default)]
    completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Deserialize)]
struct CompletionTokensDetails {
    reasoning_tokens: Option<i64>,
}

#[derive(Debug, Default)]
struct ToolCallAccumulator {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

struct StreamState {
    chunk_stream: BoxStream<'static, Result<ChatCompletionChunk>>,
    accumulators: HashMap<usize, ToolCallAccumulator>,
    started: HashSet<usize>,
    finish_reason: Option<String>,
    buffer: VecDeque<Result<ProviderEvent>>,
    finalized: bool,
}

fn parse_sse_stream(
    stream: BoxStream<'static, Result<Bytes, reqwest::Error>>,
) -> BoxStream<'static, Result<ChatCompletionChunk>> {
    Box::pin(stream::unfold(
        (stream, String::new()),
        |(mut stream, mut buffer)| async move {
            loop {
                if let Some((data, rest)) = extract_sse_data(&buffer) {
                    buffer = rest;
                    if data.is_empty() {
                        continue;
                    }
                    if data == "[DONE]" {
                        return None;
                    }
                    match serde_json::from_str::<ChatCompletionChunk>(&data) {
                        Ok(chunk) => return Some((Ok(chunk), (stream, buffer))),
                        Err(e) => {
                            return Some((
                                Err(anyhow::anyhow!("Failed to parse SSE data: {}", e)),
                                (stream, buffer),
                            ));
                        }
                    }
                }

                match stream.next().await {
                    Some(Ok(bytes)) => buffer.push_str(&String::from_utf8_lossy(&bytes)),
                    Some(Err(e)) => {
                        return Some((
                            Err(anyhow::anyhow!("Stream error: {}", e)),
                            (stream, buffer),
                        ));
                    }
                    None => {
                        if buffer.trim().is_empty() {
                            return None;
                        }
                        if let Some((data, _)) = extract_sse_data(&buffer) {
                            buffer = String::new();
                            if data == "[DONE]" || data.is_empty() {
                                return None;
                            }
                            match serde_json::from_str::<ChatCompletionChunk>(&data) {
                                Ok(chunk) => return Some((Ok(chunk), (stream, buffer))),
                                Err(e) => {
                                    return Some((
                                        Err(anyhow::anyhow!("Failed to parse SSE data: {}", e)),
                                        (stream, buffer),
                                    ));
                                }
                            }
                        }
                        return None;
                    }
                }
            }
        },
    ))
}

fn extract_sse_data(buffer: &str) -> Option<(String, String)> {
    let (idx, len) = if let Some(idx) = buffer.find("\r\n\r\n") {
        (idx, 4)
    } else if let Some(idx) = buffer.find("\n\n") {
        (idx, 2)
    } else {
        return None;
    };

    let event = &buffer[..idx];
    let rest = buffer[idx + len..].to_string();

    for line in event.lines() {
        let line = line.trim_end_matches('\r').trim();
        if let Some(data) = line.strip_prefix("data:") {
            return Some((data.trim().to_string(), rest));
        }
    }

    Some((String::new(), rest))
}

fn handle_chunk(state: &mut StreamState, chunk: &ChatCompletionChunk) {
    if let Some(usage) = &chunk.usage {
        state.buffer.push_back(Ok(ProviderEvent::Usage {
            input_tokens: usage.prompt_tokens as usize,
            output_tokens: usage.completion_tokens as usize,
            reasoning_tokens: usage
                .completion_tokens_details
                .as_ref()
                .and_then(|d| d.reasoning_tokens)
                .unwrap_or(0) as usize,
        }));
    }

    for choice in &chunk.choices {
        if let Some(finish_reason) = &choice.finish_reason {
            state.finish_reason = Some(finish_reason.clone());
        }

        if let Some(content) = &choice.delta.content {
            state
                .buffer
                .push_back(Ok(ProviderEvent::TextDelta(content.clone())));
        }

        if let Some(reasoning) = choice
            .delta
            .reasoning
            .as_ref()
            .or(choice.delta.reasoning_content.as_ref())
        {
            state
                .buffer
                .push_back(Ok(ProviderEvent::ReasoningDelta(reasoning.clone())));
        }

        for tc in &choice.delta.tool_calls {
            let index = tc.index.unwrap_or(0);
            let acc = state.accumulators.entry(index).or_default();

            if let Some(id) = &tc.id {
                acc.id = Some(id.clone());
            }
            if let Some(name) = tc.function.as_ref().and_then(|f| f.name.clone()) {
                acc.name = Some(name);
            }
            if let Some(args) = tc.function.as_ref().and_then(|f| f.arguments.clone()) {
                acc.arguments.push_str(&args);
            }

            if let (Some(id), Some(name)) = (acc.id.clone(), acc.name.clone()) {
                if !state.started.contains(&index) {
                    state
                        .buffer
                        .push_back(Ok(ProviderEvent::ToolCallStart { id, name }));
                    state.started.insert(index);
                    if !acc.arguments.is_empty() {
                        state.buffer.push_back(Ok(ProviderEvent::ToolCallDelta {
                            id: acc.id.clone().unwrap(),
                            arguments: acc.arguments.clone(),
                        }));
                    }
                } else if let Some(args) = tc.function.as_ref().and_then(|f| f.arguments.clone()) {
                    state.buffer.push_back(Ok(ProviderEvent::ToolCallDelta {
                        id: acc.id.clone().unwrap(),
                        arguments: args,
                    }));
                }
            }
        }
    }
}

fn finalize_stream(state: &mut StreamState) {
    let mut completed_indices = vec![];

    for (&index, acc) in &state.accumulators {
        if let Some(id) = &acc.id {
            if !state.started.contains(&index) {
                if let Some(name) = &acc.name {
                    state.buffer.push_back(Ok(ProviderEvent::ToolCallStart {
                        id: id.clone(),
                        name: name.clone(),
                    }));
                    if !acc.arguments.is_empty() {
                        state.buffer.push_back(Ok(ProviderEvent::ToolCallDelta {
                            id: id.clone(),
                            arguments: acc.arguments.clone(),
                        }));
                    }
                } else {
                    state.buffer.push_back(Err(anyhow::anyhow!(
                        "Tool call {} completed without a name",
                        id
                    )));
                    continue;
                }
            }
            state
                .buffer
                .push_back(Ok(ProviderEvent::ToolCallDone { id: id.clone() }));
            completed_indices.push(index);
        }
    }

    let has_tool_calls = !completed_indices.is_empty();
    let reason = match state.finish_reason.as_deref() {
        Some("stop") => FinishReason::EndTurn,
        Some("tool_calls") => FinishReason::ToolCalls,
        Some("length") => FinishReason::MaxTokens,
        Some("content_filter") => FinishReason::ContentFilter,
        Some(other) => FinishReason::Other(other.to_string()),
        None => {
            if has_tool_calls {
                FinishReason::ToolCalls
            } else {
                FinishReason::EndTurn
            }
        }
    };

    state.buffer.push_back(Ok(ProviderEvent::Done { reason }));
}

fn into_openai_chat_completion(ti: TranscriptItem) -> Vec<ChatCompletionMessage> {
    let mut messages = vec![];
    let mut text_parts = vec![];
    let mut tool_calls = vec![];

    for block in ti.blocks {
        match block {
            TranscriptContent::Text(text) => text_parts.push(text),
            TranscriptContent::ToolCall(ToolCall { id, name, input }) => {
                tool_calls.push(openai_api_rs::v1::chat_completion::ToolCall {
                    id,
                    r#type: "function".to_string(),
                    function: openai_api_rs::v1::chat_completion::ToolCallFunction {
                        name: Some(name),
                        arguments: Some(input.to_string()),
                    },
                });
            }
            TranscriptContent::ToolResult(ToolResult { id, content, .. }) => {
                messages.push(ChatCompletionMessage {
                    role: MessageRole::tool,
                    content: Content::Text(content),
                    name: None,
                    tool_calls: None,
                    tool_call_id: Some(id),
                });
            }
            TranscriptContent::Reasoning { .. } => {
                // Chat completions do not round-trip reasoning.
            }
        }
    }

    if !text_parts.is_empty() || !tool_calls.is_empty() {
        let content = if text_parts.is_empty() {
            Content::Text(String::new())
        } else {
            Content::Text(text_parts.join("\n"))
        };

        messages.push(ChatCompletionMessage {
            role: match ti.role {
                Role::User => MessageRole::user,
                Role::Assistant => MessageRole::assistant,
                Role::System => MessageRole::system,
            },
            content,
            name: None,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_call_id: None,
        });
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use livvi_core::model::{ToolCall, ToolResult};
    use serde_json::json;

    #[test]
    fn into_openai_user_message_is_not_empty() {
        let item = TranscriptItem::user_message("Hello, world!");
        let messages = into_openai_chat_completion(item);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, MessageRole::user);
    }

    #[test]
    fn into_openai_assistant_text() {
        let item = TranscriptItem::assistant_message("Hi there.");
        let messages = into_openai_chat_completion(item);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, MessageRole::assistant);
    }

    #[test]
    fn into_openai_function_call_and_output() {
        let item = TranscriptItem::assistant_tool_call(ToolCall {
            name: "calc".to_string(),
            id: "call-1".to_string(),
            input: json!({"a": 2, "b": 2}),
        });
        let messages = into_openai_chat_completion(item);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].tool_calls.is_some());

        let serialized = serde_json::to_value(&messages[0]).unwrap();
        assert_eq!(serialized["role"], "assistant");
        assert!(serialized["tool_calls"].is_array());
        assert_eq!(serialized["tool_calls"][0]["id"], "call-1");
        assert_eq!(serialized["tool_calls"][0]["type"], "function");
        assert_eq!(serialized["tool_calls"][0]["function"]["name"], "calc");
        assert_eq!(
            serialized["tool_calls"][0]["function"]["arguments"],
            "{\"a\":2,\"b\":2}"
        );

        let item = TranscriptItem::tool_result(ToolResult {
            id: "call-1".to_string(),
            content: "4".to_string(),
            is_error: false,
        });
        let messages = into_openai_chat_completion(item);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, MessageRole::tool);
        assert_eq!(messages[0].tool_call_id.as_deref(), Some("call-1"));

        let serialized = serde_json::to_value(&messages[0]).unwrap();
        assert_eq!(serialized["role"], "tool");
        assert_eq!(serialized["tool_call_id"], "call-1");
        assert_eq!(serialized["content"], "4");
    }

    #[test]
    fn transcript_round_trips_to_messages() {
        let mut transcript = Transcript::new();
        transcript.add_item(TranscriptItem::system_message("Be helpful."));
        transcript.add_item(TranscriptItem::user_message("What's 2+2?"));
        transcript.add_item(TranscriptItem::assistant_tool_call(ToolCall {
            name: "calc".to_string(),
            id: "call-1".to_string(),
            input: json!({"a": 2, "b": 2}),
        }));
        transcript.add_item(TranscriptItem::tool_result(ToolResult {
            id: "call-1".to_string(),
            content: "4".to_string(),
            is_error: false,
        }));

        let messages: Vec<_> = transcript
            .items()
            .into_iter()
            .flat_map(into_openai_chat_completion)
            .collect();

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, MessageRole::system);
        assert_eq!(messages[1].role, MessageRole::user);
        assert_eq!(messages[2].role, MessageRole::assistant);
        assert!(messages[2].tool_calls.is_some());
        assert_eq!(messages[3].role, MessageRole::tool);
        assert_eq!(messages[3].tool_call_id.as_deref(), Some("call-1"));

        let serialized = serde_json::to_value(&messages).unwrap();
        assert_eq!(serialized[2]["tool_calls"][0]["id"], "call-1");
        assert_eq!(serialized[3]["tool_call_id"], "call-1");
    }

    #[test]
    fn extract_sse_data_parses_simple_event() {
        let buffer = "data: {\"foo\":1}\n\n";
        let (data, rest) = extract_sse_data(buffer).unwrap();
        assert_eq!(data, "{\"foo\":1}");
        assert!(rest.is_empty());
    }

    #[test]
    fn extract_sse_data_handles_partial_buffer() {
        let buffer = "data: {\"foo\":1}";
        assert!(extract_sse_data(buffer).is_none());
    }

    #[test]
    fn handle_chunk_emits_text_delta() {
        let mut state = StreamState {
            chunk_stream: Box::pin(stream::empty()),
            accumulators: HashMap::new(),
            started: HashSet::new(),
            finish_reason: None,
            buffer: VecDeque::new(),
            finalized: false,
        };
        let chunk = ChatCompletionChunk {
            choices: vec![ChatCompletionChunkChoice {
                delta: ChatCompletionChunkDelta {
                    content: Some("Hello".to_string()),
                    ..Default::default()
                },
                finish_reason: None,
            }],
            usage: None,
        };
        handle_chunk(&mut state, &chunk);
        assert_eq!(state.buffer.len(), 1);
        assert!(matches!(
            state.buffer[0].as_ref().unwrap(),
            ProviderEvent::TextDelta(text) if text == "Hello"
        ));
    }

    #[test]
    fn handle_chunk_accumulates_tool_calls() {
        let mut state = StreamState {
            chunk_stream: Box::pin(stream::empty()),
            accumulators: HashMap::new(),
            started: HashSet::new(),
            finish_reason: None,
            buffer: VecDeque::new(),
            finalized: false,
        };

        let chunk1 = ChatCompletionChunk {
            choices: vec![ChatCompletionChunkChoice {
                delta: ChatCompletionChunkDelta {
                    tool_calls: vec![ChatCompletionChunkToolCall {
                        index: Some(0),
                        id: Some("call-1".to_string()),
                        function: Some(ChatCompletionChunkToolCallFunction {
                            name: Some("calc".to_string()),
                            arguments: Some("".to_string()),
                        }),
                    }],
                    ..Default::default()
                },
                finish_reason: None,
            }],
            usage: None,
        };
        handle_chunk(&mut state, &chunk1);
        assert!(matches!(
            state.buffer[0].as_ref().unwrap(),
            ProviderEvent::ToolCallStart { id, name } if id == "call-1" && name == "calc"
        ));

        let chunk2 = ChatCompletionChunk {
            choices: vec![ChatCompletionChunkChoice {
                delta: ChatCompletionChunkDelta {
                    tool_calls: vec![ChatCompletionChunkToolCall {
                        index: Some(0),
                        id: None,
                        function: Some(ChatCompletionChunkToolCallFunction {
                            name: None,
                            arguments: Some("{\"a\":2}".to_string()),
                        }),
                    }],
                    ..Default::default()
                },
                finish_reason: None,
            }],
            usage: None,
        };
        state.buffer.clear();
        handle_chunk(&mut state, &chunk2);
        assert!(matches!(
            state.buffer[0].as_ref().unwrap(),
            ProviderEvent::ToolCallDelta { id, arguments } if id == "call-1" && arguments == "{\"a\":2}"
        ));
    }

    #[test]
    fn finalize_stream_emits_done() {
        let mut state = StreamState {
            chunk_stream: Box::pin(stream::empty()),
            accumulators: HashMap::new(),
            started: HashSet::new(),
            finish_reason: Some("stop".to_string()),
            buffer: VecDeque::new(),
            finalized: false,
        };
        finalize_stream(&mut state);
        assert!(matches!(
            state.buffer[0].as_ref().unwrap(),
            ProviderEvent::Done {
                reason: FinishReason::EndTurn
            }
        ));
    }
}
