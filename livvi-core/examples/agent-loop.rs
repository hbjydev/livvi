use anyhow::Result;
use livvi_core::agent::Agent;
use livvi_core::provider::{FinishReason, MockProvider, ProviderEvent};
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
        vec![
            ProviderEvent::ToolCallStart {
                id: "call-1".to_string(),
                name: "calc".to_string(),
            },
            ProviderEvent::ToolCallDelta {
                id: "call-1".to_string(),
                arguments: "{\"a\":2,\"b\":2}".to_string(),
            },
            ProviderEvent::ToolCallDone {
                id: "call-1".to_string(),
            },
            ProviderEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            ProviderEvent::TextDelta("2 + 2 is 4.".to_string()),
            ProviderEvent::Done {
                reason: FinishReason::EndTurn,
            },
        ],
    ]);

    let mut agent = Agent::new(provider, tools, ());

    let result = agent.run("Hello, world!").await?;

    for item in result.items().iter() {
        println!("{:?}", item);
    }

    Ok(())
}
