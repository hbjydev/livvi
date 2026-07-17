use std::sync::Arc;

use anyhow::Result;
use serenity::all::{ChannelId, CreateMessage, Http, Message, MessageId, ReactionType};

/// A Discord-specific state object that wraps the serenity HTTP client.
///
/// This is intended to be passed as the agent state (or a part of it) so that
/// tools like [`discord_send`](crate::tools::discord_send) can send messages
/// without exposing serenity types outside of this crate.
#[derive(Clone)]
pub struct DiscordState {
    http: Arc<Http>,
}

impl AsRef<DiscordState> for DiscordState {
    fn as_ref(&self) -> &DiscordState {
        self
    }
}

impl DiscordState {
    /// Build a new `DiscordState` from a Discord bot token.
    pub fn new(token: impl AsRef<str>) -> Self {
        Self {
            http: Arc::new(Http::new(token.as_ref())),
        }
    }

    /// Send a message to a Discord channel, optionally replying to another
    /// message.
    #[tracing::instrument(
        skip(self, message),
        fields(
            otel.name = "discord.send_message",
            channel_id = channel_id,
            reply_to_message_id = ?reply_to_message_id,
        ),
    )]
    pub async fn send_message(
        &self,
        message: impl AsRef<str>,
        channel_id: u64,
        reply_to_message_id: Option<u64>,
    ) -> Result<()> {
        let mut builder = CreateMessage::new().content(message.as_ref());

        if let Some(reply_id) = reply_to_message_id {
            builder =
                builder.reference_message((ChannelId::new(channel_id), MessageId::new(reply_id)));
        }

        ChannelId::new(channel_id)
            .send_message(&*self.http, builder)
            .await?;

        Ok(())
    }

    /// Fetch a message from a channel, including its attachment metadata.
    ///
    /// Requires the bot to have read access to the channel.
    #[tracing::instrument(
        skip(self),
        fields(
            otel.name = "discord.get_message",
            channel_id = channel_id,
            message_id = message_id,
        ),
    )]
    pub async fn get_message(&self, channel_id: u64, message_id: u64) -> Result<Message> {
        let message = ChannelId::new(channel_id)
            .message(&*self.http, MessageId::new(message_id))
            .await?;

        Ok(message)
    }

    /// Send a reaction to a Discord message.
    #[tracing::instrument(
        skip(self),
        fields(
            otel.name = "discord.send_reaction",
        ),
    )]
    pub async fn send_reaction(
        &self,
        channel_id: u64,
        message_id: u64,
        emoji: String,
    ) -> Result<()> {
        ChannelId::new(channel_id)
            .create_reaction(&*self.http, message_id, ReactionType::Unicode(emoji))
            .await?;

        Ok(())
    }
}
