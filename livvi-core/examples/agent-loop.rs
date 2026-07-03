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
    let state = AgentState;
    let mut tools = Toolbox::new();
    tools.add_tool(calc);

    let provider = MockProvider::new(vec![
        ProviderEvent::Token("Hello, world!".to_string()),
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
        while let Ok(event) = rx.recv().await {
            println!("Agent event: {:?}", event);
        }
    });

    let _ = agent.run().await?;
    let _ = handle.await?;
    Ok(())
}
