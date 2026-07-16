pub mod tools;

mod state;

pub use state::DiscordState;

use std::collections::HashSet;

use anyhow::Result;
use livvi_core::interrupt::{AllowToolEvent, Interrupt, ResetEvent};
use serenity::all::{
    CacheHttp, Client, Command, CommandInteraction, CommandOptionType, Context, CreateCommand,
    CreateCommandOption, CreateInteractionResponse, CreateInteractionResponseMessage, EventHandler,
    GatewayIntents, Interaction, Message, Ready,
};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

pub const DISCORD_INSTRUCTIONS: &str = include_str!("./instructions.md");

struct Handler {
    interrupt_tx: mpsc::Sender<Interrupt>,
    allowed_tool_user_ids: HashSet<u64>,
}

#[serenity::async_trait]
impl EventHandler for Handler {
    #[tracing::instrument(
        skip(self, _ctx, msg),
        fields(
            otel.name = "discord.recv_message",
            channel_id = %msg.channel_id,
            author_id = %msg.author.id,
        )
    )]
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

    #[tracing::instrument(
        skip(self, ctx, interaction),
        fields(
            otel.name = "discord.recv_interaction",
        )
    )]
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let Interaction::Command(command) = interaction else {
            return;
        };

        match command.data.name.as_str() {
            "reset" => {
                if let Err(e) = self.handle_reset_command(&ctx, &command).await {
                    error!(error = %e, "failed to handle reset command");
                }
            }
            "allow" => {
                if let Err(e) = self.handle_allow_tool_command(&ctx, &command).await {
                    error!(error = %e, "failed to handle allow tool command");
                }
            }
            _ => {}
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!(bot_username = %ready.user.name, "Discord bot connected");

        let reset_command = CreateCommand::new("reset")
            .description("Wipe the conversation context for this channel");

        let allow_tool_command = CreateCommand::new("allow")
            .description("Allow a tool to run in this channel")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "tool",
                    "Allow a specific tool",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "tool_name",
                        "Name of the tool to allow",
                    )
                    .required(true),
                ),
            );

        if let Err(e) =
            Command::set_global_commands(ctx.http(), vec![reset_command, allow_tool_command]).await
        {
            error!(error = %e, "failed to register Discord slash commands");
        }
    }
}

impl Handler {
    async fn handle_allow_tool_command(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
    ) -> Result<()> {
        if !self.allowed_tool_user_ids.contains(&command.user.id.get()) {
            let response = CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("You are not authorized to use this command.")
                    .ephemeral(true),
            );
            command.create_response(ctx.http(), response).await?;
            return Ok(());
        }

        let tool_name =
            command
                .data
                .options
                .iter()
                .find_map(|opt| match (opt.name.as_str(), &opt.value) {
                    ("tool", serenity::all::CommandDataOptionValue::SubCommand(sub_options)) => {
                        sub_options
                            .iter()
                            .find_map(|sub| match (sub.name.as_str(), &sub.value) {
                                (
                                    "tool_name",
                                    serenity::all::CommandDataOptionValue::String(name),
                                ) => Some(name.clone()),
                                _ => None,
                            })
                    }
                    _ => None,
                });

        let Some(tool_name) = tool_name else {
            let response = CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("Please provide a tool name.")
                    .ephemeral(true),
            );
            command.create_response(ctx.http(), response).await?;
            return Ok(());
        };

        let response = CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content(format!(
                    "Tool `{tool_name}` is now allowed in this channel."
                ))
                .ephemeral(true),
        );
        command.create_response(ctx.http(), response).await?;

        let event = AllowToolEvent::new(
            "discord",
            livvi_core::interrupt::ExternalAuthor {
                transport_kind: "discord".to_string(),
                transport_id: command.user.id.to_string(),
                display_name: Some(command.user.name.clone()),
                metadata: serde_json::json!({
                    "author_name": command.user.name,
                    "discriminator": command.user.discriminator,
                }),
            },
            livvi_core::interrupt::ExternalConversation {
                transport_kind: "discord".to_string(),
                transport_id: command.channel_id.to_string(),
                display_name: None,
                metadata: serde_json::json!({
                    "guild_id": command.guild_id.map(|g| g.to_string()),
                    "is_dm": command.guild_id.is_none(),
                }),
            },
            tool_name,
        );

        if let Err(e) = self.interrupt_tx.send(Interrupt::allow_tool(event)).await {
            error!(error = %e, "failed to forward allow tool command to agent loop");
        }

        Ok(())
    }

    async fn handle_reset_command(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
    ) -> Result<()> {
        let response = CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content("Conversation context reset.")
                .ephemeral(true),
        );
        command.create_response(ctx.http(), response).await?;

        let display_name = command
            .member
            .as_ref()
            .and_then(|m| m.nick.clone())
            .unwrap_or_else(|| command.user.name.clone());

        let event = ResetEvent::new(
            "discord",
            livvi_core::interrupt::ExternalAuthor {
                transport_kind: "discord".to_string(),
                transport_id: command.user.id.to_string(),
                display_name: Some(display_name),
                metadata: serde_json::json!({
                    "author_name": command.user.name,
                    "discriminator": command.user.discriminator,
                }),
            },
            livvi_core::interrupt::ExternalConversation {
                transport_kind: "discord".to_string(),
                transport_id: command.channel_id.to_string(),
                display_name: None,
                metadata: serde_json::json!({
                    "guild_id": command.guild_id.map(|g| g.to_string()),
                    "is_dm": command.guild_id.is_none(),
                }),
            },
        );

        if let Err(e) = self.interrupt_tx.send(Interrupt::reset(event)).await {
            error!(error = %e, "failed to forward reset command to agent loop");
        }

        Ok(())
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
        allowed_tool_user_ids: HashSet<u64>,
    ) -> Result<Self> {
        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT;

        let handler = Handler {
            interrupt_tx,
            allowed_tool_user_ids,
        };

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
