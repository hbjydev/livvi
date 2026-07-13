use std::fmt;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use crate::person::{Person, PersonId};

/// Canonical identifier for a [`Conversation`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct ConversationId(pub String);

impl fmt::Display for ConversationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for ConversationId {
    fn from(value: String) -> Self {
        ConversationId(value)
    }
}

impl From<&str> for ConversationId {
    fn from(value: &str) -> Self {
        ConversationId(value.to_string())
    }
}

/// A conversation thread, identified by a transport-specific channel or room.
///
/// `Conversation` does not store the message history itself; that is left for a
/// separate context persistence layer. It tracks the participants and transport
/// metadata needed to route events and associate people with the thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: ConversationId,
    pub transport_kind: String,
    pub transport_id: String,
    pub title: Option<String>,
    pub metadata: Value,
    pub created_at: OffsetDateTime,
    pub last_active_at: OffsetDateTime,
}

/// Repository for [`Conversation`] records and their participants.
#[async_trait]
pub trait ConversationStorage: Send + Sync + 'static {
    /// Look up a conversation by its transport identity.
    async fn resolve_conversation(
        &self,
        transport_kind: &str,
        transport_id: &str,
    ) -> Result<Option<Conversation>>;

    /// Create a new conversation record.
    async fn create_conversation(
        &self,
        transport_kind: &str,
        transport_id: &str,
        title: Option<String>,
        metadata: Value,
    ) -> Result<Conversation>;

    /// Fetch a conversation by its canonical ID.
    async fn get_conversation(&self, id: &ConversationId) -> Result<Option<Conversation>>;

    /// Add a person to a conversation. Idempotent.
    async fn add_participant(
        &self,
        conversation_id: &ConversationId,
        person_id: &PersonId,
    ) -> Result<()>;

    /// List all participants in a conversation.
    async fn get_participants(&self, conversation_id: &ConversationId) -> Result<Vec<Person>>;

    /// Resolve a conversation, creating it if no match exists.
    async fn ensure_conversation(
        &self,
        transport_kind: &str,
        transport_id: &str,
        title: Option<String>,
        metadata: Value,
    ) -> Result<Conversation> {
        if let Some(conversation) = self
            .resolve_conversation(transport_kind, transport_id)
            .await?
        {
            return Ok(conversation);
        }

        self.create_conversation(transport_kind, transport_id, title, metadata)
            .await
    }
}
