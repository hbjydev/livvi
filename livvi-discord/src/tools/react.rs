use anyhow::{Context, Result};
use livvi_core::tool::{Input, State, tool};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::DiscordState;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscordReactInput {
    /// The Discord ID of the message to add the reaction to.
    pub message_id: String,
    /// The Discord ID of the channel the message is in.
    pub channel_id: String,
    /// The Unicode emoji to react with.
    pub emoji: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordReactOutput {
    /// Whether or not the reaction was added.
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordReactError {
    pub message: String,
}

/// React to a Discord message in a channel with an emoji. Useful for expressing
/// support or conveying how a message made you feel.
#[tool]
pub async fn discord_react(
    Input(DiscordReactInput {
        message_id,
        channel_id,
        emoji,
    }): Input<DiscordReactInput>,
    State(state): State<'_, DiscordState>,
) -> Result<DiscordReactOutput, DiscordReactError> {
    if emoji.is_empty() || message_id.is_empty() {
        return Err(DiscordReactError {
            message: "Message cannot be empty".to_string(),
        });
    }

    let channel_id = channel_id
        .parse::<u64>()
        .with_context(|| "parsing channel id")
        .map_err(|e| DiscordReactError {
            message: e.to_string(),
        })?;

    let message_id = message_id
        .parse::<u64>()
        .with_context(|| "parsing message id")
        .map_err(|e| DiscordReactError {
            message: e.to_string(),
        })?;

    state
        .send_reaction(channel_id, message_id, emoji)
        .await
        .map_err(|e| DiscordReactError {
            message: format!("Failed to send Discord reaction: {e}"),
        })?;

    Ok(DiscordReactOutput { ok: true })
}
