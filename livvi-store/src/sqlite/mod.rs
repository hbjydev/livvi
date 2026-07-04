use anyhow::Result;
use async_trait::async_trait;
use sqlx::{SqlitePool, types::Json};
use time::OffsetDateTime;
use tracing::instrument;
use uuid::Uuid;

use crate::conversation::{Conversation, ConversationId, ConversationStorage};
use crate::person::{Person, PersonId, PersonIdentity, PersonStorage};

/// SQLite-backed implementation of Livvi's storage traits.
///
/// Use [`LivviSqliteStore::connect`] to create a store from a database URL and
/// run pending migrations. For tests or custom pool setup, use
/// [`LivviSqliteStore::from_pool`].
pub struct LivviSqliteStore {
    pool: SqlitePool,
}

impl LivviSqliteStore {
    /// Connect to a SQLite database and run migrations.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = SqlitePool::connect(database_url).await?;
        sqlx::migrate!().run(&pool).await?;
        Ok(Self { pool })
    }

    /// Create a store from an existing pool.
    ///
    /// This does not run migrations; the caller is responsible for ensuring the
    /// schema is up to date.
    pub fn from_pool(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct PersonRow {
    id: String,
    display_name: Option<String>,
    also_known_as: String,
    metadata: Json<serde_json::Value>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl From<PersonRow> for Person {
    fn from(row: PersonRow) -> Self {
        Person {
            id: PersonId(row.id),
            display_name: row.display_name,
            also_known_as: deserialize_aliases(&row.also_known_as),
            metadata: row.metadata.0,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct ConversationRow {
    id: String,
    transport_kind: String,
    transport_id: String,
    title: Option<String>,
    metadata: Json<serde_json::Value>,
    created_at: OffsetDateTime,
    last_active_at: OffsetDateTime,
}

impl From<ConversationRow> for Conversation {
    fn from(row: ConversationRow) -> Self {
        Conversation {
            id: ConversationId(row.id),
            transport_kind: row.transport_kind,
            transport_id: row.transport_id,
            title: row.title,
            metadata: row.metadata.0,
            created_at: row.created_at,
            last_active_at: row.last_active_at,
        }
    }
}

#[async_trait]
impl PersonStorage for LivviSqliteStore {
    #[instrument(skip(self), level = "trace")]
    async fn resolve_identity(
        &self,
        transport_kind: &str,
        transport_id: &str,
    ) -> Result<Option<Person>> {
        let row = sqlx::query_as::<_, PersonRow>(
            r#"
            SELECT p.id, p.display_name, p.also_known_as, p.metadata, p.created_at, p.updated_at
            FROM persons p
            JOIN person_identities pi ON p.id = pi.person_id
            WHERE pi.transport_kind = ? AND pi.transport_id = ?
            "#,
        )
        .bind(transport_kind)
        .bind(transport_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    #[instrument(skip(self, metadata), level = "trace")]
    async fn create_person(
        &self,
        display_name: Option<String>,
        also_known_as: Vec<String>,
        metadata: serde_json::Value,
    ) -> Result<Person> {
        let id = Uuid::new_v4().to_string();
        let now = OffsetDateTime::now_utc();
        let also_known_as_csv = serialize_aliases(&also_known_as);

        sqlx::query(
            "INSERT INTO persons (id, display_name, also_known_as, metadata, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&display_name)
        .bind(&also_known_as_csv)
        .bind(Json(&metadata))
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(Person {
            id: PersonId(id),
            display_name,
            also_known_as,
            metadata,
            created_at: now,
            updated_at: now,
        })
    }

    #[instrument(skip(self, metadata), level = "trace")]
    async fn link_identity(
        &self,
        person_id: &PersonId,
        transport_kind: &str,
        transport_id: &str,
        metadata: serde_json::Value,
    ) -> Result<PersonIdentity> {
        let now = OffsetDateTime::now_utc();

        sqlx::query(
            "INSERT INTO person_identities (person_id, transport_kind, transport_id, metadata, linked_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&person_id.0)
        .bind(transport_kind)
        .bind(transport_id)
        .bind(Json(&metadata))
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(PersonIdentity {
            person_id: person_id.clone(),
            transport_kind: transport_kind.to_string(),
            transport_id: transport_id.to_string(),
            metadata,
            linked_at: now,
        })
    }

    #[instrument(skip(self), level = "trace")]
    async fn get_person(&self, id: &PersonId) -> Result<Option<Person>> {
        let row = sqlx::query_as::<_, PersonRow>(
            "SELECT id, display_name, also_known_as, metadata, created_at, updated_at FROM persons WHERE id = ?",
        )
        .bind(&id.0)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    #[instrument(skip(self, name), level = "trace")]
    async fn add_also_known_as(&self, person_id: &PersonId, name: String) -> Result<Person> {
        let existing = self.get_person(person_id).await?;
        let mut aliases = existing
            .as_ref()
            .map(|p| p.also_known_as.clone())
            .unwrap_or_default();

        if !aliases.contains(&name)
            && existing.as_ref().and_then(|p| p.display_name.as_ref()) != Some(&name)
        {
            aliases.push(name);
        }

        let csv = serialize_aliases(&aliases);
        let now = OffsetDateTime::now_utc();

        sqlx::query("UPDATE persons SET also_known_as = ?, updated_at = ? WHERE id = ?")
            .bind(&csv)
            .bind(now)
            .bind(&person_id.0)
            .execute(&self.pool)
            .await?;

        self.get_person(person_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("person disappeared after updating aliases"))
    }
}

#[async_trait]
impl ConversationStorage for LivviSqliteStore {
    #[instrument(skip(self), level = "trace")]
    async fn resolve_conversation(
        &self,
        transport_kind: &str,
        transport_id: &str,
    ) -> Result<Option<Conversation>> {
        let row = sqlx::query_as::<_, ConversationRow>(
            "SELECT id, transport_kind, transport_id, title, metadata, created_at, last_active_at FROM conversations WHERE transport_kind = ? AND transport_id = ?",
        )
        .bind(transport_kind)
        .bind(transport_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    #[instrument(skip(self, metadata), level = "trace")]
    async fn create_conversation(
        &self,
        transport_kind: &str,
        transport_id: &str,
        title: Option<String>,
        metadata: serde_json::Value,
    ) -> Result<Conversation> {
        let id = Uuid::new_v4().to_string();
        let now = OffsetDateTime::now_utc();

        sqlx::query(
            "INSERT INTO conversations (id, transport_kind, transport_id, title, metadata, created_at, last_active_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(transport_kind)
        .bind(transport_id)
        .bind(&title)
        .bind(Json(&metadata))
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(Conversation {
            id: ConversationId(id),
            transport_kind: transport_kind.to_string(),
            transport_id: transport_id.to_string(),
            title,
            metadata,
            created_at: now,
            last_active_at: now,
        })
    }

    #[instrument(skip(self), level = "trace")]
    async fn get_conversation(&self, id: &ConversationId) -> Result<Option<Conversation>> {
        let row = sqlx::query_as::<_, ConversationRow>(
            "SELECT id, transport_kind, transport_id, title, metadata, created_at, last_active_at FROM conversations WHERE id = ?",
        )
        .bind(&id.0)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    #[instrument(skip(self), level = "trace")]
    async fn add_participant(
        &self,
        conversation_id: &ConversationId,
        person_id: &PersonId,
    ) -> Result<()> {
        let now = OffsetDateTime::now_utc();

        sqlx::query(
            "INSERT OR IGNORE INTO conversation_participants (conversation_id, person_id, joined_at) VALUES (?, ?, ?)",
        )
        .bind(&conversation_id.0)
        .bind(&person_id.0)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[instrument(skip(self), level = "trace")]
    async fn get_participants(&self, conversation_id: &ConversationId) -> Result<Vec<Person>> {
        let rows = sqlx::query_as::<_, PersonRow>(
            r#"
            SELECT p.id, p.display_name, p.also_known_as, p.metadata, p.created_at, p.updated_at
            FROM persons p
            JOIN conversation_participants cp ON p.id = cp.person_id
            WHERE cp.conversation_id = ?
            "#,
        )
        .bind(&conversation_id.0)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }
}

fn serialize_aliases(aliases: &[String]) -> String {
    aliases.join(",")
}

fn deserialize_aliases(s: &str) -> Vec<String> {
    if s.is_empty() {
        return Vec::new();
    }

    s.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    async fn create_test_store() -> LivviSqliteStore {
        LivviSqliteStore::connect("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn person_create_and_resolve() {
        let store = create_test_store().await;

        let person = store
            .create_person(
                Some("hayden".to_string()),
                vec!["hayden2".to_string()],
                json!({"note": "cool"}),
            )
            .await
            .unwrap();

        assert_eq!(person.display_name, Some("hayden".to_string()));
        assert_eq!(person.also_known_as, vec!["hayden2".to_string()]);
        assert_eq!(person.metadata, json!({"note": "cool"}));

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
    async fn add_also_known_as_tracks_aliases() {
        let store = create_test_store().await;

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
    async fn ensure_identity_is_idempotent() {
        let store = create_test_store().await;

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
    async fn conversation_and_participants() {
        let store = create_test_store().await;

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
