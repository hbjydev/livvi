use std::{fmt, str::FromStr};

use anyhow::Result;
use async_trait::async_trait;
use livvi_store::{ConversationId, PersonId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Provenance and namespace information for a memory operation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryContext {
    /// The primary Memini namespace for the operation (e.g. `livvi/conversations/<id>`).
    pub namespace: String,
    /// The caller's home namespace (e.g. `livvi/persons/<id>`), used for `visibility: personal`.
    pub home_namespace: Option<String>,
    /// The canonical person associated with the operation, if known.
    pub person_id: Option<PersonId>,
    /// The canonical conversation associated with the operation, if known.
    pub conversation_id: Option<ConversationId>,
}

impl MemoryContext {
    /// Build a `MemoryContext` directly from the namespace components.
    pub fn new(
        base_namespace: &str,
        conversation_id: &ConversationId,
        person_id: Option<&PersonId>,
    ) -> Self {
        let namespace = format!("{base_namespace}/conversations/{conversation_id}");
        let home_namespace = person_id.map(|p| format!("{base_namespace}/persons/{p}"));
        Self {
            namespace,
            home_namespace,
            person_id: person_id.cloned(),
            conversation_id: Some(conversation_id.clone()),
        }
    }

    /// Build a `MemoryContext` from the current agent conversation context.
    ///
    /// The conversation namespace is derived from `conversation_id` and the person namespace
    /// from the most recent user message that carries a `person_id`.
    pub fn from_tool_context(base_namespace: &str, context: &crate::context::Context) -> Self {
        let conversation_id = context
            .conversation_id
            .clone()
            .unwrap_or_else(|| ConversationId::from("global"));
        let person_id = context
            .turns
            .iter()
            .rev()
            .find(|m| matches!(m.role, crate::model::Role::User))
            .and_then(|m| m.person_id.clone());

        let namespace = format!("{base_namespace}/conversations/{conversation_id}");
        let home_namespace = person_id
            .as_ref()
            .map(|p| format!("{base_namespace}/persons/{p}"));

        Self {
            namespace,
            home_namespace,
            person_id,
            conversation_id: Some(conversation_id),
        }
    }
}

/// Internal scope selector for memory operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum About {
    Person(PersonId),
    Conversation(ConversationId),
    Global,
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

        let mem_ctx = MemoryContext::from_tool_context("livvi", &ctx);
        assert_eq!(mem_ctx.namespace, "livvi/conversations/conv-1");
        assert_eq!(
            mem_ctx.home_namespace,
            Some("livvi/persons/person-1".to_string())
        );
        assert_eq!(mem_ctx.person_id, Some(PersonId::from("person-1")));
    }
}
