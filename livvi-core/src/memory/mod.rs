use std::{fmt, str::FromStr};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub use livvi_store::{ConversationId, PersonId};

/// Agent-facing instructions for the memory system.
///
/// These instructions are always loaded into the agent's system prompt so that
/// memory tools are discoverable and used consistently, regardless of which
/// transport or persona is in use.
pub const MEMORY_INSTRUCTIONS: &str = include_str!("instructions.md");

/// Provenance and scope for a memory operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryContext {
    /// The target scope of the operation.
    pub about: About,
    /// The person initiating the operation, if known.
    pub caller: Option<PersonId>,
}

impl MemoryContext {
    /// Build a `MemoryContext` for a specific target and caller.
    pub fn new(about: About, caller: Option<PersonId>) -> Self {
        Self { about, caller }
    }

    /// Build a `MemoryContext` from the current agent conversation context.
    ///
    /// Defaults the target scope to the current conversation and the caller to the
    /// most recent user message that carries a `person_id`.
    pub fn from_tool_context(context: &crate::context::Context) -> Self {
        let about = context
            .conversation_id
            .clone()
            .map(About::Conversation)
            .unwrap_or(About::Global);
        let caller = context
            .turns
            .iter()
            .rev()
            .find(|m| matches!(m.role, crate::model::Role::User))
            .and_then(|m| m.person_id.clone());

        Self { about, caller }
    }
}

/// Internal scope selector for memory operations.
///
/// `About` tells a memory tool which person, conversation, or global scope to target.
/// When serialized as JSON, it is a single string: `"global"`, `"person:<id>"`, or
/// `"conversation:<id>"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum About {
    /// Target a specific person by their canonical person ID.
    Person(PersonId),
    /// Target a specific conversation by its canonical conversation ID.
    Conversation(ConversationId),
    /// Target the global scope, i.e. memories not tied to a person or conversation.
    Global,
}

impl Serialize for About {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            About::Global => serializer.serialize_str("global"),
            About::Person(id) => serializer.serialize_str(&format!("person:{id}")),
            About::Conversation(id) => serializer.serialize_str(&format!("conversation:{id}")),
        }
    }
}

impl<'de> Deserialize<'de> for About {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value == "global" {
            Ok(About::Global)
        } else if let Some(id) = value.strip_prefix("person:") {
            Ok(About::Person(PersonId(id.to_string())))
        } else if let Some(id) = value.strip_prefix("conversation:") {
            Ok(About::Conversation(ConversationId(id.to_string())))
        } else {
            Err(serde::de::Error::custom(format!(
                "invalid About value: {value}. expected 'global', 'person:<id>' or 'conversation:<id>'"
            )))
        }
    }
}

impl schemars::JsonSchema for About {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "About".into()
    }

    fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        serde_json::from_value(serde_json::json!({
            "description": "Scope selector for a memory operation. Use 'global' for everyone, 'person:<id>' for a specific person, or 'conversation:<id>' for a specific conversation.",
            "type": "string"
        }))
        .expect("About schema is valid")
    }
}

/// Tier of a memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Working,
    Episodic,
    Semantic,
    Procedural,
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tier::Working => write!(f, "working"),
            Tier::Episodic => write!(f, "episodic"),
            Tier::Semantic => write!(f, "semantic"),
            Tier::Procedural => write!(f, "procedural"),
        }
    }
}

impl FromStr for Tier {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "working" => Ok(Tier::Working),
            "episodic" => Ok(Tier::Episodic),
            "semantic" => Ok(Tier::Semantic),
            "procedural" => Ok(Tier::Procedural),
            _ => Err(format!("unknown tier: {s}")),
        }
    }
}

/// Derivation level of a memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Explicit,
    Deduced,
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Level::Explicit => write!(f, "explicit"),
            Level::Deduced => write!(f, "deduced"),
        }
    }
}

impl FromStr for Level {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "explicit" => Ok(Level::Explicit),
            "deduced" => Ok(Level::Deduced),
            _ => Err(format!("unknown level: {s}")),
        }
    }
}

/// Recall/briefing scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Project,
    Full,
    Everywhere,
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scope::Project => write!(f, "project"),
            Scope::Full => write!(f, "full"),
            Scope::Everywhere => write!(f, "everywhere"),
        }
    }
}

impl FromStr for Scope {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "project" => Ok(Scope::Project),
            "full" => Ok(Scope::Full),
            "everywhere" => Ok(Scope::Everywhere),
            _ => Err(format!("unknown scope: {s}")),
        }
    }
}

/// Request to store a memory.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RememberRequest {
    pub content: String,
    #[serde(default = "Tier::default_working")]
    pub tier: Tier,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub metadata: serde_json::Map<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub importance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<Level>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(skip_serializing)]
    pub about: Option<About>,
}

impl Tier {
    fn default_working() -> Tier {
        Tier::Working
    }
}

/// Request to recall memories.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RecallRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiers: Option<Vec<Tier>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<Vec<Level>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_metadata: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_fresh_turns: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_rewrite: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_expired: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_superseded: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespaces: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_score: Option<f64>,
    #[serde(skip_serializing)]
    pub about: Option<About>,
}

/// Request a session-start briefing.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BriefingRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_section: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_section_pinned: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_section_facts: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_section_procedures: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_section_recent: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespaces: Option<Vec<String>>,
}

/// Request a list of memories in a namespace.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ListRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiers: Option<Vec<Tier>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<Vec<Level>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_expired: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_superseded: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
}

/// Request to update (upsert) an existing memory.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UpdateRequest {
    pub id: String,
    pub content: String,
    #[serde(default = "Tier::default_working")]
    pub tier: Tier,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub metadata: serde_json::Map<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub importance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<Level>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(skip_serializing)]
    pub about: Option<About>,
}

impl From<UpdateRequest> for RememberRequest {
    fn from(update: UpdateRequest) -> Self {
        Self {
            content: update.content,
            tier: update.tier,
            summary: update.summary,
            tags: update.tags,
            metadata: update.metadata,
            importance: update.importance,
            level: update.level,
            ttl_seconds: update.ttl_seconds,
            id: Some(update.id),
            valid_from: update.valid_from,
            valid_to: update.valid_to,
            confidence: update.confidence,
            visibility: update.visibility,
            about: update.about,
        }
    }
}

/// A stored memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub namespace: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub tier: Tier,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<Level>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub importance: f64,
    #[serde(default, with = "time::serde::iso8601::option")]
    pub created_at: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::iso8601::option")]
    pub updated_at: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::iso8601::option")]
    pub valid_from: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::iso8601::option")]
    pub valid_to: Option<OffsetDateTime>,
}

/// A memory returned from recall, with a relevance score and provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredMemory {
    pub memory: Memory,
    pub score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// One item in a briefing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingItem {
    pub memory: Memory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// One child namespace rollup in a briefing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingChild {
    pub namespace: String,
    pub total: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pinned: Vec<Memory>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<Memory>,
}

/// Session-start briefing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Briefing {
    pub namespace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_header: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<BriefingItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub procedures: Vec<BriefingItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<BriefingItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pinned: Vec<BriefingItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<BriefingChild>>,
}

impl Briefing {
    /// Render the briefing as a single system prompt string.
    pub fn to_system_prompt(&self) -> String {
        let mut sections = vec!["Memory briefing:".to_string()];

        if !self.pinned.is_empty() {
            sections.push("Pinned:".to_string());
            for item in &self.pinned {
                sections.push(format!("- {}", item.memory.content));
            }
        }

        if !self.facts.is_empty() {
            sections.push("Durable facts:".to_string());
            for item in &self.facts {
                sections.push(format!("- {}", item.memory.content));
            }
        }

        if !self.procedures.is_empty() {
            sections.push("Procedures:".to_string());
            for item in &self.procedures {
                sections.push(format!("- {}", item.memory.content));
            }
        }

        if !self.recent.is_empty() {
            sections.push("Recent activity:".to_string());
            for item in &self.recent {
                sections.push(format!("- {}", item.memory.content));
            }
        }

        if sections.len() == 1 {
            return String::new();
        }

        sections.join("\n")
    }
}

/// Backend-agnostic memory provider.
#[async_trait]
pub trait MemoryProvider: Send + Sync + 'static {
    async fn remember(&self, ctx: MemoryContext, request: RememberRequest) -> Result<Memory>;
    async fn recall(&self, ctx: MemoryContext, request: RecallRequest)
    -> Result<Vec<ScoredMemory>>;
    async fn briefing(&self, ctx: MemoryContext, request: BriefingRequest) -> Result<Briefing>;
    async fn get(&self, ctx: MemoryContext, id: &str) -> Result<Option<Memory>>;
    async fn list(&self, ctx: MemoryContext, request: ListRequest) -> Result<Vec<Memory>>;
    async fn forget(&self, ctx: MemoryContext, id: &str) -> Result<()>;
    async fn update(&self, ctx: MemoryContext, request: UpdateRequest) -> Result<Memory>;

    fn clone_dyn(&self) -> Box<dyn MemoryProvider>;
}

#[async_trait]
impl MemoryProvider for Box<dyn MemoryProvider> {
    async fn remember(&self, ctx: MemoryContext, request: RememberRequest) -> Result<Memory> {
        self.as_ref().remember(ctx, request).await
    }

    async fn recall(
        &self,
        ctx: MemoryContext,
        request: RecallRequest,
    ) -> Result<Vec<ScoredMemory>> {
        self.as_ref().recall(ctx, request).await
    }

    async fn briefing(&self, ctx: MemoryContext, request: BriefingRequest) -> Result<Briefing> {
        self.as_ref().briefing(ctx, request).await
    }

    async fn get(&self, ctx: MemoryContext, id: &str) -> Result<Option<Memory>> {
        self.as_ref().get(ctx, id).await
    }

    async fn list(&self, ctx: MemoryContext, request: ListRequest) -> Result<Vec<Memory>> {
        self.as_ref().list(ctx, request).await
    }

    async fn forget(&self, ctx: MemoryContext, id: &str) -> Result<()> {
        self.as_ref().forget(ctx, id).await
    }

    async fn update(&self, ctx: MemoryContext, request: UpdateRequest) -> Result<Memory> {
        self.as_ref().update(ctx, request).await
    }

    fn clone_dyn(&self) -> Box<dyn MemoryProvider> {
        self.as_ref().clone_dyn()
    }
}

impl Clone for Box<dyn MemoryProvider> {
    fn clone(&self) -> Self {
        self.clone_dyn()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_display_round_trips() {
        assert_eq!(Tier::Episodic.to_string(), "episodic");
        assert_eq!("semantic".parse::<Tier>().unwrap(), Tier::Semantic);
    }

    #[test]
    fn memory_context_derives_from_tool_context() {
        let mut ctx = crate::context::Context::new("soul", Some(ConversationId::from("conv-1")));
        ctx.push_user("hello", Some(PersonId::from("person-1")));

        let mem_ctx = MemoryContext::from_tool_context(&ctx);
        assert_eq!(
            mem_ctx.about,
            About::Conversation(ConversationId::from("conv-1"))
        );
        assert_eq!(mem_ctx.caller, Some(PersonId::from("person-1")));
    }

    #[test]
    fn about_deserializes_person_string() {
        let value = serde_json::json!("person:c257674e-835a-457b-abb7-9ce6259e4f37");
        let about: About = serde_json::from_value(value).expect("should deserialize person string");
        assert_eq!(
            about,
            About::Person(PersonId::from("c257674e-835a-457b-abb7-9ce6259e4f37"))
        );
    }

    #[test]
    fn about_serializes_to_string() {
        assert_eq!(
            serde_json::to_value(About::Person(PersonId::from(
                "c257674e-835a-457b-abb7-9ce6259e4f37"
            )))
            .unwrap(),
            serde_json::json!("person:c257674e-835a-457b-abb7-9ce6259e4f37")
        );
        assert_eq!(
            serde_json::to_value(About::Conversation(ConversationId::from(
                "647f75c2-0c7f-438e-8e95-f1606390c4de"
            )))
            .unwrap(),
            serde_json::json!("conversation:647f75c2-0c7f-438e-8e95-f1606390c4de")
        );
        assert_eq!(
            serde_json::to_value(About::Global).unwrap(),
            serde_json::json!("global")
        );
    }

    #[test]
    fn about_string_validates_against_schema() {
        let schema = schemars::schema_for!(About);
        let validator = jsonschema::validator_for(schema.as_value()).unwrap();

        let person_value = serde_json::json!("person:c257674e-835a-457b-abb7-9ce6259e4f37");
        validator
            .validate(&person_value)
            .expect("person string should validate");

        let conversation_value = serde_json::json!("conversation:647f75c2-0c7f-438e-8e95-f1606390c4de");
        validator
            .validate(&conversation_value)
            .expect("conversation string should validate");

        let global_value = serde_json::json!("global");
        validator
            .validate(&global_value)
            .expect("global string should validate");
    }

    #[test]
    fn remember_request_with_about_person_validates() {
        let schema = schemars::schema_for!(RememberRequest);
        let validator = jsonschema::validator_for(schema.as_value()).unwrap();
        let value = serde_json::json!({
            "content": "a memory",
            "about": "person:c257674e-835a-457b-abb7-9ce6259e4f37"
        });
        validator
            .validate(&value)
            .expect("remember request with about person should validate");
    }
}
