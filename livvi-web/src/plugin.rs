use anyhow::Result;
use livvi_core::plugin::{Plugin, PluginContext};

use crate::WebState;
use crate::tools::{web_fetch, web_search};

/// Self-registering web tools plugin.
///
/// `web_fetch` is always registered; `web_search` is only registered when a
/// SearxNG URL is configured.
pub struct WebPlugin {
    searxng_url: Option<String>,
}

impl WebPlugin {
    pub fn new(searxng_url: Option<String>) -> Self {
        Self { searxng_url }
    }

    /// Build from `LIVVI_SEARXNG_URL`. Always available; without a URL only
    /// `web_fetch` is registered.
    pub fn from_env() -> Self {
        let url = std::env::var("LIVVI_SEARXNG_URL")
            .ok()
            .filter(|s| !s.is_empty());
        Self::new(url)
    }

    /// Whether a SearxNG URL is configured (i.e. `web_search` will be registered).
    pub fn has_search(&self) -> bool {
        self.searxng_url.is_some()
    }
}

impl Plugin for WebPlugin {
    fn name(&self) -> &str {
        "web"
    }

    fn register(self, ctx: &mut PluginContext) -> Result<()> {
        ctx.insert_state(WebState::new(self.searxng_url.clone()));
        ctx.add_tool(web_fetch);
        if self.searxng_url.is_some() {
            ctx.add_tool(web_search);
        }
        Ok(())
    }
}
