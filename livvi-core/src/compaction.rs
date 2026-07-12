use anyhow::Result;
use async_trait::async_trait;
use livvi_store::ConversationId;

use crate::model::{Message, Role};

/// Strategy for compressing a long conversation history so the context window
/// stays bounded while keeping as much useful signal as possible.
///
/// A [`Compactor`] receives the full list of turns for a specific conversation
/// and returns a new, shorter list of turns that represents the same
/// conversation. Implementations decide how much to keep verbatim, how much
/// to drop, and how to summarize what is dropped.
#[async_trait]
pub trait Compactor: Send + Sync + 'static {
    /// Compact `messages` into a smaller set of turns for `conversation_id`.
    ///
    /// The returned vector replaces the input turns entirely. The first message
    /// is typically a system-style summary of the dropped history, followed
    /// by the most recent turns that were kept verbatim. If compaction is not
    /// needed, the compactor may return the turns unchanged.
    async fn compact(
        &self,
        messages: &[Message],
        conversation_id: &ConversationId,
    ) -> Result<Vec<Message>>;
}

#[async_trait]
impl Compactor for Box<dyn Compactor> {
    async fn compact(
        &self,
        messages: &[Message],
        conversation_id: &ConversationId,
    ) -> Result<Vec<Message>> {
        self.as_ref().compact(messages, conversation_id).await
    }
}

/// A simple compactor that keeps the last ~20% of turns verbatim and summarizes
/// the older turns into a single system message.
///
/// The goal is to preserve the *texture* of the recent conversation (the exact
/// back-and-forth, phrasing, tool calls, and results) while compressing older
/// turns into a compact narrative that keeps the speaker, the gist, and the
/// flow rather than a cold keyword list.
#[derive(Debug, Clone)]
pub struct WindowCompactor {
    /// Fraction of turns to keep verbatim (0.0–1.0). Defaults to 0.2 (20%).
    pub keep_ratio: f32,
    /// Minimum number of turns to keep, even when the ratio would keep fewer.
    /// Defaults to 6.
    pub min_keep: usize,
    /// Only compact when there are more than this many turns. Prevents churn
    /// and loss of early texture in short conversations. Defaults to 50.
    pub trigger_threshold: usize,
    /// Maximum characters of original content to include per summarized turn.
    /// Defaults to 200.
    pub max_chars_per_turn: usize,
}

impl Default for WindowCompactor {
    fn default() -> Self {
        Self {
            keep_ratio: 0.2,
            min_keep: 6,
            trigger_threshold: 50,
            max_chars_per_turn: 200,
        }
    }
}

impl WindowCompactor {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Compactor for WindowCompactor {
    async fn compact(
        &self,
        messages: &[Message],
        _conversation_id: &ConversationId,
    ) -> Result<Vec<Message>> {
        if messages.len() <= self.trigger_threshold.max(self.min_keep) {
            return Ok(messages.to_vec());
        }

        let keep = ((messages.len() as f32) * self.keep_ratio)
            .ceil()
            .max(self.min_keep as f32)
            .min(messages.len() as f32) as usize;

        let summary_len = messages.len().saturating_sub(keep);
        if summary_len == 0 {
            return Ok(messages.to_vec());
        }

        let summary = summarize_messages(&messages[..summary_len], self.max_chars_per_turn);
        let mut result = Vec::with_capacity(keep + 1);
        result.push(Message::system(summary));
        result.extend_from_slice(&messages[summary_len..]);
        Ok(result)
    }
}

fn summarize_messages(messages: &[Message], max_chars_per_turn: usize) -> String {
    if messages.is_empty() {
        return String::new();
    }

    let groups = group_by_speaker(messages);
    let mut lines = vec!["Earlier conversation summary:".to_string()];
    for group in groups {
        let speaker = speaker_label(&group[0]);
        let body = describe_group(group, max_chars_per_turn);
        lines.push(format!("- {speaker} {body}"));
    }
    lines.join("\n")
}

fn speaker_label(msg: &Message) -> String {
    let role = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
        Role::Tool => "tool",
    };
    match &msg.person_id {
        Some(person_id) => format!("{} ({})", role, person_id.0),
        None => role.to_string(),
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    let text = text.trim();
    if text.is_empty() {
        return "(no content)".to_string();
    }

    // Strip the scratchpad wrapper that the assistant uses for raw responses so
    // the summary reads more like narrative and less like XML.
    let text = text
        .strip_prefix("<scratchpad>")
        .and_then(|s| s.strip_suffix("</scratchpad>"))
        .unwrap_or(text);

    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        text.chars().take(max_chars).collect::<String>() + " …"
    }
}

fn group_by_speaker(messages: &[Message]) -> Vec<&[Message]> {
    let mut groups: Vec<&[Message]> = Vec::new();
    let mut start = 0;
    for (i, msg) in messages.iter().enumerate().skip(1) {
        if !same_speaker(&messages[start], msg) {
            groups.push(&messages[start..i]);
            start = i;
        }
    }
    groups.push(&messages[start..]);
    groups
}

fn same_speaker(a: &Message, b: &Message) -> bool {
    a.role == b.role && a.person_id == b.person_id
}

fn describe_group(messages: &[Message], max_chars_per_turn: usize) -> String {
    let role = messages[0].role.clone();
    let phrases: Vec<String> = messages
        .iter()
        .map(|msg| describe_message(msg, max_chars_per_turn))
        .collect();
    let joined = join_phrases(&phrases);

    match role {
        Role::User => {
            if joined.ends_with('?') {
                format!("asked {}{}", joined, "?")
            } else {
                format!("said {}", joined)
            }
        }
        Role::Assistant
            if messages[0]
                .tool_calls
                .as_ref()
                .is_some_and(|c| !c.is_empty()) =>
        {
            format!("used {}", joined)
        }
        Role::Assistant => format!("responded {}", joined),
        Role::Tool => format!("returned {}", joined),
        Role::System => format!("noted {}", joined),
    }
}

fn describe_message(msg: &Message, max_chars_per_turn: usize) -> String {
    if let Some(calls) = &msg.tool_calls
        && !calls.is_empty()
    {
        let names: Vec<&str> = calls.iter().map(|c| c.name.as_str()).collect();
        return format!("tool(s): {}", names.join(", "));
    }
    truncate(msg.content_str(), max_chars_per_turn)
}

fn join_phrases(phrases: &[String]) -> String {
    if phrases.is_empty() {
        return String::new();
    }
    if phrases.len() == 1 {
        return phrases[0].clone();
    }
    let all = phrases.join(", ");
    let last_comma = all.rfind(", ").unwrap_or(all.len() - 1);
    format!("{} and {}", &all[..last_comma], &all[last_comma + 2..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ToolCall;

    fn msg(role: Role, content: &str) -> Message {
        Message {
            role,
            person_id: None,
            content: Some(content.to_string()),
            thinking_content: None,
            tool_calls: None,
            tool_call_id: None,
            conversation_id: None,
            message_id: None,
        }
    }

    #[tokio::test]
    async fn window_compactor_leaves_short_history_alone() {
        let compactor = WindowCompactor::default();
        let messages: Vec<Message> = (0..10)
            .map(|i| {
                if i % 2 == 0 {
                    msg(Role::User, &format!("user turn {i}"))
                } else {
                    msg(Role::Assistant, &format!("assistant turn {i}"))
                }
            })
            .collect();

        let compacted = compactor.compact(&messages, &"test".into()).await.unwrap();
        assert_eq!(compacted.len(), messages.len());
        assert_eq!(compacted, messages);
    }

    #[tokio::test]
    async fn window_compactor_keeps_last_fraction_verbatim() {
        let compactor = WindowCompactor {
            keep_ratio: 0.2,
            min_keep: 2,
            trigger_threshold: 10,
            max_chars_per_turn: 200,
        };
        let messages: Vec<Message> = (0..20)
            .map(|i| {
                if i % 2 == 0 {
                    msg(Role::User, &format!("user turn {i}"))
                } else {
                    msg(Role::Assistant, &format!("assistant turn {i}"))
                }
            })
            .collect();

        let compacted = compactor.compact(&messages, &"test".into()).await.unwrap();
        // 20 * 0.2 = 4 kept, 16 summarized into one system message.
        assert_eq!(compacted.len(), 5);
        assert_eq!(compacted[0].role, Role::System);
        assert_eq!(compacted[1].content.as_deref(), Some("user turn 16"));
        assert_eq!(compacted[2].content.as_deref(), Some("assistant turn 17"));
        assert_eq!(compacted[3].content.as_deref(), Some("user turn 18"));
        assert_eq!(compacted[4].content.as_deref(), Some("assistant turn 19"));
    }

    #[tokio::test]
    async fn window_compactor_respects_min_keep() {
        let compactor = WindowCompactor {
            keep_ratio: 0.05,
            min_keep: 5,
            trigger_threshold: 10,
            max_chars_per_turn: 200,
        };
        let messages: Vec<Message> = (0..20)
            .map(|i| msg(Role::User, &format!("turn {i}")))
            .collect();

        let compacted = compactor.compact(&messages, &"test".into()).await.unwrap();
        // 5 kept + 1 summary = 6.
        assert_eq!(compacted.len(), 6);
    }

    #[tokio::test]
    async fn summary_preserves_texture_not_just_keywords() {
        let compactor = WindowCompactor {
            keep_ratio: 0.2,
            min_keep: 2,
            trigger_threshold: 5,
            max_chars_per_turn: 50,
        };
        let messages = vec![
            msg(
                Role::User,
                "hey livvi, can you help me plan a trip to japan?",
            ),
            msg(
                Role::Assistant,
                "<scratchpad>totally! japan is amazing, what part are you thinking?</scratchpad>",
            ),
            msg(Role::User, "tokyo and kyoto, maybe two weeks"),
            msg(
                Role::Assistant,
                "<scratchpad>solid choices. you'll want a jr pass for sure.</scratchpad>",
            ),
            msg(Role::User, "what's the weather like in march?"),
            msg(
                Role::Assistant,
                "<scratchpad>march is great, cherry blossoms start in late march.</scratchpad>",
            ),
            msg(Role::User, "any hotel recommendations in kyoto?"),
            msg(
                Role::Assistant,
                "<scratchpad>check out the ryokan near gion for a classic experience.</scratchpad>",
            ),
        ];

        let compacted = compactor.compact(&messages, &"test".into()).await.unwrap();
        let summary = compacted[0].content.as_deref().unwrap_or("");

        // The summary should read like a narrative back-and-forth, not a keyword list.
        assert!(summary.contains("user asked"));
        assert!(summary.contains("assistant responded"));
        assert!(summary.contains("plan a trip to japan"));
        assert!(summary.contains("cherry blossoms"));
        // Scratchpad tags should be stripped from the summary narrative.
        assert!(!summary.contains("<scratchpad>"));
    }

    #[tokio::test]
    async fn tool_calls_are_summarized() {
        let compactor = WindowCompactor {
            keep_ratio: 0.5,
            min_keep: 1,
            trigger_threshold: 1,
            max_chars_per_turn: 200,
        };
        let mut assistant_msg = Message::assistant("", None::<String>);
        assistant_msg.tool_calls = Some(vec![ToolCall {
            id: "call_1".to_string(),
            name: "weather".to_string(),
            input: serde_json::json!({"city": "tokyo"}),
        }]);
        let user_msg = Message::user("what's the weather?", None);
        let messages = vec![assistant_msg, user_msg];

        let compacted = compactor.compact(&messages, &"test".into()).await.unwrap();
        let summary = compacted[0].content.as_deref().unwrap_or("");
        assert!(summary.contains("used tool(s): weather"));
    }

    #[tokio::test]
    async fn person_id_is_included_in_summary() {
        let compactor = WindowCompactor {
            keep_ratio: 0.5,
            min_keep: 1,
            trigger_threshold: 1,
            max_chars_per_turn: 200,
        };
        let mut user_msg = Message::user("hello", None);
        user_msg.person_id = Some("person-42".into());
        let assistant_msg = Message::assistant("hi person-42!", None::<String>);
        let messages = vec![user_msg, assistant_msg];

        let compacted = compactor.compact(&messages, &"test".into()).await.unwrap();
        let summary = compacted[0].content.as_deref().unwrap_or("");
        assert!(summary.contains("user (person-42) said"));
    }
}
