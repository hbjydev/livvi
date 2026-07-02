use anyhow::Result;
use async_trait::async_trait;
use livvi_core::agent::Agent;
use livvi_core::provider::{
    MockProvider, ProviderResponse, ProviderResponseToolCall, ProviderResponseValue,
};
use livvi_core::tool::{Tool, ToolSchema, Tools};
use livvi_openai::OpenAIProvider;
use serde_json::Value;

pub struct CalcTool;

#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct CalcToolInput {
    pub a: i32,
    pub b: i32,
}

#[async_trait]
impl Tool for CalcTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "calc".to_string(),
            description: "A simple calculator tool that can perform addition".to_string(),
            input_schema: schemars::schema_for!(CalcToolInput),
        }
    }

    async fn call(&self, _args: Value) -> Result<String> {
        Ok("4".to_string())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut tools = Tools::new();
    tools.add_tool(CalcTool);

    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY environment variable not set");
    let api_url = std::env::var("OPENAI_API_URL").unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let model_name = std::env::var("OPENAI_MODEL_NAME").unwrap_or_else(|_| "gpt-4".to_string());

    let provider = OpenAIProvider::new(
        &api_key,
        &api_url,
        &model_name,
    )
        .expect("Failed to create OpenAI provider");

    let mut agent = Agent::new(provider, tools);

    let result = agent.run("Hello, world!").await?;
    for item in result.items().iter() {
        println!("{:?}", item);
    }

    Ok(())
}
