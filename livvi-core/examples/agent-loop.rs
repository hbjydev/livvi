use anyhow::Result;
use livvi_core::agent::Agent;
use livvi_core::provider::{
    MockProvider, ProviderResponse, ProviderResponseToolCall, ProviderResponseValue,
};
use livvi_core::tool::{Input, Tools, tool};
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
    let mut tools = Tools::new();
    tools.add_tool(calc);

    let provider = MockProvider::new(vec![
        ProviderResponse {
            value: ProviderResponseValue::ToolCalls(vec![ProviderResponseToolCall {
                tool_name: "calc".to_string(),
                tool_args: serde_json::json!({"a": 2, "b": 2}),
                tool_call_id: "call-1".to_string(),
            }]),
            input_tokens: 0,
            output_tokens: 0,
            reasoning_tokens: 0,
        },
        ProviderResponse {
            value: ProviderResponseValue::Text("2 + 2 is 4.".to_string()),
            input_tokens: 0,
            output_tokens: 0,
            reasoning_tokens: 0,
        },
    ]);

    let mut agent = Agent::new(provider, tools, ());

    let result = agent.run("Hello, world!").await?;

    for item in result.items().iter() {
        println!("{:?}", item);
    }

    Ok(())
}
