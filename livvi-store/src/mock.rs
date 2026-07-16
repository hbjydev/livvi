use std::collections::{HashMap, HashSet};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use time::OffsetDateTime;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::conversation::{Conversation, ConversationId, ConversationStorage};
use crate::person::{Person, PersonId, PersonIdentity, PersonStorage};
use crate::tool_permission::ToolPermissionStorage;

/// In-memory implementation of the storage traits for unit tests.
///
/// All state is held in process memory with `tokio::sync::RwLock`. It is
/// `Send` + `Sync` and implements `LivviStore`, so it can be substituted for
/// `LivviSqliteStore` in any test that depends on storage.
pub struct MockStore {
    persons: RwLock<HashMap<PersonId, Person>>,
    identities: RwLock<HashMap<(String, String), PersonId>>,
    conversations: RwLock<HashMap<ConversationId, Conversation>>,
    conversation_transports: RwLock<HashMap<(String, String), ConversationId>>,
    participants: RwLock<HashMap<ConversationId, HashSet<PersonId>>>,
    tool_permissions: RwLock<HashMap<(ConversationId, String), bool>>,
}

impl MockStore {
    /// Create a new empty mock store.
    pub fn new() -> Self {
        Self {
            persons: RwLock::new(HashMap::new()),
            identities: RwLock::new(HashMap::new()),
            conversations: RwLock::new(HashMap::new()),
            conversation_transports: RwLock::new(HashMap::new()),
            participants: RwLock::new(HashMap::new()),
            tool_permissions: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for MockStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PersonStorage for MockStore {
    async fn resolve_identity(
        &self,
        transport_kind: &str,
        transport_id: &str,
    ) -> Result<Option<Person>> {
        let key = (transport_kind.to_string(), transport_id.to_string());
        let person_id = {
            let identities = self.identities.read().await;
            identities.get(&key).cloned()
        };

        if let Some(person_id) = person_id {
            let persons = self.persons.read().await;
            Ok(persons.get(&person_id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn create_person(
        &self,
        display_name: Option<String>,
        also_known_as: Vec<String>,
        metadata: Value,
    ) -> Result<Person> {
        let id = PersonId(Uuid::new_v4().to_string());
        let now = OffsetDateTime::now_utc();
        let person = Person {
            id: id.clone(),
            display_name,
            also_known_as,
            metadata,
            created_at: now,
            updated_at: now,
        };

        let mut persons = self.persons.write().await;
        persons.insert(id, person.clone());

        Ok(person)
    }

    async fn link_identity(
        &self,
        person_id: &PersonId,
        transport_kind: &str,
        transport_id: &str,
        metadata: Value,
    ) -> Result<PersonIdentity> {
        let key = (transport_kind.to_string(), transport_id.to_string());
        let mut identities = self.identities.write().await;
        identities.insert(key, person_id.clone());

        Ok(PersonIdentity {
            person_id: person_id.clone(),
            transport_kind: transport_kind.to_string(),
            transport_id: transport_id.to_string(),
            metadata,
            linked_at: OffsetDateTime::now_utc(),
        })
    }

    async fn add_also_known_as(&self, person_id: &PersonId, name: String) -> Result<Person> {
        let mut persons = self.persons.write().await;
        let person = persons
            .get_mut(person_id)
            .ok_or_else(|| anyhow::anyhow!("person not found"))?;

        if !person.also_known_as.contains(&name) && person.display_name.as_ref() != Some(&name) {
            person.also_known_as.push(name);
            person.updated_at = OffsetDateTime::now_utc();
        }

        Ok(person.clone())
    }

    async fn get_person(&self, id: &PersonId) -> Result<Option<Person>> {
        let persons = self.persons.read().await;
        Ok(persons.get(id).cloned())
    }
}

#[async_trait]
impl ConversationStorage for MockStore {
    async fn resolve_conversation(
        &self,
        transport_kind: &str,
        transport_id: &str,
    ) -> Result<Option<Conversation>> {
        let key = (transport_kind.to_string(), transport_id.to_string());
        let conversation_id = {
            let conversation_transports = self.conversation_transports.read().await;
            conversation_transports.get(&key).cloned()
        };

        if let Some(conversation_id) = conversation_id {
            let conversations = self.conversations.read().await;
            Ok(conversations.get(&conversation_id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn create_conversation(
        &self,
        transport_kind: &str,
        transport_id: &str,
        title: Option<String>,
        metadata: Value,
    ) -> Result<Conversation> {
        let id = ConversationId(Uuid::new_v4().to_string());
        let now = OffsetDateTime::now_utc();
        let conversation = Conversation {
            id: id.clone(),
            transport_kind: transport_kind.to_string(),
            transport_id: transport_id.to_string(),
            title,
            metadata,
            created_at: now,
            last_active_at: now,
        };

        let key = (transport_kind.to_string(), transport_id.to_string());

        // Lock transport index first, then conversations, to keep a consistent
        // ordering with resolve_conversation and avoid deadlocks.
        let mut conversation_transports = self.conversation_transports.write().await;
        let mut conversations = self.conversations.write().await;
        conversation_transports.insert(key, id.clone());
        conversations.insert(id, conversation.clone());

        Ok(conversation)
    }

    async fn get_conversation(&self, id: &ConversationId) -> Result<Option<Conversation>> {
        let conversations = self.conversations.read().await;
        Ok(conversations.get(id).cloned())
    }

    async fn add_participant(
        &self,
        conversation_id: &ConversationId,
        person_id: &PersonId,
    ) -> Result<()> {
        let mut participants = self.participants.write().await;
        participants
            .entry(conversation_id.clone())
            .or_insert_with(HashSet::new)
            .insert(person_id.clone());
        Ok(())
    }

    async fn get_participants(&self, conversation_id: &ConversationId) -> Result<Vec<Person>> {
        let participant_ids: Vec<PersonId> = {
            let participants = self.participants.read().await;
            participants
                .get(conversation_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect()
        };

        let persons = self.persons.read().await;
        Ok(participant_ids
            .into_iter()
            .filter_map(|id| persons.get(&id).cloned())
            .collect())
    }
}

#[async_trait]
impl ToolPermissionStorage for MockStore {
    async fn set_tool_permission(
        &self,
        conversation_id: &ConversationId,
        tool_name: &str,
        allowed: bool,
    ) -> Result<()> {
        let mut perms = self.tool_permissions.write().await;
        perms.insert((conversation_id.clone(), tool_name.to_string()), allowed);
        Ok(())
    }

    async fn get_tool_permission(
        &self,
        conversation_id: &ConversationId,
        tool_name: &str,
    ) -> Result<Option<bool>> {
        let perms = self.tool_permissions.read().await;
        Ok(perms
            .get(&(conversation_id.clone(), tool_name.to_string()))
            .copied())
    }

    async fn list_tool_permissions(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<HashMap<String, bool>> {
        let perms = self.tool_permissions.read().await;
        Ok(perms
            .iter()
            .filter(|((conv_id, _), _)| conv_id == conversation_id)
            .map(|((_, tool_name), allowed)| (tool_name.clone(), *allowed))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn mock_person_create_and_resolve() {
        let store = MockStore::new();

        let person = store
            .create_person(
                Some("hayden".to_string()),
                vec!["hayden2".to_string()],
                json!({"note": "cool"}),
            )
            .await
            .unwrap();

        assert_eq!(person.also_known_as, vec!["hayden2".to_string()]);

        store
            .link_identity(
                &person.id,
                "discord",
                "12345",
                json!({"discriminator": "0001"}),
            )
            .await
            .unwrap();

        let resolved = store
            .resolve_identity("discord", "12345")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(resolved.id, person.id);
        assert_eq!(resolved.display_name, Some("hayden".to_string()));
    }

    #[tokio::test]
    async fn mock_add_also_known_as_tracks_aliases() {
        let store = MockStore::new();

        let person = store
            .create_person(Some("hayden".to_string()), Vec::new(), json!({}))
            .await
            .unwrap();

        let updated = store
            .add_also_known_as(&person.id, "hayden2".to_string())
            .await
            .unwrap();

        assert_eq!(updated.also_known_as, vec!["hayden2".to_string()]);

        let fetched = store.get_person(&person.id).await.unwrap().unwrap();
        assert_eq!(fetched.also_known_as, vec!["hayden2".to_string()]);
    }

    #[tokio::test]
    async fn mock_ensure_identity_is_idempotent() {
        let store = MockStore::new();

        let first = store
            .ensure_identity("discord", "67890", Some("livvi".to_string()), json!({}))
            .await
            .unwrap();

        let second = store
            .ensure_identity("discord", "67890", Some("livvi".to_string()), json!({}))
            .await
            .unwrap();

        assert_eq!(first.id, second.id);
    }

    #[tokio::test]
    async fn mock_conversation_and_participants() {
        let store = MockStore::new();

        let person = store
            .ensure_identity("discord", "11111", Some("user".to_string()), json!({}))
            .await
            .unwrap();

        let conversation = store
            .ensure_conversation("discord", "chan-1", Some("general".to_string()), json!({}))
            .await
            .unwrap();

        assert_eq!(conversation.transport_id, "chan-1");

        store
            .add_participant(&conversation.id, &person.id)
            .await
            .unwrap();

        let participants = store.get_participants(&conversation.id).await.unwrap();

        assert_eq!(participants.len(), 1);
        assert_eq!(participants[0].id, person.id);
    }
}
