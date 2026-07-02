use anyhow::Result;
use async_trait::async_trait;
use livvi_core::{model::{ToolCall, ToolResult, Transcript, TranscriptContent, TranscriptItem}, provider::{Provider, ProviderResponse, ProviderResponseValue}};
use openai_api_rs::v1::{api::OpenAIClient, responses::responses::{CreateResponseRequest, ResponseObject}};
use serde_json::json;

pub struct OpenAIProvider {
    client: OpenAIClient,
    model_name: String,
}

impl OpenAIProvider {
    pub fn new(
        api_key: &str,
        api_url: &str,
        model_name: &str,
    ) -> Result<Self> {
        let client = OpenAIClient::builder()
            .with_endpoint(api_url)
            .with_api_key(api_key)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create OpenAI client: {}", e))?;

        Ok(OpenAIProvider { client, model_name: model_name.to_string() })
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    async fn complete(&mut self, transcript: Transcript) -> Result<ProviderResponse> {
        let mut input_items = vec![];
        for item in transcript.items() {
            input_items.extend(into_openai(item)?);
        }

        let mut req = CreateResponseRequest::new();
        req.model = Some(self.model_name.clone());
        req.input = Some(input_items.into());

        let res = self
            .client
            .create_response(req)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create response: {}", e))?;

        Ok(from_openai(res.inner)?)
    }
}

fn from_openai(ro: ResponseObject) -> Result<ProviderResponse> {
    let mut resp = ProviderResponse {
        value: ProviderResponseValue::Text("".into()),
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0
    };

    let mut reasoning_parts = vec![];
    let mut openai_items = vec![];

    for item in ro
        .output
        .clone()
        .ok_or(anyhow::anyhow!("No output in response"))?
        .as_array()
        .ok_or(anyhow::anyhow!("Output was not an array"))?
    {
        match item.get("type").and_then(|t| t.as_str()) {
            Some("reasoning") => {
                if let Some(summary) = item.get("summary") {
                    reasoning_parts.push(summary.clone());
                }
                openai_items.push(json!({
                    "id": item.get("id").unwrap_or(&json!("")),
                    "encrypted_content": item.get("encrypted_content").unwrap_or(&json!("")),
                    "summary": json!([]),
                }));
            }
            _ => {}
        }
        
    }

    // let reasoning_text = reasoning_parts
    //     .into_iter()
    //     .map(|part| part.as_str().unwrap_or("").to_string())
    //     .collect::<Vec<String>>()
    //     .join("\n");
    // let reasoning_metadata = json!({"openai_items": openai_items});

    let usage = ro.usage.ok_or(anyhow::anyhow!("No usage in response"))?;
    let details = usage
        .get("output_tokens_details")
        .ok_or(anyhow::anyhow!("No output_tokens_details in usage"))?
        .as_object()
        .ok_or(anyhow::anyhow!("output_tokens_details is not an object"))?;

    resp.reasoning_tokens = details
        .get("reasoning")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    for item in ro
        .output
        .clone()
        .ok_or(anyhow::anyhow!("No output in response"))?
        .as_array()
        .ok_or(anyhow::anyhow!("Output was not an array"))?
    {
        match item.get("type").and_then(|t| t.as_str()) {
            Some("function_call") => {
                let call_id = item
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .ok_or(anyhow::anyhow!("No call_id in function_call item"))?;
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or(anyhow::anyhow!("No name in function_call item"))?;
                let arguments = item
                    .get("arguments")
                    .ok_or(anyhow::anyhow!("No arguments in function_call item"))?;

                resp.value = ProviderResponseValue::ToolCalls(vec![livvi_core::provider::ProviderResponseToolCall {
                    tool_name: name.to_string(),
                    tool_args: arguments.clone(),
                    tool_call_id: call_id.to_string(),
                }]);
            }
            _ => {}
        }
    }

    let mut texts = vec![];
    for item in ro
        .output
        .ok_or(anyhow::anyhow!("No output in response"))?
        .as_array()
        .ok_or(anyhow::anyhow!("Output was not an array"))?
    {
        match item.get("type").and_then(|t| t.as_str()) {
            Some("message") => {
                for block in item.get("content").and_then(|b| b.as_array()).unwrap_or(&vec![]) {
                    if block.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                        if let Some(content) = block.get("text").and_then(|c| c.as_str()) {
                            texts.push(content.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(resp)
}

fn into_openai(ti: TranscriptItem) -> Result<Vec<serde_json::Value>> {
    // tool results become function_call_output items (no role)
    if ti.blocks.iter().any(|b| matches!(b, TranscriptContent::ToolResult { .. })) {
        let mut blocks = vec![];
        for block in ti.blocks.iter() {
            if let TranscriptContent::ToolResult(ToolResult { id, content, .. }) = block {
                blocks.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": id,
                    "output": content,
                }));
            }
        }

        return Ok(blocks);
    }

    let mut items = vec![];

    for block in &ti.blocks {
        if let TranscriptContent::Reasoning { metadata, .. } = block {
            for spec in metadata
                .as_object()
                .unwrap_or(&serde_json::Map::new())
                .get("openai_items")
                .unwrap_or(&json!([]))
                .as_array()
                .unwrap_or(&vec![])
            {
                let mut item = json!({
                    "type": "reasoning",
                    "summary": spec.get("summary").unwrap_or(&json!([]))
                });

                if let Some(rid) = spec.get("id") {
                    item["id"] = rid.clone();
                }

                if let Some(enc) = spec.get("encrypted_content") {
                    item["encrypted_content"] = enc.clone();
                }

                items.push(item);
            }
        }

        if let TranscriptContent::ToolCall(ToolCall { id, name, input }) = block {
            items.push(json!({
                "type": "function_call",
                "call_id": id,
                "name": name,
                "arguments": input,
            }))
        }

        return Ok(items)
    }

    let text = ti
        .blocks
        .iter()
        .filter_map(|b| {
            if let TranscriptContent::Text(t) = b {
                Some(t.clone())
            } else {
                None
            }
        })
        .collect::<Vec<String>>()
        .join("\n");

    Ok(vec![serde_json::json!({
        "role": ti.role.to_string(),
        "content": text,
    })])
}
