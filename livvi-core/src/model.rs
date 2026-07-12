use std::fmt::Display;

use livvi_store::{ConversationId, PersonId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
}

impl Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::User => write!(f, "user"),
            Role::Assistant => write!(f, "assistant"),
            Role::System => write!(f, "system"),
            Role::Tool => write!(f, "tool"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// The ID of the specific tool call.
    pub id: String,

    /// The name of the tool being called.
    pub name: String,

    /// The input provided to the tool call, represented as a JSON value.
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    /// The ID of the specific tool call that produced this result.
    pub id: String,

    /// The tool results, represented as a JSON value.
    pub content: String,

    /// Indicates whether the tool call resulted in an error.
    pub is_error: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub person_id: Option<PersonId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<ConversationId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<Uuid>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Message {
            role: Role::System,
            person_id: None,
            content: Some(content.into()),
            thinking_content: None,
            tool_calls: None,
            tool_call_id: None,
            conversation_id: None,
            message_id: None,
        }
    }

    pub fn user(content: impl Into<String>, person_id: Option<PersonId>) -> Self {
        Message {
            role: Role::User,
            person_id,
            content: Some(content.into()),
            thinking_content: None,
            tool_calls: None,
            tool_call_id: None,
            conversation_id: None,
            message_id: Some(Uuid::new_v4()),
        }
    }

    pub fn assistant(
        content: impl Into<String>,
        thinking_content: Option<impl Into<String>>,
    ) -> Self {
        Message {
            role: Role::Assistant,
            person_id: None,
            content: Some(content.into()),
            thinking_content: thinking_content.map(|c| c.into()),
            tool_calls: None,
            tool_call_id: None,
            conversation_id: None,
            message_id: Some(Uuid::new_v4()),
        }
    }

    pub fn with_tool_calls(
        calls: Vec<ToolCall>,
        content: Option<impl Into<String>>,
        thinking_content: Option<impl Into<String>>,
    ) -> Self {
        Message {
            role: Role::Assistant,
            person_id: None,
            content: content.map(|c| c.into()),
            thinking_content: thinking_content.map(|c| c.into()),
            tool_calls: Some(calls),
            tool_call_id: None,
            conversation_id: None,
            message_id: Some(Uuid::new_v4()),
        }
    }

    pub fn tool_result(id: impl Into<String>, content: impl Into<String>) -> Self {
        Message {
            role: Role::Tool,
            person_id: None,
            content: Some(content.into()),
            thinking_content: None,
            tool_calls: None,
            tool_call_id: Some(id.into()),
            conversation_id: None,
            message_id: Some(Uuid::new_v4()),
        }
    }

    pub fn content_str(&self) -> &str {
        self.content.as_deref().unwrap_or("")
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub reasoning_tokens: usize,
    pub cached_tokens: usize,
    pub uncached_tokens: usize,
    pub prompt_processing_ms: u64,
    pub generation_ms: u64,
}
