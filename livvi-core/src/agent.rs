use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use futures::stream::StreamExt;

use crate::{
    model::{Role, ToolCall, ToolResult, Transcript, TranscriptContent, TranscriptItem},
    provider::{FinishReason, Provider, ProviderEvent},
    tool::{ToolCallOutput, ToolContext, Tools},
};

pub const MAX_ITERATIONS: usize = 50;

pub const AGENT_INSTRUCTIONS: &str = include_str!("../prompts/instructions.md");

#[derive(Debug, Clone)]
pub enum AgentEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallDelta {
        id: String,
        arguments: String,
    },
    ToolCallDone {
        id: String,
    },
    ToolCallExecuting {
        name: String,
    },
    ToolCallResult {
        id: String,
        content: String,
        is_error: bool,
    },
    Usage {
        input_tokens: usize,
        output_tokens: usize,
        reasoning_tokens: usize,
    },
    Done {
        reason: FinishReason,
    },
    TurnComplete,
    Error(String),
}

#[async_trait]
pub trait AgentEventSink: Send + Sync {
    async fn send(&self, event: AgentEvent);
}

pub struct NoOpEventSink;

#[async_trait]
impl AgentEventSink for NoOpEventSink {
    async fn send(&self, _event: AgentEvent) {}
}

#[derive(Debug, Default)]
struct TokenUsage {
    input_tokens: usize,
    output_tokens: usize,
    reasoning_tokens: usize,
}

#[derive(Debug)]
struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
}

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
    event_sink: Box<dyn AgentEventSink>,
}

impl<P, S> Agent<P, S>
where
    P: Provider<S>,
    S: Send + Sync + 'static,
{
    /// Creates a new Agent with the given provider, tools, and application state.
    pub fn new(provider: P, tools: Tools<S>, state: S) -> Self {
        Self::with_event_sink(provider, tools, state, Box::new(NoOpEventSink))
    }

    /// Creates a new Agent that streams events to the given sink.
    pub fn with_event_sink(
        provider: P,
        tools: Tools<S>,
        state: S,
        event_sink: Box<dyn AgentEventSink>,
    ) -> Self {
        Agent {
            provider,
            tools,
            state,
            event_sink,
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
            let mut stream = self
                .provider
                .stream(transcript.clone(), self.tools.clone())
                .await?;

            let mut text = String::new();
            let mut reasoning = String::new();
            let mut tool_calls: HashMap<String, PendingToolCall> = HashMap::new();
            let mut finish_reason: Option<FinishReason> = None;
            let mut usage = TokenUsage::default();

            while let Some(result) = stream.next().await {
                tracing::debug!("Provider event: {:?}", result);

                match result {
                    Ok(event) => {
                        self.emit(provider_event_to_agent_event(event.clone()))
                            .await;

                        match event {
                            ProviderEvent::TextDelta(delta) => text.push_str(&delta),
                            ProviderEvent::ReasoningDelta(delta) => reasoning.push_str(&delta),
                            ProviderEvent::ToolCallStart { id, name } => {
                                tool_calls.insert(
                                    id.clone(),
                                    PendingToolCall {
                                        id,
                                        name,
                                        arguments: String::new(),
                                    },
                                );
                            }
                            ProviderEvent::ToolCallDelta { id, arguments } => {
                                if let Some(call) = tool_calls.get_mut(&id) {
                                    call.arguments.push_str(&arguments);
                                }
                            }
                            ProviderEvent::ToolCallDone { id } => {
                                if !tool_calls.contains_key(&id) {
                                    anyhow::bail!("ToolCallDone for unknown id: {}", id);
                                }
                            }
                            ProviderEvent::Usage {
                                input_tokens,
                                output_tokens,
                                reasoning_tokens,
                            } => {
                                usage.input_tokens = input_tokens;
                                usage.output_tokens = output_tokens;
                                usage.reasoning_tokens = reasoning_tokens;
                            }
                            ProviderEvent::Done { reason } => finish_reason = Some(reason),
                        }
                    }
                    Err(e) => {
                        self.emit(AgentEvent::Error(e.to_string())).await;
                        return Err(e);
                    }
                }
            }

            let reason = finish_reason
                .ok_or_else(|| anyhow::anyhow!("Provider stream ended without finish reason"))?;

            if !tool_calls.is_empty() {
                let completed_calls: Vec<ToolCall> = tool_calls
                    .into_values()
                    .map(|call| {
                        let input = serde_json::from_str(&call.arguments)
                            .unwrap_or(serde_json::Value::Null);
                        ToolCall {
                            name: call.name,
                            id: call.id,
                            input,
                        }
                    })
                    .collect();

                let mut item = TranscriptItem {
                    role: Role::Assistant,
                    created_at: std::time::Instant::now(),
                    blocks: vec![],
                };
                if !reasoning.is_empty() {
                    item.blocks.push(TranscriptContent::Reasoning {
                        text: reasoning,
                        metadata: serde_json::json!({
                            "input_tokens": usage.input_tokens,
                            "output_tokens": usage.output_tokens,
                            "reasoning_tokens": usage.reasoning_tokens,
                        }),
                    });
                }
                for call in &completed_calls {
                    item.blocks.push(TranscriptContent::ToolCall(call.clone()));
                }
                transcript.add_item(item);

                for call in completed_calls {
                    self.emit(AgentEvent::ToolCallExecuting {
                        name: call.name.clone(),
                    })
                    .await;

                    let result = self.execute_tool(&call, &transcript).await;

                    self.emit(AgentEvent::ToolCallResult {
                        id: result.id.clone(),
                        content: result.content.clone(),
                        is_error: result.is_error,
                    })
                    .await;

                    transcript.add_item(TranscriptItem::tool_result(result));
                }

                continue;
            }

            match reason {
                FinishReason::EndTurn | FinishReason::Other(_) | FinishReason::ContentFilter => {
                    let mut item = TranscriptItem {
                        role: Role::Assistant,
                        created_at: std::time::Instant::now(),
                        blocks: vec![],
                    };
                    if !reasoning.is_empty() {
                        item.blocks.push(TranscriptContent::Reasoning {
                            text: reasoning,
                            metadata: serde_json::json!({
                                "input_tokens": usage.input_tokens,
                                "output_tokens": usage.output_tokens,
                                "reasoning_tokens": usage.reasoning_tokens,
                            }),
                        });
                    }
                    if !text.is_empty() || item.blocks.is_empty() {
                        item.blocks.push(TranscriptContent::Text(text));
                    }
                    transcript.add_item(item);
                    self.emit(AgentEvent::TurnComplete).await;
                    return Ok(transcript);
                }
                FinishReason::MaxTokens | FinishReason::Incomplete => {
                    anyhow::bail!("Provider response was truncated: {:?}", reason);
                }
                FinishReason::ToolCalls => {
                    anyhow::bail!("Provider stopped for tool calls but emitted no tool calls");
                }
            }
        }

        Err(anyhow::anyhow!(
            "Max iterations reached without a final response"
        ))
    }

    async fn execute_tool(&self, tool_call: &ToolCall, transcript: &Transcript) -> ToolResult {
        if tool_call.name.is_empty() {
            return ToolResult {
                id: tool_call.id.clone(),
                content: "Tool name is empty".into(),
                is_error: true,
            };
        }

        let tool = match self.tools.get_tool(&tool_call.name) {
            Some(tool) => tool,
            None => {
                return ToolResult {
                    id: tool_call.id.clone(),
                    content: format!("Tool does not exist: {:?}", tool_call.name),
                    is_error: true,
                };
            }
        };

        let ctx = ToolContext {
            transcript,
            tool_call_id: &tool_call.id,
            state: &self.state,
        };

        let output: ToolCallOutput = tool.call(&ctx, tool_call.input.clone()).await;
        output.into_tool_result(tool_call.id.clone())
    }

    async fn emit(&self, event: AgentEvent) {
        self.event_sink.send(event).await;
    }
}

fn provider_event_to_agent_event(event: ProviderEvent) -> AgentEvent {
    match event {
        ProviderEvent::TextDelta(delta) => AgentEvent::TextDelta(delta),
        ProviderEvent::ReasoningDelta(delta) => AgentEvent::ReasoningDelta(delta),
        ProviderEvent::ToolCallStart { id, name } => AgentEvent::ToolCallStart { id, name },
        ProviderEvent::ToolCallDelta { id, arguments } => {
            AgentEvent::ToolCallDelta { id, arguments }
        }
        ProviderEvent::ToolCallDone { id } => AgentEvent::ToolCallDone { id },
        ProviderEvent::Usage {
            input_tokens,
            output_tokens,
            reasoning_tokens,
        } => AgentEvent::Usage {
            input_tokens,
            output_tokens,
            reasoning_tokens,
        },
        ProviderEvent::Done { reason } => AgentEvent::Done { reason },
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
        provider::{FinishReason, MockProvider, ProviderEvent},
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

    fn setup_agent(turns: Vec<Vec<ProviderEvent>>) -> Agent<MockProvider, Arc<AppState>> {
        let mut tools = Tools::new();
        tools.add_tool(calc);
        let provider = MockProvider::new(turns);
        Agent::new(provider, tools, Arc::new(AppState))
    }

    #[tokio::test]
    async fn test_agent_runs() {
        let mut agent = setup_agent(vec![
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
            vec![
                ProviderEvent::ToolCallStart {
                    id: "call-1".to_string(),
                    name: "".to_string(),
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
            vec![
                ProviderEvent::ToolCallStart {
                    id: "call-1".to_string(),
                    name: "missing-tool".to_string(),
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
            vec![
                ProviderEvent::ToolCallStart {
                    id: "call-1".to_string(),
                    name: "calc".to_string(),
                },
                ProviderEvent::ToolCallDelta {
                    id: "call-1".to_string(),
                    arguments: args.to_string(),
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
