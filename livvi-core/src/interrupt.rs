#[derive(Debug, Clone)]
pub enum Interrupt {
    Message { source: String, content: String },
    ExternalEvent(ExternalEvent),
    Reset,
    Compact,
    UpdateSoul { content: String },
}

#[derive(Debug, Clone)]
pub struct ExternalEvent {
    pub source: String,
    pub conversation_id: String,
    pub content: String,
    pub author_id: Option<String>,
    pub author_name: Option<String>,
    pub message_id: Option<String>,
    pub timestamp: Option<i64>,
    pub metadata: serde_json::Value,
}

impl Interrupt {}
