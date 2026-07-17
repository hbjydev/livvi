use std::collections::HashSet;

use anyhow::Result;
use livvi_core::plugin::{Plugin, PluginContext};

use crate::tools::{discord_react, discord_send};
use crate::{DISCORD_INSTRUCTIONS, DiscordState, DiscordTransport};

/// Self-registering Discord transport plugin.
pub struct DiscordPlugin {
    token: String,
    allowed_tool_user_ids: HashSet<u64>,
}

impl DiscordPlugin {
    pub fn new(token: impl Into<String>, allowed_tool_user_ids: HashSet<u64>) -> Self {
        Self {
            token: token.into(),
            allowed_tool_user_ids,
        }
    }

    /// Build from `LIVVI_DISCORD_TOKEN` (or `DISCORD_TOKEN`) and
    /// `LIVVI_DISCORD_ALLOW_TOOL_USER_IDS`. Returns `None` when no token is set.
    pub fn from_env() -> Option<Self> {
        let token = std::env::var("LIVVI_DISCORD_TOKEN")
            .or_else(|_| std::env::var("DISCORD_TOKEN"))
            .ok()?;
        let allowed_tool_user_ids = parse_allowed_tool_user_ids(
            &std::env::var("LIVVI_DISCORD_ALLOW_TOOL_USER_IDS").unwrap_or_default(),
        );
        Some(Self::new(token, allowed_tool_user_ids))
    }
}

fn parse_allowed_tool_user_ids(raw: &str) -> HashSet<u64> {
    raw.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<u64>().ok())
        .collect()
}

impl Plugin for DiscordPlugin {
    fn name(&self) -> &str {
        "discord"
    }

    fn register(self, ctx: &mut PluginContext) -> Result<()> {
        ctx.insert_state(DiscordState::new(&self.token));
        ctx.add_tool(discord_send);
        ctx.add_tool(discord_react);
        ctx.add_instructions(DISCORD_INSTRUCTIONS);

        let interrupt_tx = ctx.interrupt_sender();
        let token = self.token;
        let allowed_tool_user_ids = self.allowed_tool_user_ids;
        ctx.spawn_task(async move {
            let transport =
                DiscordTransport::new(token, interrupt_tx, allowed_tool_user_ids).await?;
            transport.run().await
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comma_separated_ids_with_gaps() {
        let ids = parse_allowed_tool_user_ids("1, 2,,3");
        assert_eq!(ids, HashSet::from([1, 2, 3]));
    }

    #[test]
    fn empty_string_yields_no_ids() {
        assert!(parse_allowed_tool_user_ids("").is_empty());
    }

    #[test]
    fn non_numeric_entries_are_skipped() {
        assert!(parse_allowed_tool_user_ids("abc").is_empty());
    }
}
