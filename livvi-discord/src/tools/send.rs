use anyhow::Result;
use livvi_core::tool::{Input, tool};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
) -> Result<DiscordSendOutput, DiscordSendError> {
    if message.is_empty() {
        return Err(DiscordSendError {
            message: "Message cannot be empty".to_string(),
        });
    }

    tracing::info!(
        "Sending message to Discord channel {}: {}",
        channel_id,
        message
    );
    if let Some(reply_to) = reply_to_message_id {
        tracing::info!("Replying to message ID: {}", reply_to);
    }

    Ok(DiscordSendOutput { ok: true })
}
