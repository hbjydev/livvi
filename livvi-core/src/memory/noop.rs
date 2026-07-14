use async_trait::async_trait;

use crate::memory::MemoryProvider;

pub struct NoopMemoryProvider;

#[async_trait]
impl MemoryProvider for NoopMemoryProvider {
    async fn remember(
        &self,
        _ctx: crate::memory::MemoryContext,
        _request: crate::memory::RememberRequest,
    ) -> anyhow::Result<Option<crate::memory::Memory>> {
        Err(anyhow::anyhow!("memory provider not configured"))
    }

    async fn recall(
        &self,
        _ctx: crate::memory::MemoryContext,
        _request: crate::memory::RecallRequest,
    ) -> anyhow::Result<Vec<crate::memory::ScoredMemory>> {
        Ok(vec![])
    }

    async fn briefing(
        &self,
        _ctx: crate::memory::MemoryContext,
        _request: crate::memory::BriefingRequest,
    ) -> anyhow::Result<crate::memory::Briefing> {
        Ok(crate::memory::Briefing {
            namespace: String::new(),
            scope_header: None,
            facts: vec![],
            procedures: vec![],
            recent: vec![],
            pinned: vec![],
            children: None,
        })
    }

    async fn get(
        &self,
        _ctx: crate::memory::MemoryContext,
        _id: &str,
    ) -> anyhow::Result<Option<crate::memory::Memory>> {
        Ok(None)
    }

    async fn list(
        &self,
        _ctx: crate::memory::MemoryContext,
        _request: crate::memory::ListRequest,
    ) -> anyhow::Result<Vec<crate::memory::Memory>> {
        Ok(vec![])
    }

    async fn forget(&self, _ctx: crate::memory::MemoryContext, _id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn update(
        &self,
        _ctx: crate::memory::MemoryContext,
        _request: crate::memory::UpdateRequest,
    ) -> anyhow::Result<Option<crate::memory::Memory>> {
        Err(anyhow::anyhow!("memory provider not configured"))
    }

    fn clone_dyn(&self) -> Box<dyn MemoryProvider> {
        Box::new(NoopMemoryProvider)
    }
}
