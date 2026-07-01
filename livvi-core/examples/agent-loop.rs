use anyhow::Result;
use async_trait::async_trait;
use livvi_core::agent::Agent;
use livvi_core::provider::{MockProvider, ProviderResponse};
use livvi_core::tool::{Tool, ToolSchema, Tools};
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
            description: "A simple calculator tool".to_string(),
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

    let provider = MockProvider::new(vec![
        ProviderResponse::ToolCall {
            tool_name: "calc".to_string(),
            tool_args: serde_json::json!({"a": 2, "b": 2}),
            tool_call_id: "call-1".to_string(),
        },
        ProviderResponse::Text("2 + 2 is 4.".to_string()),
    ]);

    let mut agent = Agent::new(provider, tools);

    let result = agent.run("Hello, world!").await?;

    println!("{}", result);

    Ok(())
}
