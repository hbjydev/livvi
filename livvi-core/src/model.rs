use std::{fmt::Display, time::Instant};

use serde_json::{Value, json};

use crate::provider::ProviderResponse;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    System,
}

impl Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::User => write!(f, "user"),
            Role::Assistant => write!(f, "assistant"),
            Role::System => write!(f, "system"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub id: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub id: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptContent {
    Text(String),
    ToolCall(ToolCall),
    ToolResult(ToolResult),
    Reasoning { metadata: Value, text: String },
}

#[derive(Debug, Clone)]
pub struct TranscriptItem {
    pub role: Role,
    pub blocks: Vec<TranscriptContent>,
    pub created_at: Instant,
}

impl TranscriptItem {
    pub fn user_message(content: impl Into<String>) -> Self {
        TranscriptItem {
            role: Role::User,
            blocks: vec![TranscriptContent::Text(content.into())],
            created_at: Instant::now(),
        }
    }

    pub fn assistant_message(content: impl Into<String>) -> Self {
        TranscriptItem {
            role: Role::Assistant,
            blocks: vec![TranscriptContent::Text(content.into())],
            created_at: Instant::now(),
        }
    }

    pub fn assistant_reasoning(content: impl Into<String>) -> Self {
        TranscriptItem {
            role: Role::Assistant,
            blocks: vec![TranscriptContent::Reasoning {
                text: content.into(),
                metadata: json!({}),
            }],
            created_at: Instant::now(),
        }
    }

    pub fn assistant_tool_call(tool_call: ToolCall) -> Self {
        TranscriptItem {
            role: Role::Assistant,
            blocks: vec![TranscriptContent::ToolCall(tool_call)],
            created_at: Instant::now(),
        }
    }

    pub fn tool_result(tool_result: ToolResult) -> Self {
        TranscriptItem {
            role: Role::User,
            blocks: vec![TranscriptContent::ToolResult(tool_result)],
            created_at: Instant::now(),
        }
    }

    pub fn system_message(content: impl Into<String>) -> Self {
        TranscriptItem {
            role: Role::System,
            blocks: vec![TranscriptContent::Text(content.into())],
            created_at: Instant::now(),
        }
    }
}

impl From<ProviderResponse> for TranscriptItem {
    fn from(response: ProviderResponse) -> Self {
        match response.value {
            crate::provider::ProviderResponseValue::ToolCalls(calls) => {
                let mut item = TranscriptItem {
                    role: Role::Assistant,
                    created_at: Instant::now(),
                    blocks: vec![],
                };

                for call in calls {
                    item.blocks.push(TranscriptContent::ToolCall(ToolCall {
                        name: call.tool_name,
                        id: call.tool_call_id,
                        input: call.tool_args,
                    }));
                }

                item
            }
            crate::provider::ProviderResponseValue::Text(text) => {
                TranscriptItem::assistant_message(text)
            }
            crate::provider::ProviderResponseValue::Reasoning(reasoning) => TranscriptItem {
                role: Role::Assistant,
                blocks: vec![TranscriptContent::Reasoning {
                    metadata: serde_json::json!({
                        "input_tokens": response.input_tokens,
                        "output_tokens": response.output_tokens,
                        "reasoning_tokens": response.reasoning_tokens
                    }),
                    text: reasoning,
                }],
                created_at: Instant::now(),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct Transcript(Vec<TranscriptItem>);

impl Transcript {
    pub fn new() -> Self {
        Transcript(Vec::new())
    }

    pub fn add_item(&mut self, item: TranscriptItem) {
        self.0.push(item);
    }

    pub fn items(&self) -> Vec<TranscriptItem> {
        self.0.clone()
    }
}

impl Default for Transcript {
    fn default() -> Self {
        Transcript::new()
    }
}
