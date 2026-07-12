use livvi_store::ConversationId;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// A summary node in the LCM DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryNode {
    pub id: Uuid,
    pub conversation_id: Option<ConversationId>,
    pub depth: usize,
    pub content: String,
    pub source_ids: Vec<Uuid>,
    #[serde(skip_serializing)]
    pub source_ids_json: String,
    pub parent_id: Option<Uuid>,
    pub created_at: Option<OffsetDateTime>,
}

impl SummaryNode {
    pub fn new(depth: usize, content: String, source_ids: Vec<Uuid>) -> Self {
        Self {
            id: Uuid::new_v4(),
            conversation_id: None,
            depth,
            content,
            source_ids_json: serde_json::to_string(&source_ids).unwrap_or_default(),
            source_ids,
            parent_id: None,
            created_at: Some(OffsetDateTime::now_utc()),
        }
    }
}

/// Configuration for the LCM compactor.
#[derive(Debug, Clone, Copy)]
pub struct LcmConfig {
    /// Number of raw messages to keep verbatim at the end of the conversation.
    pub fresh_tail_count: usize,
    /// Approximate token threshold (using `chars / 4`) before eligible raw
    /// messages are summarized.
    pub chunk_threshold: usize,
    /// Number of active summaries at a given depth before they are condensed
    /// into a higher-depth summary.
    pub condensation_count: usize,
    /// Maximum depth of the summary hierarchy.
    pub max_depth: usize,
}

impl Default for LcmConfig {
    fn default() -> Self {
        Self {
            fresh_tail_count: 6,
            chunk_threshold: 2000,
            condensation_count: 4,
            max_depth: 3,
        }
    }
}

impl LcmConfig {
    /// Read configuration from environment variables, falling back to defaults.
    pub fn from_env() -> Self {
        Self {
            fresh_tail_count: parse_env_usize("LIVVI_LCM_FRESH_TAIL_COUNT", 6),
            chunk_threshold: parse_env_usize("LIVVI_LCM_CHUNK_THRESHOLD", 2000),
            condensation_count: parse_env_usize("LIVVI_LCM_CONDENSATION_COUNT", 4),
            max_depth: parse_env_usize("LIVVI_LCM_MAX_DEPTH", 3),
        }
    }
}

fn parse_env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// In-memory state for a single conversation managed by LCM.
#[derive(Debug, Default, Clone)]
pub struct LcmConversationState {
    pub raw_messages: Vec<Message>,
    pub summaries: Vec<SummaryNode>,
}

use livvi_core::model::Message;

impl LcmConversationState {
    /// Merge raw messages into the state, replacing any existing messages by
    /// `message_id` and appending new ones.
    pub fn merge_raw_messages(&mut self, messages: &[Message]) {
        for msg in messages {
            if let Some(id) = msg.message_id
                && let Some(existing) = self
                    .raw_messages
                    .iter_mut()
                    .find(|m| m.message_id == Some(id))
            {
                *existing = msg.clone();
                continue;
            }
            if !matches!(msg.role, livvi_core::model::Role::System) {
                self.raw_messages.push(msg.clone());
            }
        }
    }

    /// Return the active (top-level) summaries: those with no parent.
    pub fn active_summaries(&self) -> Vec<&SummaryNode> {
        self.summaries
            .iter()
            .filter(|s| s.parent_id.is_none())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use livvi_core::model::Message;
    use uuid::Uuid;

    #[test]
    fn active_summaries_returns_roots_not_children() {
        let mut state = LcmConversationState::default();
        let child_a = SummaryNode::new(0, "child a".to_string(), vec![Uuid::new_v4()]);
        let child_b = SummaryNode::new(0, "child b".to_string(), vec![Uuid::new_v4()]);
        let parent = SummaryNode::new(1, "parent".to_string(), vec![child_a.id, child_b.id]);

        // Mirror what condense_summaries does: children point to parent.
        let mut child_a = child_a;
        let mut child_b = child_b;
        child_a.parent_id = Some(parent.id);
        child_b.parent_id = Some(parent.id);

        state.summaries = vec![child_a, child_b, parent.clone()];

        let active = state.active_summaries();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, parent.id);
    }

    #[test]
    fn merge_raw_messages_skips_system_messages() {
        let mut state = LcmConversationState::default();
        state.merge_raw_messages(&[
            Message::user("hello".to_string(), None),
            Message::system("beep"),
        ]);
        assert_eq!(state.raw_messages.len(), 1);
        assert!(matches!(
            state.raw_messages[0].role,
            livvi_core::model::Role::User
        ));
    }
}
