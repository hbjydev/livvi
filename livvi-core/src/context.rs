use crate::model::{Message, ToolCall, Usage};
use livvi_store::{ConversationId, PersonId};

#[derive(Debug, Clone)]
pub struct Context {
    pub system: Vec<Message>,
    pub turns: Vec<Message>,
    pub usage: Usage,
    pub conversation_id: Option<ConversationId>,
}

/// Strip scratchpad tags from assistant output before storing it in context.
///
/// The system prompt tells the model that plain assistant text *is* the
/// scratchpad, so we do not wrap it. However, the model may still emit
/// `<scratchpad>`/`<\/scratchpad>` tags (especially if it learned the format
/// from earlier contexts). This function removes them so they cannot
/// accumulate and recurse.
pub(crate) fn clean_assistant_text(content: impl Into<String>) -> String {
    let content = content.into();
    content
        .replace("<scratchpad>", "")
        .replace("</scratchpad>", "")
        .trim()
        .to_string()
}

/// Replace empty assistant content with a placeholder so that providers which
/// reject empty assistant messages (e.g. Moonshot) still receive a valid turn.
fn non_empty_assistant_content(content: impl Into<String>) -> String {
    let content = content.into();
    if content.is_empty() {
        "(no content)".to_string()
    } else {
        content
    }
}

impl Context {
    /// Create a new context with a given soul file and optional conversation id.
    /// The soul file is stored as a system message in the context.
    pub fn new(soul: impl Into<String>, conversation_id: Option<ConversationId>) -> Self {
        Context {
            system: vec![Message::system(soul)],
            turns: vec![],
            usage: Usage::default(),
            conversation_id,
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
    pub fn push_user(&mut self, content: impl Into<String>, person_id: Option<PersonId>) {
        let mut msg = Message::user(content, person_id);
        msg.conversation_id = self.conversation_id.clone();
        self.turns.push(msg);
    }

    /// Push an output (assistant) message to the context, optionally with
    /// thinking text.
    pub fn push_assistant(&mut self, content: impl Into<String>, thinking_content: Option<String>) {
        let content = non_empty_assistant_content(content);
        let mut msg = Message::assistant(content, thinking_content);
        msg.conversation_id = self.conversation_id.clone();
        self.turns.push(msg);
    }

    /// Push tool calls from the assistant to the context, optionally with
    /// text content and thinking text.
    pub fn push_assistant_tool_calls(
        &mut self,
        calls: Vec<ToolCall>,
        content: Option<impl Into<String>>,
        thinking_content: Option<impl Into<String>>,
    ) {
        let content = content.map(non_empty_assistant_content);
        let mut msg = Message::with_tool_calls(calls, content, thinking_content);
        msg.conversation_id = self.conversation_id.clone();
        self.turns.push(msg);
    }

    /// Push a tool result message to the context, with the tool call ID and
    /// the result content.
    pub fn push_tool_result(&mut self, id: impl Into<String>, content: impl Into<String>) {
        let mut msg = Message::tool_result(id, content);
        msg.conversation_id = self.conversation_id.clone();
        self.turns.push(msg);
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
        self.usage.cached_tokens += usage.cached_tokens;
        self.usage.uncached_tokens += usage.uncached_tokens;
    }

    /// Compact the conversation history using the provided compactor.
    ///
    /// The compactor receives the current turns and the conversation id and
    /// returns a new set of turns that usually begins with a summary of older
    /// messages followed by the most recent turns kept verbatim. If compaction
    /// is not needed, the compactor may return the turns unchanged.
    pub async fn compact(
        &mut self,
        compactor: &dyn crate::compaction::Compactor,
        conversation_id: &livvi_store::ConversationId,
    ) {
        self.turns = compactor
            .compact(&self.turns, conversation_id)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("compaction failed: {}", e);
                self.turns.clone()
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_assistant_text_strips_tags_and_trims() {
        assert_eq!(clean_assistant_text("hello"), "hello");
        assert_eq!(clean_assistant_text(""), "");
        assert_eq!(
            clean_assistant_text("<scratchpad>hello</scratchpad>"),
            "hello"
        );
        assert_eq!(
            clean_assistant_text("<scratchpad><scratchpad>hello</scratchpad></scratchpad>"),
            "hello"
        );
        assert_eq!(clean_assistant_text("<scratchpad></scratchpad>"), "");
        assert_eq!(
            clean_assistant_text("<scratchpad>  hello  </scratchpad>"),
            "hello"
        );
    }

    #[test]
    fn test_soul_injected_on_create() {
        let ctx = Context::new("This is the soul", None);
        assert_eq!(ctx.system.len(), 1);

        let msg = &ctx.system[0];
        assert_eq!(msg.role, crate::model::Role::System);
        assert_eq!(msg.content.as_deref(), Some("This is the soul"));
    }

    #[test]
    fn test_soul_replaced_on_update() {
        let mut ctx = Context::new("This is the soul", None);
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

    #[tokio::test]
    async fn test_compact_replaces_old_turns_with_summary() {
        let compactor = crate::compaction::WindowCompactor {
            keep_ratio: 0.2,
            min_keep: 2,
            trigger_threshold: 5,
            max_chars_per_turn: 50,
        };
        let mut ctx = Context::new("soul", None);
        for i in 0..8 {
            ctx.push_assistant(format!("assistant {i}"), None);
            ctx.push_user(format!("user {i}"), None);
        }
        // 16 turns > threshold of 5, so compaction should fire.
        ctx.compact(&compactor, &"test".into()).await;

        // 1 summary + ceil(16 * 0.2)=4 kept turns = 5 turns.
        assert_eq!(ctx.turns.len(), 5);
        assert_eq!(ctx.turns[0].role, crate::model::Role::System);
        assert_eq!(ctx.turns[1].content.as_deref(), Some("assistant 6"));
    }
}
