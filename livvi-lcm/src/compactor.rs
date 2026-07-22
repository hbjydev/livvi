use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
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

const SUMMARIZE_TIMEOUT: Duration = Duration::from_secs(60);

/// Hierarchical LCM compactor that maintains a persistent summary DAG.
pub struct LcmCompactor {
    summarizer: Arc<dyn Summarizer>,
    store: Arc<dyn LcmStore>,
    config: LcmConfig,
    state: Mutex<HashMap<ConversationId, Arc<Mutex<LcmConversationState>>>>,
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
        // Get or create the per-conversation state lock. The outer lock is only
        // held briefly; compaction work for this conversation is serialized by
        // the inner lock, not a global lock.
        let state_arc = {
            let mut cache = self.state.lock().await;
            cache
                .entry(conversation_id.clone())
                .or_insert_with(|| Arc::new(Mutex::new(LcmConversationState::default())))
                .clone()
        };
        let mut state = state_arc.lock().await;

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

        let tail_start = safe_tail_start(&state.raw_messages, self.config.fresh_tail_count);
        let eligible = &state.raw_messages[..tail_start];

        let token_proxy: usize = eligible
            .iter()
            .map(|m| m.content_str().chars().count() / 4)
            .sum();

        let mut summarized_ids: HashSet<Uuid> = HashSet::new();
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
                summarized_ids.extend(
                    state.raw_messages[i..chunk_end]
                        .iter()
                        .filter_map(|m| m.message_id),
                );

                self.condense_summaries(conversation_id, &mut state).await?;
            }

            // Remove raw messages that have been rolled into depth-0 summaries
            // so they are not re-summarized on the next turn.
            state
                .raw_messages
                .retain(|m| !summarized_ids.contains(&m.message_id.unwrap_or_default()));
        }

        // Assemble the active context: top-level summaries followed by the
        // fresh tail of raw messages.
        let mut active = state.active_summaries();
        active.sort_by_key(|a| a.created_at);

        let mut result = Vec::with_capacity(active.len() + self.config.fresh_tail_count);
        for summary in active {
            result.push(Message::system(summary.content.clone()));
        }
        let new_tail_start = safe_tail_start(&state.raw_messages, self.config.fresh_tail_count);
        result.extend(state.raw_messages[new_tail_start..].iter().cloned());

        Ok(result)
    }
}

/// Compute the start index of the fresh tail without splitting a tool-call
/// group: if the tail would begin with tool results, walk back to include the
/// assistant message that produced them.
fn safe_tail_start(messages: &[Message], fresh_tail_count: usize) -> usize {
    let mut start = messages.len().saturating_sub(fresh_tail_count);
    while start > 0 && matches!(messages[start].role, Role::Tool) {
        start -= 1;
    }
    start
}

impl LcmCompactor {
    async fn condense_summaries(
        &self,
        conversation_id: &ConversationId,
        state: &mut LcmConversationState,
    ) -> Result<()> {
        for depth in 0..self.config.max_depth {
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

            // Mark children as condensed and persist their updated parent_id.
            for child_id in &to_condense {
                if let Some(s) = state.summaries.iter_mut().find(|s| s.id == *child_id) {
                    s.parent_id = Some(parent.id);
                    self.store.save_summary(conversation_id, s).await?;
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
    let result = tokio::time::timeout(SUMMARIZE_TIMEOUT, summarizer.summarize(prompt)).await;
    match result {
        Ok(Ok(text)) if text.trim().is_empty() => Ok(default),
        Ok(Ok(text)) => Ok(text),
        Ok(Err(e)) => {
            tracing::warn!("summarizer failed: {}", e);
            Ok(default)
        }
        Err(_) => {
            tracing::warn!("summarizer timed out after {:?}", SUMMARIZE_TIMEOUT);
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
    prompt.extend(context.iter().map(flatten_for_summary));
    prompt
}

/// Render a message as plain text for summarization. Tool calls and results
/// are flattened so the prompt never contains `tool` role messages or
/// `tool_calls`, which strict providers reject when a chunk boundary splits a
/// tool-call group.
fn flatten_for_summary(msg: &Message) -> Message {
    match msg.role {
        Role::Tool => {
            let mut flattened = Message::user(
                format!(
                    "[tool result {}]: {}",
                    msg.tool_call_id.as_deref().unwrap_or("unknown"),
                    msg.content_str()
                ),
                msg.person_id.clone(),
            );
            flattened.message_id = msg.message_id;
            flattened
        }
        Role::Assistant if msg.tool_calls.is_some() => {
            let mut text = msg.content_str().to_string();
            for call in msg.tool_calls.as_deref().unwrap_or_default() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&format!("[tool call {}({})]", call.name, call.input));
            }
            let mut flattened = Message::assistant(text, None::<&str>);
            flattened.message_id = msg.message_id;
            flattened
        }
        _ => msg.clone(),
    }
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

    #[tokio::test]
    async fn lcm_compactor_prunes_summarised_raw_messages() {
        let store = Arc::new(MockLcmStore::new());
        let summarizer = Arc::new(MockSummarizer {
            prefix: "summary".to_string(),
        });
        let config = LcmConfig {
            fresh_tail_count: 4,
            chunk_threshold: 50,
            condensation_count: 4,
            max_depth: 3,
        };
        let compactor = LcmCompactor::new(summarizer, store.clone(), config);
        let conversation_id = ConversationId::from("test-conv");

        let messages: Vec<Message> = (0..12)
            .map(|i| {
                let content = format!(
                    "user message {} with a lot of content to exceed the threshold",
                    i
                );
                Message::user(content, None)
            })
            .collect();

        // First compaction summarises eligible messages and prunes them from memory.
        compactor
            .compact(&messages, &conversation_id)
            .await
            .unwrap();

        // Second compaction with no new messages should not create duplicate summaries.
        let compacted = compactor.compact(&[], &conversation_id).await.unwrap();

        let summary_count = compacted
            .iter()
            .filter(|m| matches!(m.role, Role::System))
            .count();
        assert_eq!(summary_count, 2, "expected two top-level summaries");
        assert_eq!(compacted.len() - summary_count, config.fresh_tail_count);

        let stored_summaries = store.load_summaries(&conversation_id).await.unwrap();
        assert_eq!(stored_summaries.len(), 2);
    }

    #[test]
    fn summary_prompt_flattens_tool_messages() {
        use livvi_core::model::ToolCall;
        use serde_json::json;

        let context = vec![
            Message::user("run the thing", None),
            Message::with_tool_calls(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "calc".to_string(),
                    input: json!({"a": 1}),
                }],
                None::<&str>,
                None::<&str>,
            ),
            Message::tool_result("call-1", "42"),
        ];

        let prompt = build_summary_prompt(0, &context);

        assert!(prompt.iter().all(|m| !matches!(m.role, Role::Tool)));
        assert!(prompt.iter().all(|m| m.tool_calls.is_none()));
        assert!(
            prompt
                .iter()
                .any(|m| m.content_str().contains("[tool call calc("))
        );
        assert!(
            prompt
                .iter()
                .any(|m| m.content_str().contains("[tool result call-1]: 42"))
        );
    }

    #[test]
    fn tail_start_does_not_split_tool_groups() {
        use livvi_core::model::ToolCall;
        use serde_json::json;

        let messages = vec![
            Message::user("one", None),
            Message::user("two", None),
            Message::with_tool_calls(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "calc".to_string(),
                    input: json!({}),
                }],
                None::<&str>,
                None::<&str>,
            ),
            Message::tool_result("call-1", "42"),
            Message::user("three", None),
        ];

        // A naive tail of 2 would start at the orphaned tool result.
        let start = safe_tail_start(&messages, 2);
        assert_eq!(start, 2);
        assert!(matches!(messages[start].role, Role::Assistant));
    }
}
