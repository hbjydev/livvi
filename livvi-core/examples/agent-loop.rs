use anyhow::Result;
// use livvi_core::agent::Agent;
// use livvi_core::provider::{FinishReason, MockProvider, ProviderEvent};
use livvi_core::{
    agent::Agent,
    provider::{MockProvider, ProviderEvent},
    tool::{Input, tool},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let provider = MockProvider::new(vec![
        ProviderEvent::Token("Hello".to_string()),
        ProviderEvent::Token(", ".to_string()),
        ProviderEvent::Token("world!".to_string()),
    ]);

    let builder = Agent::builder()
        .with_provider(Box::new(provider))
        .with_tool(calc)
        .with_soul("example soul".to_string());
    let input_tx = builder.interrupt_sender();
    let (mut rx, agent, _tasks) = builder.build()?;

    input_tx
        .send(livvi_core::interrupt::Interrupt::message("Hello, world!"))
        .await?;

    let handle = tokio::spawn(async move {
        loop {
            while let Ok(event) = rx.recv().await {
                println!("Agent event: {:?}", event);
            }

            tokio::time::sleep(std::time::Duration::from_millis(5000)).await;

            input_tx
                .send(livvi_core::interrupt::Interrupt::message("Hello, world!"))
                .await
                .unwrap();
        }
    });

    tokio::select! {
        _ = shutdown_signal() => {
            tracing::info!("Shutdown signal received, terminating...");
        }
        _ = async {
            agent.run().await.unwrap();
            handle.await.unwrap();
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
