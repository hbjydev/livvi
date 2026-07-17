use anyhow::Result;
use livvi_core::plugin::{Plugin, PluginContext};

use crate::WebState;
use crate::tools::{web_fetch, web_search};

/// Self-registering web tools plugin.
pub struct WebPlugin {
    searxng_url: String,
}

impl WebPlugin {
    pub fn new(searxng_url: impl Into<String>) -> Self {
        Self {
            searxng_url: searxng_url.into(),
        }
    }

    /// Build from `LIVVI_SEARXNG_URL`. Returns `None` when unset or empty.
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("LIVVI_SEARXNG_URL")
            .ok()
            .filter(|s| !s.is_empty())?;
        Some(Self::new(url))
    }
}

impl Plugin for WebPlugin {
    fn name(&self) -> &str {
        "web"
    }

    fn register(self, ctx: &mut PluginContext) -> Result<()> {
        ctx.insert_state(WebState::new(Some(self.searxng_url)));
        ctx.add_tool(web_fetch);
        ctx.add_tool(web_search);
        Ok(())
    }
}
