use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::conversation::ConversationId;

/// A single explicit permission entry for a tool in a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolPermission {
    pub conversation_id: ConversationId,
    pub tool_name: String,
    pub allowed: bool,
    pub updated_at: OffsetDateTime,
}

/// Backend-agnostic repository for per-conversation tool permissions.
#[async_trait]
pub trait ToolPermissionStorage: Send + Sync + 'static {
    /// Grant or revoke permission for a tool in a conversation.
    async fn set_tool_permission(
        &self,
        conversation_id: &ConversationId,
        tool_name: &str,
        allowed: bool,
    ) -> Result<()>;

    /// Look up the explicit permission for a tool in a conversation, if any.
    async fn get_tool_permission(
        &self,
        conversation_id: &ConversationId,
        tool_name: &str,
    ) -> Result<Option<bool>>;

    /// List all explicit permissions for a single conversation.
    async fn list_tool_permissions(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<HashMap<String, bool>>;
}
