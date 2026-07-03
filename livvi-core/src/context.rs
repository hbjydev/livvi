use crate::model::{Message, ToolCall, Usage};

#[derive(Debug, Clone)]
/// Context holds the conversation context for an agent, including the system
/// message (soul), the turns of the conversation, and usage statistics.
pub struct Context {
    pub system: Vec<Message>,
    pub turns: Vec<Message>,
    pub usage: Usage,
}

impl Context {
    /// Create a new context with a given soul file. The soul file is stored as
    /// a system message in the context.
    pub fn new(soul: impl Into<String>) -> Self {
        Context {
            system: vec![Message::system(soul)],
            turns: vec![],
            usage: Usage::default(),
        }
    }

    /// Rebuild the soul of the context with a fresh soul file.
    pub fn update_soul(&mut self, soul: impl Into<String>) {
        if let Some(sys) = self.system.first_mut() {
            sys.content = Some(soul.into());
        }
    }

    /// Remove a turn from the context by index. If the index is out of bounds,
    /// this method does nothing.
    pub fn remove_turn(&mut self, index: usize) {
        if index < self.turns.len() {
            self.turns.remove(index);
        }
    }

    /// Push an input (user) message to the context.
    pub fn push_user(&mut self, content: impl Into<String>) {
        self.turns.push(Message::user(content))
    }

    /// Push an output (assistant) message to the context, optionally with
    /// thinking text.
    pub fn push_assistant(&mut self, content: impl Into<String>, thinking_content: Option<String>) {
        self.turns
            .push(Message::assistant(content, thinking_content))
    }

    /// Push tool calls from the assistant to the context, optionally with
    /// text content and thinking text.
    pub fn push_assistant_tool_calls(
        &mut self,
        calls: Vec<ToolCall>,
        content: Option<impl Into<String>>,
        thinking_content: Option<impl Into<String>>,
    ) {
        self.turns
            .push(Message::with_tool_calls(calls, content, thinking_content))
    }

    /// Push a tool result message to the context, with the tool call ID and
    /// the result content.
    pub fn push_tool_result(&mut self, id: impl Into<String>, content: impl Into<String>) {
        self.turns.push(Message::tool_result(id, content))
    }

    pub fn as_messages(&self) -> Vec<Message> {
        self.system.iter().chain(&self.turns).cloned().collect()
    }

    pub fn turn_count(&self) -> usize {
        self.turns.len()
    }

    pub fn clear(&mut self) {
        self.turns.clear();
        self.usage = Usage::default();
    }

    pub fn update_usage(&mut self, usage: Usage) {
        self.usage.input_tokens += usage.input_tokens;
        self.usage.output_tokens += usage.output_tokens;
        self.usage.prompt_processing_ms += usage.prompt_processing_ms;
        self.usage.reasoning_tokens += usage.reasoning_tokens;
        self.usage.generation_ms += usage.generation_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_soul_injected_on_create() {
        let ctx = Context::new("This is the soul");
        assert_eq!(ctx.system.len(), 1);

        let msg = &ctx.system[0];
        assert_eq!(msg.role, crate::model::Role::System);
        assert_eq!(msg.content.as_deref(), Some("This is the soul"));
    }

    #[test]
    fn test_soul_replaced_on_update() {
        let mut ctx = Context::new("This is the soul");
        assert_eq!(ctx.system.len(), 1);
        let msg = &ctx.system[0];
        assert_eq!(msg.role, crate::model::Role::System);
        assert_eq!(msg.content.as_deref(), Some("This is the soul"));

        ctx.update_soul("This is the new soul");
        assert_eq!(ctx.system.len(), 1);
        let msg = &ctx.system[0];
        assert_eq!(msg.role, crate::model::Role::System);
        assert_eq!(msg.content.as_deref(), Some("This is the new soul"));
    }
}
