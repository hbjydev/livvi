use anyhow::Result;
use livvi_core::agent::Agent;
use livvi_core::interrupt::Interrupt;
use livvi_core::tool::{Input, Toolbox, tool};
use livvi_openai::OpenAIChatCompletionsProvider;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema)]
struct CalcInput {
    a: i32,
    b: i32,
}

/// A simple calculator tool that can perform addition.
#[tool]
async fn calc(Input(CalcInput { a, b }): Input<CalcInput>) -> i32 {
    a + b
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let mut tools = Toolbox::new();
    tools.add_tool(calc);

    let api_key = std::env::var("LIVVI_OPENAI_API_KEY")
        .expect("LIVVI_OPENAI_API_KEY environment variable not set");
    let api_url = std::env::var("LIVVI_OPENAI_API_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let model_name =
        std::env::var("LIVVI_OPENAI_MODEL_NAME").unwrap_or_else(|_| "gpt-4".to_string());

    let provider = OpenAIChatCompletionsProvider::new(&api_key, &api_url, &model_name)
        .expect("Failed to create OpenAI provider");

    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let (mut events_rx, agent) = Agent::builder()
        .with_provider(Box::new(provider))
        .with_state(())
        .with_toolbox(tools)
        .with_input(rx)
        .build()?;

    let agent_handle = tokio::spawn(agent.run());

    tx.send(Interrupt::message(
        "Hello there, what's 2+2? Use the calc tool",
    ))
    .await?;

    while let Ok(event) = events_rx.recv().await {
        println!("Agent event: {:?}", event);
    }

    drop(tx);
    agent_handle.await??;

    Ok(())
}
