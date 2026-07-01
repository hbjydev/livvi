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
                    ..
                } => {
                    if tool_name.is_empty() {
                        return Err(anyhow::anyhow!("Tool name is empty"));
                    }

                    let tool = self
                        .tools
                        .get_tool(&tool_name)
                        .ok_or_else(|| anyhow::anyhow!("Tool not found: {}", tool_name))?;

                    let result = tool.call().await?;

                    transcript.add_item(TranscriptItem {
                        role: Role::Assistant,
                        content: TranscriptContent::ToolUse {
                            name: tool_name.clone(),
                            id: "some_id".to_string(),
                            input: tool_args.clone(),
                        },
                    });

                    transcript.add_item(TranscriptItem {
                        role: Role::Assistant,
                        content: TranscriptContent::ToolResult {
                            id: "some_id".to_string(),
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
