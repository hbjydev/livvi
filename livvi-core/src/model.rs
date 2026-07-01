#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone)]
pub enum TranscriptContent {
    Text(String),
    ToolUse {
        name: String,
        id: String,
        input: String,
    },
    ToolResult {
        id: String,
        content: String,
    }
}

#[derive(Debug, Clone)]
pub struct TranscriptItem {
    pub role: Role,
    pub content: TranscriptContent,
}

impl TranscriptItem {
    pub fn user_message(content: impl Into<String>) -> Self {
        TranscriptItem {
            role: Role::User,
            content: TranscriptContent::Text(content.into()),
        }
    }

    pub fn assistant_message(content: impl Into<String>) -> Self {
        TranscriptItem {
            role: Role::Assistant,
            content: TranscriptContent::Text(content.into()),
        }
    }

    pub fn system_message(content: impl Into<String>) -> Self {
        TranscriptItem {
            role: Role::System,
            content: TranscriptContent::Text(content.into()),
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
