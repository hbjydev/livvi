use anyhow::Result;

use crate::{
    model::{ToolCall, ToolResult, Transcript, TranscriptItem},
    provider::{Provider, ProviderResponseToolCall, ProviderResponseValue},
    tool::{ToolCallOutput, ToolContext, Tools},
};

pub const MAX_ITERATIONS: usize = 10;

pub const AGENT_INSTRUCTIONS: &str = include_str!("../prompts/instructions.md");

/// An Agent is responsible for managing the interaction between a user, a
/// provider, and a set of tools. It maintains a transcript of the conversation
/// and handles tool calls as requested by the provider.
pub struct Agent<P, S>
where
    P: Provider<S>,
    S: Send + Sync + 'static,
{
    provider: P,
    tools: Tools<S>,
    state: S,
}

impl<P, S> Agent<P, S>
where
    P: Provider<S>,
    S: Send + Sync + 'static,
{
    /// Creates a new Agent with the given provider, tools, and application state.
    pub fn new(provider: P, tools: Tools<S>, state: S) -> Self {
        Agent {
            provider,
            tools,
            state,
        }
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
                                id: tool_call_id,
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

                        transcript.add_item(TranscriptItem::assistant_tool_call(ToolCall {
                            name: tool_name.clone(),
                            id: tool_call_id.clone(),
                            input: tool_args.clone(),
                        }));

                        let ctx = ToolContext {
                            transcript: &transcript,
                            tool_call_id: &tool_call_id,
                            state: &self.state,
                        };

                        let output: ToolCallOutput = tool.call(&ctx, tool_args).await;

                        transcript.add_item(TranscriptItem::tool_result(
                            output.into_tool_result(tool_call_id),
                        ));
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
    use std::sync::Arc;

    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    use crate::{
        agent::Agent,
        model::{Role, ToolResult, TranscriptContent},
        provider::{
            MockProvider, ProviderResponse, ProviderResponseToolCall, ProviderResponseValue,
        },
        tool::{Input, Tools, tool},
    };

    #[derive(Debug, Clone, Default)]
    struct AppState;

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

    fn setup_agent(responses: Vec<ProviderResponse>) -> Agent<MockProvider, Arc<AppState>> {
        let mut tools = Tools::new();
        tools.add_tool(calc);
        let provider = MockProvider::new(responses);
        Agent::new(provider, tools, Arc::new(AppState))
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
        let tool_result = result
            .items()
            .iter()
            .find(|&ti| {
                ti.role == Role::User
                    && ti
                        .blocks
                        .iter()
                        .any(|f| matches!(f, TranscriptContent::ToolResult(..)))
            })
            .unwrap()
            .blocks
            .iter()
            .find(|&tb| matches!(tb, TranscriptContent::ToolResult(..)))
            .unwrap()
            .clone();

        assert!(matches!(
            tool_result,
            TranscriptContent::ToolResult(ToolResult { is_error: true, .. })
        ));
    }
}
