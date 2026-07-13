use async_trait::async_trait;
use livvi_core::memory::{
    Briefing, BriefingRequest, ListRequest, Memory, MemoryContext, MemoryProvider, RecallRequest,
    RememberRequest, ScoredMemory, UpdateRequest,
};

use crate::client::MeminiClient;

/// A Memini-backed `MemoryProvider`.
#[derive(Debug, Clone)]
pub struct MeminiMemoryProvider {
    client: MeminiClient,
}

impl MeminiMemoryProvider {
    pub fn new(client: MeminiClient) -> Self {
        Self { client }
    }

    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var("MEMINI_BASE_URL")
            .or_else(|_| std::env::var("MEMINI_URL"))
            .ok()?;
        let api_key = std::env::var("MEMINI_API_KEY")
            .or_else(|_| std::env::var("MEMINI_TOKEN"))
            .unwrap_or_default();
        Some(Self::new(MeminiClient::new(base_url, api_key)))
    }
}

#[async_trait]
impl MemoryProvider for MeminiMemoryProvider {
    async fn remember(
        &self,
        ctx: MemoryContext,
        request: RememberRequest,
    ) -> anyhow::Result<Memory> {
        self.client
            .remember(&ctx.namespace, ctx.home_namespace.as_deref(), request)
            .await
    }

    async fn recall(
        &self,
        ctx: MemoryContext,
        request: RecallRequest,
    ) -> anyhow::Result<Vec<ScoredMemory>> {
        self.client
            .recall(&ctx.namespace, ctx.home_namespace.as_deref(), request)
            .await
    }

    async fn briefing(
        &self,
        ctx: MemoryContext,
        request: BriefingRequest,
    ) -> anyhow::Result<Briefing> {
        self.client
            .briefing(&ctx.namespace, ctx.home_namespace.as_deref(), request)
            .await
    }

    async fn get(&self, ctx: MemoryContext, id: &str) -> anyhow::Result<Option<Memory>> {
        self.client
            .get(&ctx.namespace, ctx.home_namespace.as_deref(), id)
            .await
    }

    async fn list(&self, ctx: MemoryContext, request: ListRequest) -> anyhow::Result<Vec<Memory>> {
        self.client
            .list(&ctx.namespace, ctx.home_namespace.as_deref(), request)
            .await
    }

    async fn forget(&self, ctx: MemoryContext, id: &str) -> anyhow::Result<()> {
        self.client
            .forget(&ctx.namespace, ctx.home_namespace.as_deref(), id)
            .await
    }

    async fn update(&self, ctx: MemoryContext, request: UpdateRequest) -> anyhow::Result<Memory> {
        self.client
            .update(&ctx.namespace, ctx.home_namespace.as_deref(), request)
            .await
    }

    fn clone_dyn(&self) -> Box<dyn MemoryProvider> {
        Box::new(self.clone())
    }
}
