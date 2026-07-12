use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use anyhow::Result;
use async_trait::async_trait;
use livvi_core::{
    compaction::Compactor,
    model::{Message, Role},
    summarizer::Summarizer,
};
use livvi_store::ConversationId;
use tokio::sync::Mutex;
use tracing::instrument;
use uuid::Uuid;

use crate::{
    dag::{LcmConfig, LcmConversationState, SummaryNode},
    store::LcmStore,
};

/// Hierarchical LCM compactor that maintains a persistent summary DAG.
pub struct LcmCompactor {
    summarizer: Arc<dyn Summarizer>,
    store: Arc<dyn LcmStore>,
    config: LcmConfig,
    state: Mutex<HashMap<ConversationId, LcmConversationState>>,
}

impl LcmCompactor {
    /// Create a new LCM compactor with the given summarizer, store, and config.
    pub fn new(
        summarizer: Arc<dyn Summarizer>,
        store: Arc<dyn LcmStore>,
        config: LcmConfig,
    ) -> Self {
        Self {
            summarizer,
            store,
            config,
            state: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl Compactor for LcmCompactor {
    #[instrument(skip(self, messages), level = "debug")]
    async fn compact(
        &self,
        messages: &[Message],
        conversation_id: &ConversationId,
    ) -> Result<Vec<Message>> {
        let mut cache = self.state.lock().await;
        let mut state = cache
            .remove(conversation_id)
            .unwrap_or_else(LcmConversationState::default);

        // If the cache is empty, load the persisted state.
        if state.raw_messages.is_empty() && state.summaries.is_empty() {
            state.raw_messages = self.store.load_messages(conversation_id).await?;
            state.summaries = self.store.load_summaries(conversation_id).await?;
        }

        // Identify brand-new raw messages before merging and ensure each has an id.
        let existing_ids: HashSet<Uuid> = state
            .raw_messages
            .iter()
            .filter_map(|m| m.message_id)
            .collect();
        let mut new_messages: Vec<Message> = messages
            .iter()
            .filter(|m| {
                if matches!(m.role, Role::System) {
                    return false;
                }
                m.message_id.is_none_or(|id| !existing_ids.contains(&id))
            })
            .cloned()
            .collect();
        for msg in &mut new_messages {
            if msg.message_id.is_none() {
                msg.message_id = Some(Uuid::new_v4());
            }
        }

        // Merge new messages into the persisted state. System messages are
        // ephemeral summaries and are not persisted as raw messages.
        state.merge_raw_messages(&new_messages);

        // Persist any new raw messages.
        if !new_messages.is_empty() {
            self.store
                .save_messages(conversation_id, &new_messages)
                .await?;
        }

        let total_raw = state.raw_messages.len();
        let tail_start = total_raw.saturating_sub(self.config.fresh_tail_count);
        let eligible = &state.raw_messages[..tail_start];

        let token_proxy: usize = eligible
            .iter()
            .map(|m| m.content_str().chars().count() / 4)
            .sum();

        if token_proxy > self.config.chunk_threshold {
            // Chunk the oldest eligible raw messages into groups of up to
            // fresh_tail_count and summarize each chunk into a depth-0 node.
            for i in (0..tail_start).step_by(self.config.fresh_tail_count) {
                let chunk_end = (i + self.config.fresh_tail_count).min(tail_start);
                let (source_ids, prompt) = {
                    let chunk = &state.raw_messages[i..chunk_end];
                    let ids = chunk.iter().filter_map(|m| m.message_id).collect();
                    let p = build_summary_prompt(0, chunk);
                    (ids, p)
                };
                let content =
                    summarize_or_default(&*self.summarizer, prompt, "(no summary)".to_string())
                        .await?;
                let summary = SummaryNode::new(0, content, source_ids);
                self.store.save_summary(conversation_id, &summary).await?;
                state.summaries.push(summary);

                self.condense_summaries(conversation_id, &mut state).await?;
            }
        }

        // Assemble the active context: top-level summaries followed by the
        // fresh tail of raw messages.
        let mut active = state.active_summaries();
        active.sort_by_key(|a| a.created_at);

        let mut result = Vec::with_capacity(active.len() + self.config.fresh_tail_count);
        for summary in active {
            result.push(Message::system(summary.content.clone()));
        }
        result.extend(state.raw_messages[tail_start..].iter().cloned());

        // Update the cache.
        cache.insert(conversation_id.clone(), state);

        Ok(result)
    }
}

impl LcmCompactor {
    async fn condense_summaries(
        &self,
        conversation_id: &ConversationId,
        state: &mut LcmConversationState,
    ) -> Result<()> {
        for depth in 0..=self.config.max_depth {
            let to_condense: Vec<Uuid> = {
                let mut active_at_depth: Vec<&SummaryNode> = state
                    .summaries
                    .iter()
                    .filter(|s| s.depth == depth && s.parent_id.is_none())
                    .collect();
                active_at_depth.sort_by_key(|a| a.created_at);

                if active_at_depth.len() >= self.config.condensation_count {
                    active_at_depth
                        .into_iter()
                        .take(self.config.condensation_count)
                        .map(|s| s.id)
                        .collect()
                } else {
                    continue;
                }
            };

            let context: Vec<Message> = state
                .summaries
                .iter()
                .filter(|s| to_condense.contains(&s.id))
                .map(|s| Message::system(s.content.clone()))
                .collect();
            let prompt = build_summary_prompt(depth + 1, &context);
            let content =
                summarize_or_default(&*self.summarizer, prompt, "(no summary)".to_string()).await?;
            let parent = SummaryNode::new(depth + 1, content, to_condense.clone());

            // Mark children as condensed.
            for child_id in &to_condense {
                if let Some(s) = state.summaries.iter_mut().find(|s| s.id == *child_id) {
                    s.parent_id = Some(parent.id);
                }
            }

            self.store.save_summary(conversation_id, &parent).await?;
            state.summaries.push(parent);
        }

        Ok(())
    }
}

async fn summarize_or_default(
    summarizer: &dyn Summarizer,
    prompt: Vec<Message>,
    default: String,
) -> Result<String> {
    match summarizer.summarize(prompt).await {
        Ok(text) if text.trim().is_empty() => Ok(default),
        Ok(text) => Ok(text),
        Err(e) => {
            tracing::warn!("summarizer failed: {}", e);
            Ok(default)
        }
    }
}

fn build_summary_prompt(depth: usize, context: &[Message]) -> Vec<Message> {
    let instruction = match depth {
        0 => {
            "Summarize this conversation segment for future turns. Preserve decisions, \
             rationale, constraints, active tasks. Remove repetition and conversational filler. \
             End with: 'Expand for details about: <what was compressed>.'"
        }
        1 => {
            "Input: depth-0 summaries, not raw messages. Preserve decisions, outcomes, \
             blockers, in-progress state. Drop transient states, dead ends, process scaffolding. \
             Include a timeline. End with: 'Expand for details about: <what was compressed>.'"
        }
        _ => {
            "Produce a durable narrative: decisions still in effect, completed work, milestone \
             timeline. End with: 'Expand for details about: <what was compressed>.'"
        }
    };

    let mut prompt = Vec::with_capacity(context.len() + 1);
    prompt.push(Message::system(instruction));
    prompt.extend(context.iter().cloned());
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MockLcmStore;
    use livvi_core::model::Message;
    use livvi_store::ConversationId;

    #[derive(Clone)]
    struct MockSummarizer {
        prefix: String,
    }

    #[async_trait]
    impl Summarizer for MockSummarizer {
        async fn summarize(&self, prompt: Vec<Message>) -> Result<String> {
            let count = prompt
                .iter()
                .filter(|m| !matches!(m.role, Role::System))
                .count();
            Ok(format!("{}[{} items]", self.prefix, count))
        }
    }

    #[tokio::test]
    async fn lcm_compactor_bounds_context() {
        let store = Arc::new(MockLcmStore::new());
        let summarizer = Arc::new(MockSummarizer {
            prefix: "summary".to_string(),
        });
        let config = LcmConfig {
            fresh_tail_count: 4,
            chunk_threshold: 50,
            condensation_count: 2,
            max_depth: 3,
        };
        let compactor = LcmCompactor::new(summarizer, store.clone(), config);
        let conversation_id = ConversationId::from("test-conv");

        // Build 12 long user messages so eligible raw messages exceed the threshold.
        let messages: Vec<Message> = (0..12)
            .map(|i| {
                let content = format!(
                    "user message {} with a lot of content to exceed the threshold",
                    i
                );
                Message::user(content, None)
            })
            .collect();

        let compacted = compactor
            .compact(&messages, &conversation_id)
            .await
            .unwrap();

        // Should have at least one summary and at most fresh_tail_count + active summaries.
        let summary_count = compacted
            .iter()
            .filter(|m| matches!(m.role, Role::System))
            .count();
        let raw_count = compacted.len() - summary_count;

        assert!(
            summary_count >= 1,
            "expected at least one summary, got {}",
            summary_count
        );
        assert!(
            raw_count <= config.fresh_tail_count,
            "raw count {} exceeds fresh_tail_count {}",
            raw_count,
            config.fresh_tail_count
        );

        // The store should contain both raw messages and summaries.
        let stored_messages = store.load_messages(&conversation_id).await.unwrap();
        let stored_summaries = store.load_summaries(&conversation_id).await.unwrap();
        assert_eq!(stored_messages.len(), 12);
        assert!(!stored_summaries.is_empty());
    }
}
