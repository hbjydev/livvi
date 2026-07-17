use anyhow::Result;
use livvi_core::plugin::{Plugin, PluginContext};

use crate::{MeminiClient, MeminiMemoryProvider};

/// Self-registering Memini memory plugin.
pub struct MeminiPlugin {
    client: MeminiClient,
    namespace: String,
}

impl MeminiPlugin {
    pub fn new(client: MeminiClient, namespace: impl Into<String>) -> Self {
        Self {
            client,
            namespace: namespace.into(),
        }
    }

    /// Build from `LIVVI_MEMINI_BASE_URL`, `LIVVI_MEMINI_API_KEY`, and
    /// `LIVVI_MEMINI_NAMESPACE` (default `"livvi"`). Returns `None` unless both the
    /// base URL and API key are set and non-empty.
    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var("LIVVI_MEMINI_BASE_URL")
            .ok()
            .filter(|u| !u.is_empty())?;
        let api_key = std::env::var("LIVVI_MEMINI_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())?;
        let namespace =
            std::env::var("LIVVI_MEMINI_NAMESPACE").unwrap_or_else(|_| "livvi".to_string());
        Some(Self::new(MeminiClient::new(base_url, api_key), namespace))
    }
}

impl Plugin for MeminiPlugin {
    fn name(&self) -> &str {
        "memini"
    }

    fn register(self, ctx: &mut PluginContext) -> Result<()> {
        ctx.set_memory_provider(MeminiMemoryProvider::new(self.client, self.namespace));
        Ok(())
    }
}
