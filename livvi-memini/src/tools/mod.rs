use livvi_core::{
    memory::{
        About, Briefing, BriefingRequest, Level, ListRequest, Memory, MemoryContext,
        MemoryProvider, RecallRequest, RememberRequest, Scope, ScoredMemory, Tier, UpdateRequest,
    },
    tool::{Context, Input, State, tool},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn base_namespace() -> String {
    std::env::var("LIVVI_MEMINI_NAMESPACE").unwrap_or_else(|_| "livvi".to_string())
}

fn namespace_for_about(base_namespace: &str, about: &About) -> String {
    match about {
        About::Person(person_id) => format!("{base_namespace}/persons/{person_id}"),
        About::Conversation(conversation_id) => {
            format!("{base_namespace}/conversations/{conversation_id}")
        }
        About::Global => base_namespace.to_string(),
    }
}

fn memory_context_for_about(
    base_namespace: &str,
    about: Option<&About>,
    tool_context: &livvi_core::context::Context,
) -> MemoryContext {
    let mut ctx = MemoryContext::from_tool_context(base_namespace, tool_context);
    if let Some(about) = about {
        ctx.namespace = namespace_for_about(base_namespace, about);
    }
    ctx
}

fn default_scope_full() -> Option<Scope> {
    Some(Scope::Full)
}

fn default_tier_semantic() -> Tier {
    Tier::Semantic
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryRecallInput {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiers: Option<Vec<Tier>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<Vec<Level>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(
        default = "default_scope_full",
        skip_serializing_if = "Option::is_none"
    )]
    pub scope: Option<Scope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<About>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryRememberInput {
    pub content: String,
    #[serde(default = "default_tier_semantic")]
    pub tier: Tier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub metadata: serde_json::Map<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importance: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<Level>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<About>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryBriefingInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_section: Option<usize>,
    #[serde(
        default = "default_scope_full",
        skip_serializing_if = "Option::is_none"
    )]
    pub scope: Option<Scope>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryGetInput {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryListInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiers: Option<Vec<Tier>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<Vec<Level>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryUpdateInput {
    pub id: String,
    pub content: String,
    #[serde(default = "default_tier_semantic")]
    pub tier: Tier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub metadata: serde_json::Map<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importance: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<Level>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<About>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryForgetInput {
    pub id: String,
}

/// Search the persistent memory store for relevant memories.
#[tool]
pub async fn memory_recall(
    Input(input): Input<MemoryRecallInput>,
    State(memory): State<'_, dyn MemoryProvider>,
    Context(agent_context): Context<'_>,
) -> Result<Vec<ScoredMemory>, String> {
    let ctx = memory_context_for_about(&base_namespace(), input.about.as_ref(), agent_context);
    let request = RecallRequest {
        query: input.query,
        tiers: input.tiers,
        levels: input.levels,
        tags: input.tags,
        metadata: None,
        exclude_metadata: None,
        include_fresh_turns: Some(false),
        query_rewrite: None,
        limit: input.limit.or(Some(10)),
        include_expired: Some(false),
        include_superseded: Some(false),
        scope: Some(input.scope.unwrap_or(Scope::Full)),
        namespaces: None,
        as_of: None,
        min_score: None,
        about: input.about,
    };

    memory
        .recall(ctx, request)
        .await
        .map_err(|e| format!("memory recall failed: {e}"))
}

/// Store a new memory in the persistent memory store.
#[tool]
pub async fn memory_remember(
    Input(input): Input<MemoryRememberInput>,
    State(memory): State<'_, dyn MemoryProvider>,
    Context(agent_context): Context<'_>,
) -> Result<Memory, String> {
    let ctx = memory_context_for_about(&base_namespace(), input.about.as_ref(), agent_context);
    let request = RememberRequest {
        content: input.content,
        tier: input.tier,
        summary: input.summary,
        tags: input.tags,
        metadata: input.metadata,
        importance: input.importance,
        level: input.level,
        ttl_seconds: None,
        id: None,
        valid_from: None,
        valid_to: None,
        confidence: None,
        visibility: input.visibility.or_else(|| Some("project".to_string())),
        about: input.about,
    };

    memory
        .remember(ctx, request)
        .await
        .map_err(|e| format!("memory remember failed: {e}"))
}

/// Get a session-start briefing of durable facts, procedures, and pinned memories.
#[tool]
pub async fn memory_briefing(
    Input(input): Input<MemoryBriefingInput>,
    State(memory): State<'_, dyn MemoryProvider>,
    Context(agent_context): Context<'_>,
) -> Result<Briefing, String> {
    let ctx = MemoryContext::from_tool_context(&base_namespace(), agent_context);
    let request = BriefingRequest {
        per_section: input.per_section,
        per_section_pinned: None,
        per_section_facts: None,
        per_section_procedures: None,
        per_section_recent: None,
        scope: Some(input.scope.unwrap_or(Scope::Full)),
        namespaces: None,
    };

    memory
        .briefing(ctx, request)
        .await
        .map_err(|e| format!("memory briefing failed: {e}"))
}

/// Fetch a single memory by its id.
#[tool]
pub async fn memory_get(
    Input(input): Input<MemoryGetInput>,
    State(memory): State<'_, dyn MemoryProvider>,
    Context(agent_context): Context<'_>,
) -> Result<Option<Memory>, String> {
    let ctx = MemoryContext::from_tool_context(&base_namespace(), agent_context);
    memory
        .get(ctx, &input.id)
        .await
        .map_err(|e| format!("memory get failed: {e}"))
}

/// List memories in the current namespace.
#[tool]
pub async fn memory_list(
    Input(input): Input<MemoryListInput>,
    State(memory): State<'_, dyn MemoryProvider>,
    Context(agent_context): Context<'_>,
) -> Result<Vec<Memory>, String> {
    let ctx = MemoryContext::from_tool_context(&base_namespace(), agent_context);
    let request = ListRequest {
        tiers: input.tiers,
        levels: input.levels,
        tags: input.tags,
        metadata: None,
        include_expired: Some(false),
        include_superseded: Some(false),
        limit: input.limit,
        sort: None,
        order: None,
    };

    memory
        .list(ctx, request)
        .await
        .map_err(|e| format!("memory list failed: {e}"))
}

/// Update (upsert) an existing memory by id.
#[tool]
pub async fn memory_update(
    Input(input): Input<MemoryUpdateInput>,
    State(memory): State<'_, dyn MemoryProvider>,
    Context(agent_context): Context<'_>,
) -> Result<Memory, String> {
    let ctx = memory_context_for_about(&base_namespace(), input.about.as_ref(), agent_context);
    let request = UpdateRequest {
        id: input.id,
        content: input.content,
        tier: input.tier,
        summary: input.summary,
        tags: input.tags,
        metadata: input.metadata,
        importance: input.importance,
        level: input.level,
        ttl_seconds: None,
        valid_from: None,
        valid_to: None,
        confidence: None,
        visibility: input.visibility,
        about: input.about,
    };

    memory
        .update(ctx, request)
        .await
        .map_err(|e| format!("memory update failed: {e}"))
}

/// Delete a memory by its id.
#[tool]
pub async fn memory_forget(
    Input(input): Input<MemoryForgetInput>,
    State(memory): State<'_, dyn MemoryProvider>,
    Context(agent_context): Context<'_>,
) -> Result<(), String> {
    let ctx = MemoryContext::from_tool_context(&base_namespace(), agent_context);
    memory
        .forget(ctx, &input.id)
        .await
        .map_err(|e| format!("memory forget failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use livvi_core::context::Context as AgentContext;
    use livvi_store::{ConversationId, PersonId};

    #[test]
    fn memory_recall_input_has_default_scope() {
        let input: MemoryRecallInput = serde_json::from_value(serde_json::json!({
            "query": "hello"
        }))
        .unwrap();
        assert_eq!(input.scope, Some(Scope::Full));
    }

    #[test]
    fn memory_remember_defaults_to_project_visibility() {
        let input: MemoryRememberInput = serde_json::from_value(serde_json::json!({
            "content": "a fact"
        }))
        .unwrap();
        assert_eq!(input.visibility, None);
    }

    #[test]
    fn memory_context_uses_last_user_person() {
        let mut ctx = AgentContext::new("soul", Some(ConversationId::from("conv-1")));
        ctx.push_user("hello", Some(PersonId::from("person-1")));
        ctx.push_assistant("hi", None::<String>);
        ctx.push_user("again", Some(PersonId::from("person-2")));

        let mem_ctx = MemoryContext::from_tool_context("livvi", &ctx);
        assert_eq!(mem_ctx.person_id, Some(PersonId::from("person-2")));
    }
}
