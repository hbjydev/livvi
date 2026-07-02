use anyhow::Result;

use crate::{
    model::{ToolCall, ToolResult, Transcript, TranscriptItem},
    provider::{Provider, ProviderResponseToolCall, ProviderResponseValue},
    tool::Tools,
};

pub const MAX_ITERATIONS: usize = 10;

pub const AGENT_INSTRUCTIONS: &str = include_str!("../prompts/instructions.md");

/// An Agent is responsible for managing the interaction between a user, a
/// provider, and a set of tools. It maintains a transcript of the conversation
/// and handles tool calls as requested by the provider.
pub struct Agent<P: Provider> {
    provider: P,
    tools: Tools,
}

impl<P: Provider> Agent<P> {
    /// Creates a new Agent with the given provider and tools.
    pub fn new(provider: P, tools: Tools) -> Self {
        Agent { provider, tools }
    }

    /// Returns the system prompt for the agent. This can be overridden to
    /// provide a custom system prompt.
    fn system_prompt(&self) -> String {
        format!("{}\n\n{}", "# SOUL.md", AGENT_INSTRUCTIONS)
    }

    /// Runs the agent with the given user message. The agent will interact with
    /// the provider and tools to generate a response. The transcript of the
    /// interaction is returned.
    pub async fn run(&mut self, user_msg: impl Into<String>) -> Result<Transcript> {
        let mut transcript = Transcript::new();
        transcript.add_item(crate::model::TranscriptItem::system_message(
            self.system_prompt(),
        ));
        transcript.add_item(crate::model::TranscriptItem::user_message(user_msg));

        while transcript.items().len() < MAX_ITERATIONS {
            let response = self
                .provider
                .complete(transcript.clone(), self.tools.clone())
                .await;
            if let Err(e) = response {
                anyhow::bail!("Provider error: {:?}", e);
            }
            let response = response.unwrap();

            match response.value {
                ProviderResponseValue::Text(text) => {
                    transcript.add_item(crate::model::TranscriptItem::assistant_message(
                        text.clone(),
                    ));
                    return Ok(transcript);
                }

                ProviderResponseValue::Reasoning(text) => {
                    transcript.add_item(crate::model::TranscriptItem::assistant_reasoning(
                        text.clone(),
                    ));
                    continue;
                }

                ProviderResponseValue::ToolCalls(calls) => {
                    for ProviderResponseToolCall {
                        tool_name,
                        tool_args,
                        tool_call_id,
                    } in calls
                    {
                        if tool_name.is_empty() {
                            transcript.add_item(TranscriptItem::tool_result(ToolResult {
                                id: tool_call_id.clone(),
                                content: "Tool name is empty".into(),
                                is_error: true,
                            }));
                            continue;
                        }

                        let tool = self.tools.get_tool(&tool_name);

                        if tool.is_none() {
                            transcript.add_item(TranscriptItem::tool_result(ToolResult {
                                id: tool_call_id.clone(),
                                content: format!("Tool does not exist: {:?}", tool_name),
                                is_error: true,
                            }));
                            continue;
                        }
                        let tool = tool.unwrap();

                        if let Err(e) = tool.validate_input(&tool_args) {
                            transcript.add_item(TranscriptItem::tool_result(ToolResult {
                                id: tool_call_id.clone(),
                                content: e.to_string(),
                                is_error: true,
                            }));
                            continue;
                        }

                        transcript.add_item(TranscriptItem::assistant_tool_call(ToolCall {
                            name: tool_name.clone(),
                            id: tool_call_id.clone(),
                            input: tool_args.clone(),
                        }));

                        match tool.call(tool_args.clone()).await {
                            Ok(result) => {
                                transcript.add_item(TranscriptItem::tool_result(ToolResult {
                                    id: tool_call_id.clone(),
                                    content: result.clone(),
                                    is_error: false,
                                }));
                            }
                            Err(e) => {
                                transcript.add_item(TranscriptItem::tool_result(ToolResult {
                                    id: tool_call_id.clone(),
                                    content: format!("Tool call failed: {:?}", e),
                                    is_error: true,
                                }));
                            }
                        }
                    }
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
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    use crate::{
        agent::Agent,
        model::{Role, ToolResult, TranscriptContent},
        provider::{
            MockProvider, ProviderResponse, ProviderResponseToolCall, ProviderResponseValue,
        },
        tool::{Tool, ToolSchema, Tools},
    };

    #[derive(ToolSchema)]
    #[tool {
        name = "calc",
        input = CalcToolInput,
    }]
    /// A simple calculator tool
    pub struct CalcTool;

    #[derive(Serialize, Deserialize, JsonSchema)]
    pub struct CalcToolInput {
        pub a: i32,
        pub b: i32,
    }

    #[async_trait]
    impl Tool for CalcTool {
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
            crate::provider::ProviderResponse {
                value: ProviderResponseValue::ToolCalls(vec![ProviderResponseToolCall {
                    tool_name: "calc".to_string(),
                    tool_args: serde_json::json!({"a": 2, "b": 2}),
                    tool_call_id: "call-1".to_string(),
                }]),
                input_tokens: 0,
                output_tokens: 0,
                reasoning_tokens: 0,
            },
            crate::provider::ProviderResponse {
                value: ProviderResponseValue::Text("2 + 2 is 4.".to_string()),
                input_tokens: 0,
                output_tokens: 0,
                reasoning_tokens: 0,
            },
        ]);

        let result = agent.run("What's 2+2?").await;

        assert!(result.is_ok());

        let result = result.unwrap();
        assert_eq!(
            result.items().last().unwrap().blocks.last().unwrap(),
            &TranscriptContent::Text("2 + 2 is 4.".into()),
        );
    }

    #[tokio::test]
    async fn test_agent_fails_on_missing_tool_name() {
        let mut agent = setup_agent(vec![
            crate::provider::ProviderResponse {
                value: ProviderResponseValue::ToolCalls(vec![ProviderResponseToolCall {
                    tool_name: "".to_string(),
                    tool_args: serde_json::json!({"a": 2, "b": 2}),
                    tool_call_id: "call-1".to_string(),
                }]),
                input_tokens: 0,
                output_tokens: 0,
                reasoning_tokens: 0,
            },
            crate::provider::ProviderResponse {
                value: ProviderResponseValue::Text("2 + 2 is 4.".to_string()),
                input_tokens: 0,
                output_tokens: 0,
                reasoning_tokens: 0,
            },
        ]);

        let result = agent.run("What's 2+2?").await;

        assert!(result.is_ok());

        let result = result.unwrap();
        assert_eq!(
            result
                .items()
                .iter()
                .find(|&ti| ti.role == Role::User
                    && ti
                        .blocks
                        .iter()
                        .any(|f| matches!(f, TranscriptContent::ToolResult(..))))
                .unwrap()
                .blocks
                .iter()
                .find(|&tb| matches!(tb, TranscriptContent::ToolResult(..)))
                .unwrap(),
            &TranscriptContent::ToolResult(ToolResult {
                id: "call-1".to_string(),
                content: "Tool name is empty".to_string(),
                is_error: true,
            }),
        );
    }

    #[tokio::test]
    async fn test_agent_fails_on_missing_tool() {
        let mut agent = setup_agent(vec![
            crate::provider::ProviderResponse {
                value: ProviderResponseValue::ToolCalls(vec![ProviderResponseToolCall {
                    tool_name: "missing-tool".to_string(),
                    tool_args: serde_json::json!({"a": 2, "b": 2}),
                    tool_call_id: "call-1".to_string(),
                }]),
                input_tokens: 0,
                output_tokens: 0,
                reasoning_tokens: 0,
            },
            crate::provider::ProviderResponse {
                value: ProviderResponseValue::Text("2 + 2 is 4.".to_string()),
                input_tokens: 0,
                output_tokens: 0,
                reasoning_tokens: 0,
            },
        ]);

        let result = agent.run("What's 2+2?").await;

        assert!(result.is_ok());

        let result = result.unwrap();
        assert_eq!(
            result
                .items()
                .iter()
                .find(|&ti| ti.role == Role::User
                    && ti
                        .blocks
                        .iter()
                        .any(|f| matches!(f, TranscriptContent::ToolResult(..))))
                .unwrap()
                .blocks
                .iter()
                .find(|&tb| matches!(tb, TranscriptContent::ToolResult(..)))
                .unwrap(),
            &TranscriptContent::ToolResult(ToolResult {
                id: "call-1".to_string(),
                content: "Tool does not exist: \"missing-tool\"".to_string(),
                is_error: true,
            }),
        );
    }

    #[tokio::test]
    async fn test_agent_fails_on_invalid_tool_args() {
        let args = serde_json::json!({"first": 2, "second": 2});

        let mut agent = setup_agent(vec![
            crate::provider::ProviderResponse {
                value: ProviderResponseValue::ToolCalls(vec![ProviderResponseToolCall {
                    tool_name: "calc".to_string(),
                    tool_args: args.clone(),
                    tool_call_id: "call-1".to_string(),
                }]),
                input_tokens: 0,
                output_tokens: 0,
                reasoning_tokens: 0,
            },
            crate::provider::ProviderResponse {
                value: ProviderResponseValue::Text("2 + 2 is 4.".to_string()),
                input_tokens: 0,
                output_tokens: 0,
                reasoning_tokens: 0,
            },
        ]);

        let result = agent.run("What's 2+2?").await;

        assert!(result.is_ok());

        let result = result.unwrap();
        assert_eq!(
            result
                .items()
                .iter()
                .find(|&ti| ti.role == Role::User
                    && ti
                        .blocks
                        .iter()
                        .any(|f| matches!(f, TranscriptContent::ToolResult(..))))
                .unwrap()
                .blocks
                .iter()
                .find(|&tb| matches!(tb, TranscriptContent::ToolResult(..)))
                .unwrap(),
            &TranscriptContent::ToolResult(ToolResult {
                id: "call-1".to_string(),
                content: format!("Invalid arguments for tool calc: {:?}", args).to_string(),
                is_error: true,
            }),
        );
    }
}
