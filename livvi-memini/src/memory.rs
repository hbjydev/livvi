use async_trait::async_trait;
use livvi_core::memory::{
    About, Briefing, BriefingRequest, ListRequest, Memory, MemoryContext, MemoryProvider, PersonId,
    RecallRequest, RememberRequest, ScoredMemory, UpdateRequest,
};

use crate::client::MeminiClient;

/// A Memini-backed `MemoryProvider`.
#[derive(Debug, Clone)]
pub struct MeminiMemoryProvider {
    client: MeminiClient,
    base_namespace: String,
}

impl MeminiMemoryProvider {
    pub fn new(client: MeminiClient, base_namespace: impl Into<String>) -> Self {
        Self {
            client,
            base_namespace: base_namespace.into(),
        }
    }

    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var("MEMINI_BASE_URL")
            .or_else(|_| std::env::var("MEMINI_URL"))
            .ok()?;
        let api_key = std::env::var("MEMINI_API_KEY")
            .or_else(|_| std::env::var("MEMINI_TOKEN"))
            .unwrap_or_default();
        let base_namespace = std::env::var("MEMINI_NAMESPACE")
            .or_else(|_| std::env::var("LIVVI_MEMINI_NAMESPACE"))
            .unwrap_or_else(|_| "livvi".to_string());
        Some(Self::new(
            MeminiClient::new(base_url, api_key),
            base_namespace,
        ))
    }

    fn namespace_for(&self, about: &About) -> String {
        match about {
            About::Person(person_id) => format!("{}/persons/{}", self.base_namespace, person_id),
            About::Conversation(conversation_id) => {
                format!("{}/conversations/{}", self.base_namespace, conversation_id)
            }
            About::Global => self.base_namespace.clone(),
        }
    }

    fn home_namespace_for(&self, caller: Option<&PersonId>) -> Option<String> {
        caller.map(|person_id| format!("{}/persons/{}", self.base_namespace, person_id))
    }
}

#[async_trait]
impl MemoryProvider for MeminiMemoryProvider {
    #[tracing::instrument(skip(self, ctx), fields(otel.name = "create_memory", gen_ai.operation.name = "create_memory"))]
    async fn remember(
        &self,
        ctx: MemoryContext,
        request: RememberRequest,
    ) -> anyhow::Result<Option<Memory>> {
        self.client
            .remember(
                &self.namespace_for(&ctx.about),
                self.home_namespace_for(ctx.caller.as_ref()).as_deref(),
                request,
            )
            .await
    }

    #[tracing::instrument(skip(self, ctx), fields(otel.name = "recall_memory", gen_ai.operation.name = "search_memory"))]
    async fn recall(
        &self,
        ctx: MemoryContext,
        request: RecallRequest,
    ) -> anyhow::Result<Vec<ScoredMemory>> {
        self.client
            .recall(
                &self.namespace_for(&ctx.about),
                self.home_namespace_for(ctx.caller.as_ref()).as_deref(),
                request,
            )
            .await
    }

    #[tracing::instrument(skip(self, ctx), fields(otel.name = "briefing_memory", gen_ai.operation.name = "search_memory"))]
    async fn briefing(
        &self,
        ctx: MemoryContext,
        request: BriefingRequest,
    ) -> anyhow::Result<Briefing> {
        self.client
            .briefing(
                &self.namespace_for(&ctx.about),
                self.home_namespace_for(ctx.caller.as_ref()).as_deref(),
                request,
            )
            .await
    }

    #[tracing::instrument(skip(self, ctx), fields(otel.name = "get_memory", gen_ai.operation.name = "search_memory"))]
    async fn get(&self, ctx: MemoryContext, id: &str) -> anyhow::Result<Option<Memory>> {
        self.client
            .get(
                &self.namespace_for(&ctx.about),
                self.home_namespace_for(ctx.caller.as_ref()).as_deref(),
                id,
            )
            .await
    }

    #[tracing::instrument(skip(self, ctx), fields(otel.name = "list_memory", gen_ai.operation.name = "search_memory"))]
    async fn list(&self, ctx: MemoryContext, request: ListRequest) -> anyhow::Result<Vec<Memory>> {
        self.client
            .list(
                &self.namespace_for(&ctx.about),
                self.home_namespace_for(ctx.caller.as_ref()).as_deref(),
                request,
            )
            .await
    }

    #[tracing::instrument(skip(self, ctx), fields(otel.name = "delete_memory", gen_ai.operation.name = "delete_memory"))]
    async fn forget(&self, ctx: MemoryContext, id: &str) -> anyhow::Result<()> {
        self.client
            .forget(
                &self.namespace_for(&ctx.about),
                self.home_namespace_for(ctx.caller.as_ref()).as_deref(),
                id,
            )
            .await
    }

    #[tracing::instrument(skip(self, ctx), fields(otel.name = "update_memory", gen_ai.operation.name = "update_memory"))]
    async fn update(
        &self,
        ctx: MemoryContext,
        request: UpdateRequest,
    ) -> anyhow::Result<Option<Memory>> {
        self.client
            .update(
                &self.namespace_for(&ctx.about),
                self.home_namespace_for(ctx.caller.as_ref()).as_deref(),
                request,
            )
            .await
    }

    fn clone_dyn(&self) -> Box<dyn MemoryProvider> {
        Box::new(self.clone())
    }
}
