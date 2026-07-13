use anyhow::{Result, anyhow};
use async_trait::async_trait;
use livvi_core::model::{Message, Role, ToolCall};
use livvi_store::ConversationId;
use parking_lot::Mutex;
use serde_json;
use sqlx::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;
use time::OffsetDateTime;
use tracing::instrument;
use uuid::Uuid;

use crate::dag::SummaryNode;

/// Backend-agnostic storage for the LCM DAG: raw messages and summary nodes.
#[async_trait]
pub trait LcmStore: Send + Sync + 'static {
    /// Persist raw messages for a conversation. Messages are appended in order
    /// and assigned the next available sequence numbers.
    async fn save_messages(
        &self,
        conversation_id: &ConversationId,
        messages: &[Message],
    ) -> Result<()>;

    /// Load all raw messages for a conversation, ordered by sequence.
    async fn load_messages(&self, conversation_id: &ConversationId) -> Result<Vec<Message>>;

    /// Persist a summary node.
    async fn save_summary(
        &self,
        conversation_id: &ConversationId,
        summary: &SummaryNode,
    ) -> Result<()>;

    /// Load all summary nodes for a conversation.
    async fn load_summaries(&self, conversation_id: &ConversationId) -> Result<Vec<SummaryNode>>;
}

/// SQLite-backed implementation of [`LcmStore`].
pub struct LcmSqliteStore {
    pool: SqlitePool,
}

impl LcmSqliteStore {
    /// Connect to a SQLite database and run pending migrations.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = if database_url.starts_with("sqlite::memory:")
            || database_url.starts_with("file::memory:")
        {
            // In-memory SQLite databases are private to each connection unless
            // a shared cache is used. Force a single-connection pool so every
            // query operates on the same in-memory database.
            SqlitePoolOptions::new()
                .max_connections(1)
                .connect(database_url)
                .await?
        } else {
            SqlitePool::connect(database_url).await?
        };
        sqlx::migrate!().run(&pool).await?;
        Ok(Self { pool })
    }

    /// Create a store from an existing pool without running migrations.
    pub fn from_pool(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl LcmStore for LcmSqliteStore {
    #[instrument(skip(self, messages), level = "trace")]
    async fn save_messages(
        &self,
        conversation_id: &ConversationId,
        messages: &[Message],
    ) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;
        let max_sequence: i64 = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(sequence), 0) FROM lcm_messages WHERE conversation_id = ?",
        )
        .bind(&conversation_id.0)
        .fetch_one(&mut *tx)
        .await?;

        for (i, msg) in messages.iter().enumerate() {
            let sequence = max_sequence + i as i64 + 1;
            let id = msg
                .message_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            let role = msg.role.to_string();
            let content = msg.content.as_deref();
            let person_id = msg.person_id.as_ref().map(|pid| pid.0.as_str());
            let tool_calls_serialized = msg
                .tool_calls
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            let tool_calls_json = tool_calls_serialized.as_deref();
            let tool_call_id = msg.tool_call_id.as_deref();
            let thinking_content = msg.thinking_content.as_deref();
            let created_at = OffsetDateTime::now_utc();

            sqlx::query(
                "INSERT OR REPLACE INTO lcm_messages \
                 (id, conversation_id, sequence, role, content, person_id, tool_calls_json, tool_call_id, thinking_content, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&conversation_id.0)
            .bind(sequence)
            .bind(&role)
            .bind(content)
            .bind(person_id)
            .bind(tool_calls_json)
            .bind(tool_call_id)
            .bind(thinking_content)
            .bind(created_at)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    #[instrument(skip(self), level = "trace")]
    async fn load_messages(&self, conversation_id: &ConversationId) -> Result<Vec<Message>> {
        let rows = sqlx::query_as::<_, MessageRow>(
            "SELECT id, conversation_id, sequence, role, content, person_id, tool_calls_json, tool_call_id, thinking_content, created_at \
             FROM lcm_messages \
             WHERE conversation_id = ? \
             ORDER BY sequence ASC",
        )
        .bind(&conversation_id.0)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    #[instrument(skip(self, summary), level = "trace")]
    async fn save_summary(
        &self,
        conversation_id: &ConversationId,
        summary: &SummaryNode,
    ) -> Result<()> {
        let source_ids_json = serde_json::to_string(&summary.source_ids)?;
        let created_at = summary.created_at.unwrap_or_else(OffsetDateTime::now_utc);

        sqlx::query(
            "INSERT OR REPLACE INTO lcm_summaries \
             (id, conversation_id, depth, content, source_ids_json, parent_id, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(summary.id.to_string())
        .bind(&conversation_id.0)
        .bind(summary.depth as i64)
        .bind(&summary.content)
        .bind(&source_ids_json)
        .bind(summary.parent_id.map(|id| id.to_string()))
        .bind(created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[instrument(skip(self), level = "trace")]
    async fn load_summaries(&self, conversation_id: &ConversationId) -> Result<Vec<SummaryNode>> {
        let rows = sqlx::query_as::<_, SummaryRow>(
            "SELECT id, conversation_id, depth, content, source_ids_json, parent_id, created_at \
             FROM lcm_summaries \
             WHERE conversation_id = ? \
             ORDER BY created_at ASC",
        )
        .bind(&conversation_id.0)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }
}

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct MessageRow {
    id: String,
    conversation_id: String,
    sequence: i64,
    role: String,
    content: Option<String>,
    person_id: Option<String>,
    tool_calls_json: Option<String>,
    tool_call_id: Option<String>,
    thinking_content: Option<String>,
    created_at: OffsetDateTime,
}

impl TryFrom<MessageRow> for Message {
    type Error = anyhow::Error;

    fn try_from(row: MessageRow) -> Result<Self, Self::Error> {
        let role = match row.role.as_str() {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "system" => Role::System,
            "tool" => Role::Tool,
            other => return Err(anyhow!("unknown role: {}", other)),
        };

        let tool_calls = row
            .tool_calls_json
            .map(|s| serde_json::from_str::<Vec<ToolCall>>(&s))
            .transpose()?;

        Ok(Message {
            role,
            person_id: row.person_id.map(livvi_store::PersonId),
            content: row.content,
            thinking_content: row.thinking_content,
            tool_calls,
            tool_call_id: row.tool_call_id,
            conversation_id: Some(ConversationId(row.conversation_id)),
            message_id: Some(Uuid::parse_str(&row.id)?),
        })
    }
}

#[derive(sqlx::FromRow)]
struct SummaryRow {
    id: String,
    conversation_id: String,
    depth: i64,
    content: String,
    source_ids_json: String,
    parent_id: Option<String>,
    created_at: OffsetDateTime,
}

impl TryFrom<SummaryRow> for SummaryNode {
    type Error = anyhow::Error;

    fn try_from(row: SummaryRow) -> Result<Self, Self::Error> {
        Ok(SummaryNode {
            id: Uuid::parse_str(&row.id)?,
            conversation_id: Some(ConversationId(row.conversation_id)),
            depth: row.depth as usize,
            content: row.content,
            source_ids_json: row.source_ids_json.clone(),
            source_ids: serde_json::from_str(&row.source_ids_json)?,
            parent_id: row.parent_id.map(|id| Uuid::parse_str(&id)).transpose()?,
            created_at: Some(row.created_at),
        })
    }
}

/// In-memory [`LcmStore`] for unit tests.
#[derive(Default)]
pub struct MockLcmStore {
    messages: Mutex<Vec<(ConversationId, Message)>>,
    summaries: Mutex<Vec<SummaryNode>>,
}

impl MockLcmStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl LcmStore for MockLcmStore {
    async fn save_messages(
        &self,
        conversation_id: &ConversationId,
        messages: &[Message],
    ) -> Result<()> {
        let mut stored = self.messages.lock();
        for msg in messages {
            let mut to_save = msg.clone();
            to_save.conversation_id = Some(conversation_id.clone());
            if to_save.message_id.is_none() {
                to_save.message_id = Some(Uuid::new_v4());
            }
            let id = to_save.message_id.unwrap();
            if let Some((_, existing)) = stored
                .iter_mut()
                .find(|(cid, m)| cid == conversation_id && m.message_id == Some(id))
            {
                *existing = to_save;
            } else {
                stored.push((conversation_id.clone(), to_save));
            }
        }
        Ok(())
    }

    async fn load_messages(&self, conversation_id: &ConversationId) -> Result<Vec<Message>> {
        let stored = self.messages.lock();
        Ok(stored
            .iter()
            .filter(|(cid, _)| cid == conversation_id)
            .map(|(_, msg)| msg)
            .cloned()
            .collect())
    }

    async fn save_summary(
        &self,
        conversation_id: &ConversationId,
        summary: &SummaryNode,
    ) -> Result<()> {
        let mut stored = self.summaries.lock();
        stored.retain(|s| s.id != summary.id);
        let mut to_save = summary.clone();
        to_save.conversation_id = Some(conversation_id.clone());
        stored.push(to_save);
        Ok(())
    }

    async fn load_summaries(&self, conversation_id: &ConversationId) -> Result<Vec<SummaryNode>> {
        let stored = self.summaries.lock();
        Ok(stored
            .iter()
            .filter(|s| s.conversation_id.as_ref() == Some(conversation_id))
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use livvi_core::model::Message;
    use livvi_store::ConversationId;

    #[tokio::test]
    async fn mock_store_upserts_messages_by_id() {
        let store = MockLcmStore::new();
        let conversation_id = ConversationId::from("test-conv");
        let message_id = Uuid::new_v4();

        let first = Message::user("first".to_string(), None);
        let mut first = first;
        first.message_id = Some(message_id);

        let mut second = Message::user("second".to_string(), None);
        second.message_id = Some(message_id);

        store
            .save_messages(&conversation_id, &[first])
            .await
            .unwrap();
        store
            .save_messages(&conversation_id, &[second.clone()])
            .await
            .unwrap();

        let loaded = store.load_messages(&conversation_id).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].content, second.content);
    }
}
