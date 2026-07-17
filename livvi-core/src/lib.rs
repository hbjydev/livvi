pub use async_trait::async_trait;
pub use schemars;
pub use serde_json;

use crate::model::{ToolCall, ToolResult};

pub mod agent;
pub mod provider;
pub mod tool;

pub mod compaction;
pub mod context;
pub mod interrupt;
pub mod memory;
pub mod model;
pub mod plugin;
pub mod resolve;
pub mod state;
pub mod summarizer;

#[derive(Debug, Clone)]
/// An event emitted by the agent during its operation. This enum captures
/// various stages of the agent's lifecycle, tool interactions, and reporting
/// events.
pub enum AgentEvent {
    // Loop Events
    Token(String),
    ScratchToken(String),
    ThinkingToken(String),

    // Lifecycle Hooks
    Started,
    Done,
    Reset,

    // Tool Lifecycle Hooks
    ToolCall(Vec<ToolCall>),
    ToolCallStarted,
    ToolError(String),
    ToolResult(ToolResult),

    // Reporting
    Error(String),
    Status(String),
}
