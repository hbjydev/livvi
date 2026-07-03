use std::env;
use std::sync::Arc;

use anyhow::Result;
use livvi_core::{agent::Agent, interrupt::Interrupt, tool::Toolbox};
use livvi_discord::DISCORD_INSTRUCTIONS;
use livvi_discord::DiscordState;
use livvi_discord::DiscordTransport;
use livvi_discord::tools::discord_send;
use livvi_openai::OpenAIChatCompletionsProvider;
use tokio::sync::mpsc;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    info!("Starting Livvi...");

    let discord_token = env::var("LIVVI_DISCORD_TOKEN")
        .or_else(|_| env::var("DISCORD_TOKEN"))
        .ok();

    let openai_api_key = env::var("LIVVI_OPENAI_API_KEY").ok();
    let openai_model =
        env::var("LIVVI_OPENAI_MODEL_NAME").unwrap_or_else(|_| "gpt-4o-mini".to_string());
    let openai_base_url = env::var("LIVVI_OPENAI_API_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

    // Without a Discord token there is no way to feed the agent loop, so just
    // wait for a shutdown signal.
    let discord_token = match discord_token {
        Some(token) => token,
        None => {
            warn!(
                "No Discord token configured (LIVVI_DISCORD_TOKEN or DISCORD_TOKEN); \
                 waiting for shutdown signal..."
            );
            shutdown_signal().await;
            return Ok(());
        }
    };

    let (interrupt_tx, interrupt_rx) = mpsc::channel::<Interrupt>(256);

    let discord_state = Arc::new(DiscordState::new(&discord_token));
    let transport = DiscordTransport::new(&discord_token, interrupt_tx).await?;

    let provider: Box<dyn livvi_core::provider::Provider> = match openai_api_key {
        Some(key) => Box::new(OpenAIChatCompletionsProvider::new(&key, &openai_base_url, &openai_model)?),
        None => {
            warn!("LIVVI_OPENAI_API_KEY not set; using mock provider");
            Box::new(livvi_core::provider::MockProvider::new(vec![]))
        }
    };

    let (mut agent_events, agent) = Agent::builder()
        .with_provider(provider)
        .with_state(Arc::clone(&discord_state))
        .with_toolbox({
            let mut toolbox = Toolbox::new();
            toolbox.add_tool(discord_send);
            toolbox
        })
        .with_soul(DISCORD_INSTRUCTIONS.to_string())
        .with_input(interrupt_rx)
        .build()?;

    let agent_handle = tokio::spawn(async move {
        if let Err(e) = agent.run().await {
            tracing::error!("agent loop error: {e}");
        }
    });

    let mut discord_handle = tokio::spawn(async move {
        if let Err(e) = transport.run().await {
            tracing::error!("Discord transport error: {e}");
        }
    });

    tokio::select! {
        _ = shutdown_signal() => {
            info!("Shutdown signal received, terminating...");
        }
        _ = agent_handle => {
            warn!("agent loop exited");
        }
        _ = &mut discord_handle => {
            warn!("Discord transport exited");
        }
    }

    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = async {
                if let Some(sigterm) = sigterm.as_mut() {
                    sigterm.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
