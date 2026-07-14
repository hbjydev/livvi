pub mod tools;

mod state;

pub use state::DiscordState;

use anyhow::Result;
use livvi_core::interrupt::Interrupt;
use serenity::all::{CacheHttp, Client, Context, EventHandler, GatewayIntents, Message, Ready};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

pub const DISCORD_INSTRUCTIONS: &str = include_str!("./instructions.md");

struct Handler {
    interrupt_tx: mpsc::Sender<Interrupt>,
}

#[serenity::async_trait]
impl EventHandler for Handler {
    async fn message(&self, _ctx: Context, msg: Message) {
        let current_user_id = match _ctx.http().get_current_user().await {
            Ok(user) => user.id,
            Err(e) => {
                error!(error = %e, "Failed to look up current user ID");
                return;
            }
        };

        if msg.author.id == current_user_id {
            return;
        }

        debug!(
            channel_id = %msg.channel_id,
            author_id = %msg.author.id,
            "forwarding Discord message to agent loop"
        );

        let display_name = msg
            .member
            .as_ref()
            .and_then(|m| m.nick.clone())
            .unwrap_or_else(|| msg.author.name.clone());

        let event = Interrupt::external_event(livvi_core::interrupt::ExternalEvent {
            transport_kind: "discord".to_string(),
            event_type: "message".to_string(),
            content: Some(msg.clone().content),
            author: livvi_core::interrupt::ExternalAuthor {
                transport_kind: "discord".to_string(),
                transport_id: msg.author.id.to_string(),
                display_name: Some(display_name),
                metadata: serde_json::json!({
                    "author_name": msg.author.name,
                    "discriminator": msg.author.discriminator,
                }),
            },
            conversation: livvi_core::interrupt::ExternalConversation {
                transport_kind: "discord".to_string(),
                transport_id: msg.channel_id.to_string(),
                display_name: None,
                metadata: serde_json::json!({
                    "guild_id": msg.guild_id.map(|g| g.to_string()),
                    "is_dm": msg.guild_id.is_none(),
                    "message_id": msg.id.to_string(),
                }),
            },
            person_id: None,
            conversation_id: None,
            metadata: serde_json::json!({
                "context": if msg.mentions_me(_ctx.http).await.unwrap_or(false) {
                    "mention"
                } else if msg.guild_id.is_some() {
                    "guild"
                } else {
                    "dm"
                }
            }),
            timestamp: Some(time::OffsetDateTime::now_utc()),
        });

        if let Err(e) = self.interrupt_tx.send(event).await {
            error!(error = %e, "failed to forward Discord message to agent loop");
        }
    }

    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!(bot_username = %ready.user.name, "Discord bot connected");
    }
}

/// A Discord transport that forwards every user message it sees into the
/// Livvi agent loop as an [`Interrupt::ExternalEvent`].
///
/// Create one with [`DiscordTransport::new`], then call [`DiscordTransport::run`]
/// to start the gateway connection. The future resolves only when the gateway
/// shuts down.
pub struct DiscordTransport {
    client: Client,
}

impl DiscordTransport {
    /// Build a new Discord transport that forwards messages into `interrupt_tx`.
    ///
    /// The `token` should be a Discord bot token. The transport requests the
    /// `GUILD_MESSAGES`, `DIRECT_MESSAGES`, and `MESSAGE_CONTENT` intents.
    pub async fn new(
        token: impl AsRef<str>,
        interrupt_tx: mpsc::Sender<Interrupt>,
    ) -> Result<Self> {
        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT;

        let handler = Handler { interrupt_tx };

        let client = Client::builder(token, intents)
            .event_handler(handler)
            .await
            .map_err(|e| anyhow::anyhow!("failed to create Discord client: {e}"))?;

        Ok(Self { client })
    }

    /// Connect to Discord and run until the gateway shuts down.
    pub async fn run(mut self) -> Result<()> {
        self.client
            .start()
            .await
            .map_err(|e| anyhow::anyhow!("Discord gateway error: {e}"))
    }
}
