use anyhow::Result;
// use livvi_core::agent::Agent;
// use livvi_core::provider::{FinishReason, MockProvider, ProviderEvent};
use livvi_core::{agent::Agent, provider::{MockProvider, ProviderEvent}, tool::{Input, Toolbox, tool}};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Serialize, Deserialize, JsonSchema)]
struct CalcInput {
    a: i32,
    b: i32,
}

/// A simple calculator tool.
#[tool]
async fn calc(Input(CalcInput { a, b }): Input<CalcInput>) -> i32 {
    a + b
}

pub struct AgentState;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let state = AgentState;
    let mut tools = Toolbox::new();
    tools.add_tool(calc);

    let provider = MockProvider::new(vec![
        ProviderEvent::Token("Hello".to_string()),
        ProviderEvent::Token(", ".to_string()),
        ProviderEvent::Token("world!".to_string()),
    ]);

    let (input_tx, input_rx) = mpsc::channel(256);

    let (mut rx, agent) = Agent::builder()
        .with_provider(Box::new(provider))
        .with_input(input_rx)
        .with_state(state)
        .with_toolbox(tools)
        .build()?;

    input_tx.send(
        livvi_core::interrupt::Interrupt::Message(
            "Hello, world!".to_string()
        )
    ).await?;

    let handle = tokio::spawn(async move {
        loop {
            while let Ok(event) = rx.recv().await {
                println!("Agent event: {:?}", event);
            }

            tokio::time::sleep(std::time::Duration::from_millis(5000)).await;

            input_tx.send(livvi_core::interrupt::Interrupt::Message(
                "Hello, world!".to_string()
            )).await.unwrap();
        }
    });

    tokio::select! {
        _ = shutdown_signal() => {
            tracing::info!("Shutdown signal received, terminating...");
        }
        _ = async {
            let _ = agent.run().await.unwrap();
            let _ = handle.await.unwrap();
        } => {}
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
}
