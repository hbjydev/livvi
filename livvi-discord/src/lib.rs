pub mod tools;

mod state;

pub use state::DiscordState;

use anyhow::Result;
use livvi_core::interrupt::Interrupt;
use serenity::all::{Client, Context, EventHandler, GatewayIntents, Message, Ready};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

struct Handler {
    interrupt_tx: mpsc::Sender<Interrupt>,
}

#[serenity::async_trait]
impl EventHandler for Handler {
    async fn message(&self, _ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        debug!(
            channel_id = %msg.channel_id,
            author_id = %msg.author.id,
            "forwarding Discord message to agent loop"
        );

        if let Err(e) = self
            .interrupt_tx
            .send(Interrupt::Message(msg.content))
            .await
        {
            error!(error = %e, "failed to forward Discord message to agent loop");
        }
    }

    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!(bot_username = %ready.user.name, "Discord bot connected");
    }
}

/// A Discord transport that forwards every user message it sees into the
/// Livvi agent loop as an [`Interrupt::Message`].
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
