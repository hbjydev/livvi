use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, BoxStream, StreamExt};
use livvi_core::{
    model::{Message, Role, ToolCall, Usage},
    provider::{Provider, ProviderEvent},
    summarizer::Summarizer,
    tool::ToolDefinition,
};
use openai_api_rs::v1::chat_completion::{ChatCompletionMessage, Content, MessageRole};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::common::tool_to_function;

#[derive(Clone)]
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
impl Provider for OpenAIChatCompletionsProvider {
    fn clone_dyn(&self) -> Box<dyn Provider> {
        Box::new(self.clone())
    }

    async fn stream(
        &mut self,
        tx: mpsc::Sender<ProviderEvent>,
        messages: Vec<Message>,
        tool_schemas: HashMap<String, ToolDefinition>,
    ) -> Result<()> {
        let mut chat_messages = vec![];
        for msg in messages {
            chat_messages.extend(into_openai_chat_completion(msg));
        }

        let tool_items: Vec<openai_api_rs::v1::chat_completion::Tool> = tool_schemas
            .into_values()
            .map(|tool| openai_api_rs::v1::chat_completion::Tool {
                r#type: openai_api_rs::v1::chat_completion::ToolType::Function,
                function: tool_to_function(tool),
            })
            .collect();

        let body = ChatCompletionRequestBody {
            model: self.model_name.clone(),
            messages: chat_messages,
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
        let mut chunk_stream = parse_sse_stream(bytes_stream);

        let mut accumulators: HashMap<usize, ToolCallAccumulator> = HashMap::new();
        let mut tool_call_started = false;

        while let Some(chunk_result) = chunk_stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    if let Some(usage) = chunk.usage {
                        let cached_tokens = usage
                            .prompt_tokens_details
                            .as_ref()
                            .and_then(|d| d.cached_tokens)
                            .unwrap_or(0) as usize;
                        if tx
                            .send(ProviderEvent::Usage(Usage {
                                input_tokens: usage.prompt_tokens as usize,
                                output_tokens: usage.completion_tokens as usize,
                                reasoning_tokens: usage
                                    .completion_tokens_details
                                    .as_ref()
                                    .and_then(|d| d.reasoning_tokens)
                                    .unwrap_or(0)
                                    as usize,
                                cached_tokens,
                                uncached_tokens: (usage.prompt_tokens as usize)
                                    .saturating_sub(cached_tokens),
                                prompt_processing_ms: 0,
                                generation_ms: 0,
                            }))
                            .await
                            .is_err()
                        {
                            return Ok(());
                        }
                    }

                    for choice in &chunk.choices {
                        if let Some(content) = &choice.delta.content
                            && tx
                                .send(ProviderEvent::Token(content.clone()))
                                .await
                                .is_err()
                        {
                            return Ok(());
                        }

                        if let Some(reasoning) = choice
                            .delta
                            .reasoning
                            .as_ref()
                            .or(choice.delta.reasoning_content.as_ref())
                            && tx
                                .send(ProviderEvent::ThinkingToken(reasoning.clone()))
                                .await
                                .is_err()
                        {
                            return Ok(());
                        }

                        if !choice.delta.tool_calls.is_empty() && !tool_call_started {
                            if tx.send(ProviderEvent::ToolCallStarted).await.is_err() {
                                return Ok(());
                            }
                            tool_call_started = true;
                        }

                        for tc in &choice.delta.tool_calls {
                            let index = tc.index.unwrap_or(0);
                            let acc = accumulators.entry(index).or_default();
                            if let Some(id) = &tc.id {
                                acc.id = Some(id.clone());
                            }
                            if let Some(name) = tc.function.as_ref().and_then(|f| f.name.clone()) {
                                acc.name = Some(name);
                            }
                            if let Some(args) =
                                tc.function.as_ref().and_then(|f| f.arguments.clone())
                            {
                                acc.arguments.push_str(&args);
                            }
                        }
                    }
                }
                Err(e) => return Err(e),
            }
        }

        let mut completed_calls = vec![];
        for (_, acc) in accumulators {
            if let (Some(id), Some(name)) = (acc.id, acc.name) {
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

#[async_trait]
impl Summarizer for OpenAIChatCompletionsProvider {
    async fn summarize(&self, prompt: Vec<Message>) -> Result<String> {
        let chat_messages: Vec<ChatCompletionMessage> = prompt
            .into_iter()
            .flat_map(into_openai_chat_completion)
            .collect();

        let body = ChatCompletionRequestBody {
            model: self.model_name.clone(),
            messages: chat_messages,
            tools: None,
            stream: false,
            stream_options: None,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.api_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send summarization request: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Summarization request failed ({}): {}",
                status,
                text
            ));
        }

        let result: ChatCompletionResponse = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse summarization response: {}", e))?;

        let content = result
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .filter(|c| !c.trim().is_empty())
            .unwrap_or_else(|| "(no summary)".to_string());

        Ok(content)
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
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Debug, Deserialize)]
struct CompletionTokensDetails {
    reasoning_tokens: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct PromptTokensDetails {
    cached_tokens: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatCompletionResponseChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponseChoice {
    message: ChatCompletionResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponseMessage {
    content: Option<String>,
}

#[derive(Debug, Default)]
struct ToolCallAccumulator {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
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

fn into_openai_chat_completion(msg: Message) -> Vec<ChatCompletionMessage> {
    let mut messages = vec![];

    match msg.role {
        Role::Tool => {
            if let (Some(content), Some(tool_call_id)) = (msg.content, msg.tool_call_id) {
                messages.push(ChatCompletionMessage {
                    role: MessageRole::tool,
                    content: Content::Text(content),
                    name: None,
                    tool_calls: None,
                    tool_call_id: Some(tool_call_id),
                });
            }
        }
        Role::Assistant => {
            let tool_calls = msg
                .tool_calls
                .unwrap_or_default()
                .into_iter()
                .map(|tc| openai_api_rs::v1::chat_completion::ToolCall {
                    id: tc.id,
                    r#type: "function".to_string(),
                    function: openai_api_rs::v1::chat_completion::ToolCallFunction {
                        name: Some(tc.name),
                        arguments: Some(tc.input.to_string()),
                    },
                })
                .collect::<Vec<_>>();

            messages.push(ChatCompletionMessage {
                role: MessageRole::assistant,
                content: Content::Text(msg.content.unwrap_or_default()),
                name: None,
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                tool_call_id: None,
            });
        }
        Role::User => {
            messages.push(ChatCompletionMessage {
                role: MessageRole::user,
                content: Content::Text(msg.content.unwrap_or_default()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }
        Role::System => {
            messages.push(ChatCompletionMessage {
                role: MessageRole::system,
                content: Content::Text(msg.content.unwrap_or_default()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use livvi_core::model::ToolCall;
    use serde_json::json;

    #[test]
    fn into_openai_user_message_is_not_empty() {
        let msg = Message::user("Hello, world!", None);
        let messages = into_openai_chat_completion(msg);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, MessageRole::user);
    }

    #[test]
    fn into_openai_assistant_text() {
        let msg = Message::assistant("Hi there.", None::<&str>);
        let messages = into_openai_chat_completion(msg);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, MessageRole::assistant);
    }

    #[test]
    fn into_openai_function_call_and_output() {
        let msg = Message::with_tool_calls(
            vec![ToolCall {
                name: "calc".to_string(),
                id: "call-1".to_string(),
                input: json!({"a": 2, "b": 2}),
            }],
            None::<&str>,
            None::<&str>,
        );
        let messages = into_openai_chat_completion(msg);
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

        let msg = Message::tool_result("call-1", "4");
        let messages = into_openai_chat_completion(msg);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, MessageRole::tool);
        assert_eq!(messages[0].tool_call_id.as_deref(), Some("call-1"));

        let serialized = serde_json::to_value(&messages[0]).unwrap();
        assert_eq!(serialized["role"], "tool");
        assert_eq!(serialized["tool_call_id"], "call-1");
        assert_eq!(serialized["content"], "4");
    }

    #[test]
    fn messages_round_trip_to_chat_format() {
        let messages = vec![
            Message::system("Be helpful."),
            Message::user("What's 2+2?", None),
            Message::with_tool_calls(
                vec![ToolCall {
                    name: "calc".to_string(),
                    id: "call-1".to_string(),
                    input: json!({"a": 2, "b": 2}),
                }],
                None::<&str>,
                None::<&str>,
            ),
            Message::tool_result("call-1", "4"),
        ];

        let chat_messages: Vec<_> = messages
            .into_iter()
            .flat_map(into_openai_chat_completion)
            .collect();

        assert_eq!(chat_messages.len(), 4);
        assert_eq!(chat_messages[0].role, MessageRole::system);
        assert_eq!(chat_messages[1].role, MessageRole::user);
        assert_eq!(chat_messages[2].role, MessageRole::assistant);
        assert!(chat_messages[2].tool_calls.is_some());
        assert_eq!(chat_messages[3].role, MessageRole::tool);
        assert_eq!(chat_messages[3].tool_call_id.as_deref(), Some("call-1"));

        let serialized = serde_json::to_value(&chat_messages).unwrap();
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
}
