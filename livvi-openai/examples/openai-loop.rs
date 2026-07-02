use anyhow::Result;
use livvi_core::agent::Agent;
use livvi_core::tool::{Input, Tools, tool};
use livvi_openai::OpenAIProvider;
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
    let mut tools = Tools::new();
    tools.add_tool(calc);

    let api_key =
        std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY environment variable not set");
    let api_url =
        std::env::var("OPENAI_API_URL").unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let model_name = std::env::var("OPENAI_MODEL_NAME").unwrap_or_else(|_| "gpt-4".to_string());

    let provider = OpenAIProvider::new(&api_key, &api_url, &model_name)
        .expect("Failed to create OpenAI provider");

    let mut agent = Agent::new(provider, tools, ());

    let result = agent
        .run("Hello there, what's 2+2? Use the calc tool")
        .await?;
    for item in result.items().iter() {
        println!("{:?}", item);
    }

    Ok(())
}
