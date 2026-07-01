use anyhow::Result;

use crate::{
    model::{Role, Transcript, TranscriptContent, TranscriptItem},
    provider::{Provider, ProviderResponse},
    tool::Tools,
};

pub const MAX_ITERATIONS: usize = 10;

pub struct Agent<P: Provider> {
    provider: P,
    tools: Tools,
}

impl<P: Provider> Agent<P> {
    pub fn new(provider: P, tools: Tools) -> Self {
        Agent { provider, tools }
    }

    pub async fn run(&mut self, user_msg: impl Into<String>) -> Result<String> {
        let mut transcript = Transcript::new();
        transcript.add_item(crate::model::TranscriptItem::user_message(user_msg));

        while transcript.items().len() < MAX_ITERATIONS {
            let response = self.provider.complete(transcript.clone()).await;
            if let Err(e) = response {
                anyhow::bail!("Provider error: {:?}", e);
            }
            let response = response.unwrap();

            match response {
                ProviderResponse::Text(text) => {
                    transcript.add_item(crate::model::TranscriptItem::assistant_message(
                        text.clone(),
                    ));
                    return Ok(text);
                }

                ProviderResponse::ToolCall {
                    tool_name,
                    tool_args,
                    tool_call_id,
                } => {
                    if tool_name.is_empty() {
                        return Err(anyhow::anyhow!("Tool name is empty"));
                    }

                    let tool = self
                        .tools
                        .get_tool(&tool_name)
                        .ok_or_else(|| anyhow::anyhow!("Tool not found: {}", tool_name))?;

                    let validator =
                        jsonschema::validator_for(tool.schema().input_schema.as_value())?;

                    if !validator.is_valid(&tool_args) {
                        anyhow::bail!("Invalid arguments for tool {}: {:?}", tool_name, tool_args);
                    }

                    let result = tool.call(tool_args.clone()).await?;

                    transcript.add_item(TranscriptItem {
                        role: Role::Assistant,
                        content: TranscriptContent::ToolUse {
                            name: tool_name.clone(),
                            id: tool_call_id.clone(),
                            input: tool_args.clone(),
                        },
                    });

                    transcript.add_item(TranscriptItem {
                        role: Role::Assistant,
                        content: TranscriptContent::ToolResult {
                            id: tool_call_id.clone(),
                            content: result.clone(),
                        },
                    });
                }
            };
        }

        Err(anyhow::anyhow!(
            "Max iterations reached without a final response"
        ))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use async_trait::async_trait;
    use schemars::{JsonSchema, schema_for};
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    use crate::{
        agent::Agent,
        provider::{MockProvider, ProviderResponse},
        tool::{Tool, ToolSchema, Tools},
    };

    pub struct CalcTool;

    #[derive(Serialize, Deserialize, JsonSchema)]
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
                input_schema: schema_for!(CalcToolInput),
            }
        }

        async fn call(&self, args: Value) -> Result<String> {
            let input: CalcToolInput = serde_json::from_value(args)?;
            Ok((input.a + input.b).to_string())
        }
    }

    fn setup_agent(responses: Vec<ProviderResponse>) -> Agent<MockProvider> {
        let mut tools = Tools::new();
        tools.add_tool(CalcTool);
        let provider = MockProvider::new(responses);
        Agent::new(provider, tools)
    }

    #[tokio::test]
    async fn test_agent_runs() {
        let mut agent = setup_agent(vec![
            crate::provider::ProviderResponse::ToolCall {
                tool_name: "calc".to_string(),
                tool_args: serde_json::json!({"a": 2, "b": 2}),
                tool_call_id: "call-1".to_string(),
            },
            crate::provider::ProviderResponse::Text("2 + 2 is 4.".to_string()),
        ]);

        let result = agent.run("What's 2+2?").await;

        assert_eq!(result.is_ok(), true);
        assert_eq!(result.unwrap(), "2 + 2 is 4.");
    }

    #[tokio::test]
    async fn test_agent_fails_on_missing_tool_name() {
        let mut agent = setup_agent(vec![
            crate::provider::ProviderResponse::ToolCall {
                tool_name: "".to_string(),
                tool_args: serde_json::json!({"a": 2, "b": 2}),
                tool_call_id: "call-1".to_string(),
            },
            crate::provider::ProviderResponse::Text("2 + 2 is 4.".to_string()),
        ]);

        let result = agent.run("What's 2+2?").await;

        assert_eq!(result.is_err(), true);
        assert_eq!(result.unwrap_err().to_string(), "Tool name is empty");
    }

    #[tokio::test]
    async fn test_agent_fails_on_missing_tool() {
        let mut agent = setup_agent(vec![
            crate::provider::ProviderResponse::ToolCall {
                tool_name: "missing-tool".to_string(),
                tool_args: serde_json::json!({"a": 2, "b": 2}),
                tool_call_id: "call-1".to_string(),
            },
            crate::provider::ProviderResponse::Text("2 + 2 is 4.".to_string()),
        ]);

        let result = agent.run("What's 2+2?").await;

        assert_eq!(result.is_err(), true);
        assert_eq!(
            result.unwrap_err().to_string(),
            "Tool not found: missing-tool"
        );
    }

    #[tokio::test]
    async fn test_agent_fails_on_invalid_tool_args() {
        let mut agent = setup_agent(vec![
            crate::provider::ProviderResponse::ToolCall {
                tool_name: "calc".to_string(),
                tool_args: serde_json::json!({"first": 2, "second": 2}),
                tool_call_id: "call-1".to_string(),
            },
            crate::provider::ProviderResponse::Text("2 + 2 is 4.".to_string()),
        ]);

        let result = agent.run("What's 2+2?").await;

        assert_eq!(result.is_err(), true);
        assert!(
            result
                .unwrap_err()
                .to_string()
                .starts_with("Invalid arguments for tool")
        );
    }
}
