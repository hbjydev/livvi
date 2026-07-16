pub mod conversation;
pub mod mock;
pub mod person;
pub mod tool_permission;

#[cfg(feature = "sqlite")]
pub mod sqlite;

pub use conversation::{Conversation, ConversationId, ConversationStorage};
pub use mock::MockStore;
pub use person::{Person, PersonId, PersonIdentity, PersonStorage};
pub use tool_permission::{ToolPermission, ToolPermissionStorage};

#[cfg(feature = "sqlite")]
pub use sqlite::LivviSqliteStore;

/// Backend-agnostic entry point for Livvi's persistent storage.
///
/// `LivviStore` is automatically implemented for any type that implements the
/// individual repository traits and is safe to share across threads.
pub trait LivviStore:
    PersonStorage + ConversationStorage + ToolPermissionStorage + Send + Sync + 'static
{
}

impl<T> LivviStore for T where
    T: PersonStorage + ConversationStorage + ToolPermissionStorage + Send + Sync + 'static
{
}
