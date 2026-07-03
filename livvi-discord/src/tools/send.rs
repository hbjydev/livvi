use anyhow::Result;
use livvi_core::tool::{Input, State, tool};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::DiscordState;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscordSendInput {
    pub message: String,
    pub channel_id: String,
    pub reply_to_message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordSendOutput {
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordSendError {
    pub message: String,
}

#[tool]
pub async fn discord_send(
    Input(DiscordSendInput {
        message,
        channel_id,
        reply_to_message_id,
    }): Input<DiscordSendInput>,
    State(state): State<'_, DiscordState>,
) -> Result<DiscordSendOutput, DiscordSendError> {
    if message.is_empty() {
        return Err(DiscordSendError {
            message: "Message cannot be empty".to_string(),
        });
    }

    let channel_id = channel_id.parse::<u64>().map_err(|e| DiscordSendError {
        message: format!("Invalid channel_id: {e}"),
    })?;

    let reply_to_message_id = match reply_to_message_id {
        Some(id) => Some(id.parse::<u64>().map_err(|e| DiscordSendError {
            message: format!("Invalid reply_to_message_id: {e}"),
        })?),
        None => None,
    };

    state
        .send_message(&message, channel_id, reply_to_message_id)
        .await
        .map_err(|e| DiscordSendError {
            message: format!("Failed to send Discord message: {e}"),
        })?;

    Ok(DiscordSendOutput { ok: true })
}
