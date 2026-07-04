use std::fmt;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

/// Canonical identifier for a [`Person`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PersonId(pub String);

impl fmt::Display for PersonId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for PersonId {
    fn from(value: String) -> Self {
        PersonId(value)
    }
}

impl From<&str> for PersonId {
    fn from(value: &str) -> Self {
        PersonId(value.to_string())
    }
}

/// A canonical cross-transport individual that Livvi can interact with.
///
/// A `Person` may represent a human user or another agent. Their transport-specific
/// identities (Discord, Bluesky, etc.) are stored separately via [`PersonIdentity`].
///
/// The `also_known_as` field holds additional display names the person is known by
/// across transports or over time, stored as a comma-separated list in SQLite.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub id: PersonId,
    pub display_name: Option<String>,
    pub also_known_as: Vec<String>,
    pub metadata: Value,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// A link between a [`Person`] and a transport-specific identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonIdentity {
    pub person_id: PersonId,
    pub transport_kind: String,
    pub transport_id: String,
    pub metadata: Value,
    pub linked_at: OffsetDateTime,
}

/// Repository for [`Person`] and [`PersonIdentity`] records.
#[async_trait]
pub trait PersonStorage: Send + Sync + 'static {
    /// Look up a person by their transport identity.
    async fn resolve_identity(
        &self,
        transport_kind: &str,
        transport_id: &str,
    ) -> Result<Option<Person>>;

    /// Create a new person with no linked identities.
    async fn create_person(
        &self,
        display_name: Option<String>,
        also_known_as: Vec<String>,
        metadata: Value,
    ) -> Result<Person>;

    /// Add an alternate display name to a person. If the name is already
    /// present, this is a no-op. Returns the updated person.
    async fn add_also_known_as(&self, person_id: &PersonId, name: String) -> Result<Person>;
    async fn link_identity(
        &self,
        person_id: &PersonId,
        transport_kind: &str,
        transport_id: &str,
        metadata: Value,
    ) -> Result<PersonIdentity>;

    /// Fetch a person by their canonical ID.
    async fn get_person(&self, id: &PersonId) -> Result<Option<Person>>;

    /// Resolve a transport identity, creating a new person and linking the identity
    /// if no match exists.
    async fn ensure_identity(
        &self,
        transport_kind: &str,
        transport_id: &str,
        display_name: Option<String>,
        metadata: Value,
    ) -> Result<Person> {
        if let Some(person) = self.resolve_identity(transport_kind, transport_id).await? {
            return Ok(person);
        }

        let person = self
            .create_person(display_name, Vec::new(), Value::Object(Default::default()))
            .await?;
        self.link_identity(&person.id, transport_kind, transport_id, metadata)
            .await?;

        Ok(person)
    }
}
