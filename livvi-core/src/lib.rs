pub mod agent;
pub mod interrupt;

pub enum AgentEvent {
    Started,
    Token(String),
    ScratchToken(String),
    ThinkToken(String),
    Done,
    Reset,

    CompactionStarted,
    CompactionDone,
    CompactionToken(String),
    CompactionThinkToken(String),

    Error(String),
    Status(String),
    SystemNudge(String),

    ExternalEvent {
        source: String,
        conversation_id: String,
        content: String,
    },

    // Tool calling
    ToolCall {
        name: String,
        args: String,
    },
    ToolCallStarted,
    ToolResult {
        name: String,
        content: String,
    },
}
