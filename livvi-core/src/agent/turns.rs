use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::{AgentEvent, agent::Agent, context::Context, interrupt::Interrupt, model::ToolCall, provider::ProviderEvent, tool::ToolContext};

const TOK_STREAM_BUFFER_SIZE: usize = 256;

struct StreamIteration {
    response: String,
    thinking: String,
    tool_calls: Vec<ToolCall>,
    stream_error: Option<String>,
    cancelled_by: Option<Interrupt>,
}

impl<S: Sync + Send + 'static> Agent<S> {
    #[tracing::instrument(skip(self, interrupt, context))]
    pub(super) async fn run_turn(
        &mut self,
        interrupt: Interrupt,
        context: &mut Context,
    ) -> Result<Option<Interrupt>> {
        let mut tool_iterations = 0usize;
        const MAX_TOOL_ITERATIONS: usize = 20;
        let mut stashed_interrupt = None;

        info!("Running turn with interrupt: {:?}", interrupt);
        let _ = self.output.send(AgentEvent::Started);

        loop {
            let StreamIteration {
                response: iteration_response,
                thinking: iteration_thinking,
                tool_calls,
                stream_error,
                cancelled_by: stream_cancelled_by,
            } = self
                .stream_iteration(context, &mut stashed_interrupt)
                .await;

            if let Some(interrupt) = stream_cancelled_by {
                let _ = self.output.send(AgentEvent::Done);
                return Ok(Some(interrupt));
            }

            if !tool_calls.is_empty() && tool_iterations < MAX_TOOL_ITERATIONS {
                tool_iterations += 1;
                for tool_call in tool_calls.clone() {
                    debug!("Executing tool call: {:?}", tool_call);
                    let _ = self.output.send(AgentEvent::ToolCall(vec![tool_call.clone()]));
                    if let Some(tool) = self.toolbox.get_tool(&tool_call.name) {
                        let _ = self.output.send(AgentEvent::ToolCallStarted);

                        let ctx = ToolContext::<S> {
                            agent_context: context,
                            tool_call_id: &tool_call.id,
                            state: &self.state,
                        };

                        let result = tool.call(&ctx, tool_call.input).await;
                        let tool_result = result.into_tool_result(&tool_call.id);

                        if tool_result.is_error {
                            let msg = format!("Tool call failed: {}", tool_result.content);
                            tracing::error!(%msg);
                            let _ = self.output.send(AgentEvent::Error(msg.clone()));
                            let _ = self
                                .output
                                .send(AgentEvent::Status("Tool call failed".into()));

                            context.push_tool_result(&tool_result.id, &tool_result.content);
                        }
                    } else {
                        let msg = format!("Tool not found: {}", tool_call.name);
                        tracing::error!(%msg);
                        let _ = self.output.send(AgentEvent::Error(msg.clone()));
                        let _ = self
                            .output
                            .send(AgentEvent::Status("Tool not found".into()));

                        context.push_tool_result(&tool_call.id, "No such tool found");
                    }
                }
            }

            if let Some(err_msg) = &stream_error {
                if iteration_response.is_empty() && tool_calls.is_empty() {
                    let error_content = format!("error: {err_msg}");
                    context.push_assistant(error_content, None);
                    let _ = self.output.send(AgentEvent::Done);
                    break;
                }
            }

            let has_final_text = !iteration_response.is_empty();
            if has_final_text || !iteration_thinking.is_empty() {
                context.push_assistant(
                    &iteration_response,
                    (!iteration_thinking.is_empty()).then_some(iteration_thinking),
                );
            }

            break;
        }

        let _ = self.output.send(AgentEvent::Done);

        info!("Turn completed. Stashed interrupt: {:?}", stashed_interrupt);

        Ok(stashed_interrupt)
    }

    async fn stream_iteration(
        &mut self,
        ctx: &mut Context,
        stashed_interrupt: &mut Option<Interrupt>,
    ) -> StreamIteration {
        let (tok_tx, mut tok_rx) = mpsc::channel(TOK_STREAM_BUFFER_SIZE);
        let mut provider = self.provider.clone_dyn();
        let msgs = ctx.as_messages();
        let tool_schemas = self.toolbox.schemas();
        let stream_task = tokio::spawn(async move { provider.stream(tok_tx, msgs, tool_schemas).await });

        let mut response = String::new();
        let mut thinking = String::new();
        let mut tool_calls = vec![];
        let mut cancelled_by = None;

        loop {
            tokio::select! {
                biased;

                interrupt = self.input.recv() => {
                    match interrupt {
                        Some(int) => {
                            // stash for after turn
                            if stashed_interrupt.is_none() {
                                *stashed_interrupt = Some(int);
                            }
                        },
                        None => {}, // channel closed = shutdown
                    }
                }

                ev = tok_rx.recv() => {
                    match ev {
                        None => break,

                        Some(ProviderEvent::ThinkingToken(token)) => {
                            thinking.push_str(&token);
                            let _ = self.output.send(AgentEvent::ThinkingToken(token));
                        }

                        Some(ProviderEvent::Token(token)) => {
                            response.push_str(&token);
                            let _ = self.output.send(AgentEvent::ScratchToken(token));
                        }

                        Some(ProviderEvent::Usage(usage)) => {
                            ctx.update_usage(usage);
                        }

                        Some(ProviderEvent::ToolCalls(calls)) => {
                            tool_calls.extend(calls.clone());
                        }

                        Some(ProviderEvent::ToolCallStarted) => {
                            let _ = self.output.send(AgentEvent::ToolCallStarted);
                        }
                    }
                }
            }
        }

        let stream_error = match stream_task.await {
            Ok(Ok(())) => None,
            Ok(Err(e)) => {
                let msg = format!("Provider stream failed: {e}");
                tracing::error!(%msg);
                let _ = self.output.send(AgentEvent::Error(msg.clone()));
                let _ = self
                    .output
                    .send(AgentEvent::Status("Provider stream failed".into()));
                Some(msg)
            }
            Err(e) if e.is_cancelled() => {
                // intentional abort — already handled by cancelled_by
                None
            }
            Err(e) => {
                let msg = format!("Provider stream task panicked/cancelled: {e}");
                tracing::error!(%msg);
                let _ = self.output.send(AgentEvent::Error(msg.clone()));
                let _ = self
                    .output
                    .send(AgentEvent::Status("Provider stream task failed".into()));
                Some(msg)
            }
        };

        StreamIteration {
            response,
            thinking,
            tool_calls,
            stream_error,
            cancelled_by,
        }
    }
}
