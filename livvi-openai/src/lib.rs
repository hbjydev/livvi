use anyhow::Result;
use async_trait::async_trait;
use livvi_core::{
    model::{ToolCall, ToolResult, Transcript, TranscriptContent, TranscriptItem},
    provider::{Provider, ProviderResponse, ProviderResponseValue},
    tool::{ToolSchema, Tools},
};
use openai_api_rs::v1::{
    api::OpenAIClient,
    responses::responses::{CreateResponseRequest, ResponseObject},
    types::{self, Function, FunctionParameters, JSONSchemaDefine, JSONSchemaType, ToolsFunction},
};
use serde_json::{Value, json};

pub struct OpenAIProvider {
    client: OpenAIClient,
    model_name: String,
}

impl OpenAIProvider {
    pub fn new(api_key: &str, api_url: &str, model_name: &str) -> Result<Self> {
        let client = OpenAIClient::builder()
            .with_endpoint(api_url)
            .with_api_key(api_key)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create OpenAI client: {}", e))?;

        Ok(OpenAIProvider {
            client,
            model_name: model_name.to_string(),
        })
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    async fn complete(&mut self, transcript: Transcript, tools: Tools) -> Result<ProviderResponse> {
        let mut input_items = vec![];
        for item in transcript.items() {
            input_items.extend(into_openai(item)?);
        }

        let mut tool_items = vec![];
        for tool in tools.schemas() {
            tool_items.push(tool_to_responses(tool));
        }

        let mut req = CreateResponseRequest::new();
        req.model = Some(self.model_name.clone());
        req.input = Some(input_items.into());
        req.tools = Some(tool_items);

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
        reasoning_tokens: 0,
    };

    if let Some(usage) = ro.usage {
        if let Some(details) = usage
            .get("output_tokens_details")
            .and_then(|d| d.as_object())
        {
            resp.reasoning_tokens = details
                .get("reasoning")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
        }
        resp.input_tokens = usage
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        resp.output_tokens = usage
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
    }

    let output_array = ro
        .output
        .and_then(|o| o.as_array().cloned())
        .unwrap_or_default();

    let mut tool_calls = vec![];
    let mut texts = vec![];
    let mut reasoning_texts = vec![];

    for item in &output_array {
        match item.get("type").and_then(|t| t.as_str()) {
            Some("reasoning") => {
                if let Some(summary) = item.get("summary").and_then(|s| s.as_array()) {
                    for part in summary {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            reasoning_texts.push(text.to_string());
                        }
                    }
                }
            }
            Some("function_call") => {
                let call_id = item
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .or_else(|| item.get("id").and_then(|v| v.as_str()))
                    .ok_or(anyhow::anyhow!("No call_id or id in function_call item"))?;
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or(anyhow::anyhow!("No name in function_call item"))?;
                let arguments = item
                    .get("arguments")
                    .ok_or(anyhow::anyhow!("No arguments in function_call item"))?;

                tool_calls.push(livvi_core::provider::ProviderResponseToolCall {
                    tool_name: name.to_string(),
                    tool_args: arguments.clone(),
                    tool_call_id: call_id.to_string(),
                });
            }
            Some("message") => {
                for block in item
                    .get("content")
                    .and_then(|b| b.as_array())
                    .unwrap_or(&vec![])
                {
                    if block.get("type").and_then(|t| t.as_str()) == Some("output_text")
                        && let Some(content) = block.get("text").and_then(|c| c.as_str())
                    {
                        texts.push(content.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    if !tool_calls.is_empty() {
        resp.value = ProviderResponseValue::ToolCalls(tool_calls);
    } else if !reasoning_texts.is_empty() {
        resp.value = ProviderResponseValue::Reasoning(reasoning_texts.join("\n"));
    } else if !texts.is_empty() {
        resp.value = ProviderResponseValue::Text(texts.join("\n"));
    }

    Ok(resp)
}

fn into_openai(ti: TranscriptItem) -> Result<Vec<serde_json::Value>> {
    let mut items = vec![];
    let mut text_parts = vec![];

    for block in ti.blocks {
        match block {
            TranscriptContent::ToolResult(ToolResult { id, content, .. }) => {
                items.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": id,
                    "output": content,
                }));
            }
            TranscriptContent::Reasoning { metadata, .. } => {
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
            TranscriptContent::ToolCall(ToolCall { id, name, input }) => {
                items.push(json!({
                    "type": "function_call",
                    "id": id,
                    "name": name,
                    "arguments": input,
                }));
            }
            TranscriptContent::Text(text) => {
                text_parts.push(text);
            }
        }
    }

    if !text_parts.is_empty() {
        items.push(serde_json::json!({
            "role": ti.role.to_string(),
            "content": text_parts.join("\n"),
        }));
    }

    Ok(items)
}

fn tool_to_responses(tool: ToolSchema) -> types::Tools {
    types::Tools::Function(ToolsFunction {
        function: Function {
            name: tool.name,
            description: Some(tool.description),
            parameters: schema_to_function_parameters(tool.input_schema),
        },
    })
}

fn schema_to_function_parameters(schema: schemars::Schema) -> FunctionParameters {
    function_parameters_from_value(schema.as_value())
}

fn function_parameters_from_value(value: &Value) -> FunctionParameters {
    FunctionParameters {
        schema_type: value
            .get("type")
            .and_then(json_schema_type_from_value)
            .unwrap_or(JSONSchemaType::Object),
        properties: value
            .get("properties")
            .and_then(|p| p.as_object())
            .map(|props| {
                props
                    .iter()
                    .map(|(k, v)| (k.clone(), Box::new(json_schema_define_from_value(v))))
                    .collect()
            }),
        required: value.get("required").and_then(|r| r.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        }),
    }
}

fn json_schema_define_from_value(value: &Value) -> JSONSchemaDefine {
    JSONSchemaDefine {
        schema_type: value.get("type").and_then(json_schema_type_from_value),
        description: value
            .get("description")
            .and_then(|v| v.as_str().map(|s| s.to_string())),
        enum_values: value.get("enum").and_then(|e| e.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        }),
        properties: value
            .get("properties")
            .and_then(|p| p.as_object())
            .map(|props| {
                props
                    .iter()
                    .map(|(k, v)| (k.clone(), Box::new(json_schema_define_from_value(v))))
                    .collect()
            }),
        required: value.get("required").and_then(|r| r.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        }),
        items: value
            .get("items")
            .map(|v| Box::new(json_schema_define_from_value(v))),
    }
}

fn json_schema_type_from_value(value: &Value) -> Option<JSONSchemaType> {
    let type_str = match value {
        Value::String(s) => Some(s.as_str()),
        Value::Array(arr) => arr.first().and_then(|v| v.as_str()),
        _ => None,
    }?;

    match type_str {
        "object" => Some(JSONSchemaType::Object),
        "array" => Some(JSONSchemaType::Array),
        "number" => Some(JSONSchemaType::Number),
        "integer" => Some(JSONSchemaType::Number),
        "string" => Some(JSONSchemaType::String),
        "boolean" => Some(JSONSchemaType::Boolean),
        "null" => Some(JSONSchemaType::Null),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response_object(
        output: serde_json::Value,
        usage: Option<serde_json::Value>,
    ) -> ResponseObject {
        ResponseObject {
            id: "resp-1".to_string(),
            object: "response".to_string(),
            created_at: None,
            model: None,
            status: None,
            output: Some(output),
            output_text: None,
            output_audio: None,
            stop_reason: None,
            refusal: None,
            tool_calls: None,
            metadata: None,
            usage,
            system_fingerprint: None,
            service_tier: None,
            status_details: None,
            incomplete_details: None,
            error: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn into_openai_user_message_is_not_empty() {
        let item = TranscriptItem::user_message("Hello, world!");
        let items = into_openai(item).unwrap();
        assert_eq!(
            items,
            vec![json!({"role": "user", "content": "Hello, world!"})]
        );
    }

    #[test]
    fn into_openai_system_message() {
        let item = TranscriptItem::system_message("Be helpful.");
        let items = into_openai(item).unwrap();
        assert_eq!(
            items,
            vec![json!({"role": "system", "content": "Be helpful."})]
        );
    }

    #[test]
    fn into_openai_assistant_text() {
        let item = TranscriptItem::assistant_message("Hi there.");
        let items = into_openai(item).unwrap();
        assert_eq!(
            items,
            vec![json!({"role": "assistant", "content": "Hi there."})]
        );
    }

    #[test]
    fn into_openai_function_call_uses_id() {
        let item = TranscriptItem::assistant_tool_call(ToolCall {
            name: "calc".to_string(),
            id: "call-1".to_string(),
            input: json!({"a": 2, "b": 2}),
        });
        let items = into_openai(item).unwrap();
        assert_eq!(
            items,
            vec![json!({
                "type": "function_call",
                "id": "call-1",
                "name": "calc",
                "arguments": {"a": 2, "b": 2}
            })]
        );
    }

    #[test]
    fn into_openai_function_call_output() {
        let item = TranscriptItem::tool_result(ToolResult {
            id: "call-1".to_string(),
            content: "4".to_string(),
            is_error: false,
        });
        let items = into_openai(item).unwrap();
        assert_eq!(
            items,
            vec![json!({
                "type": "function_call_output",
                "call_id": "call-1",
                "output": "4"
            })]
        );
    }

    #[test]
    fn from_openai_text_response() {
        let ro = response_object(
            json!([{
                "type": "message",
                "content": [{"type": "output_text", "text": "Hello back!"}]
            }]),
            Some(json!({"input_tokens": 10, "output_tokens": 5})),
        );
        let resp = from_openai(ro).unwrap();
        assert_eq!(resp.input_tokens, 10);
        assert_eq!(resp.output_tokens, 5);
        match resp.value {
            ProviderResponseValue::Text(text) => assert_eq!(text, "Hello back!"),
            _ => panic!("expected text response"),
        }
    }

    #[test]
    fn from_openai_tool_call_response() {
        let ro = response_object(
            json!([{
                "type": "function_call",
                "call_id": "call-1",
                "name": "calc",
                "arguments": "{\"a\":2,\"b\":2}"
            }]),
            Some(json!({"input_tokens": 10, "output_tokens": 5})),
        );
        let resp = from_openai(ro).unwrap();
        match resp.value {
            ProviderResponseValue::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].tool_name, "calc");
                assert_eq!(calls[0].tool_call_id, "call-1");
            }
            _ => panic!("expected tool calls"),
        }
    }

    #[test]
    fn from_openai_tool_call_falls_back_to_id() {
        let ro = response_object(
            json!([{
                "type": "function_call",
                "id": "call-1",
                "name": "calc",
                "arguments": "{\"a\":2,\"b\":2}"
            }]),
            None,
        );
        let resp = from_openai(ro).unwrap();
        match resp.value {
            ProviderResponseValue::ToolCalls(calls) => {
                assert_eq!(calls[0].tool_call_id, "call-1");
            }
            _ => panic!("expected tool calls"),
        }
    }

    #[test]
    fn from_openai_missing_usage_defaults_to_zero() {
        let ro = response_object(
            json!([{
                "type": "message",
                "content": [{"type": "output_text", "text": "Hi!"}]
            }]),
            None,
        );
        let resp = from_openai(ro).unwrap();
        assert_eq!(resp.input_tokens, 0);
        assert_eq!(resp.output_tokens, 0);
        assert_eq!(resp.reasoning_tokens, 0);
    }

    #[test]
    fn from_openai_reasoning_response() {
        let ro = response_object(
            json!([{
                "type": "reasoning",
                "summary": [{"type": "summary_text", "text": "Thinking..."}]
            }]),
            Some(json!({
                "input_tokens": 10,
                "output_tokens": 5,
                "output_tokens_details": {"reasoning": 3}
            })),
        );
        let resp = from_openai(ro).unwrap();
        assert_eq!(resp.reasoning_tokens, 3);
        match resp.value {
            ProviderResponseValue::Reasoning(text) => assert_eq!(text, "Thinking..."),
            _ => panic!("expected reasoning response"),
        }
    }

    #[test]
    fn tool_to_responses_maps_schemars_schema() {
        #[allow(dead_code)]
        #[derive(schemars::JsonSchema)]
        struct CalcInput {
            a: i32,
            b: i32,
        }

        let schema = ToolSchema {
            name: "calc".to_string(),
            description: "Adds two numbers".to_string(),
            input_schema: schemars::schema_for!(CalcInput),
        };

        let tool = tool_to_responses(schema);

        match tool {
            types::Tools::Function(func) => {
                assert_eq!(func.function.name, "calc");
                assert_eq!(
                    func.function.description.as_deref(),
                    Some("Adds two numbers")
                );
                assert_eq!(func.function.parameters.schema_type, JSONSchemaType::Object);

                let props = func
                    .function
                    .parameters
                    .properties
                    .expect("expected properties");
                assert!(props.contains_key("a"));
                assert!(props.contains_key("b"));

                assert_eq!(props["a"].schema_type, Some(JSONSchemaType::Number));
                assert_eq!(props["b"].schema_type, Some(JSONSchemaType::Number));

                let required = func
                    .function
                    .parameters
                    .required
                    .expect("expected required");
                assert!(required.contains(&"a".to_string()));
                assert!(required.contains(&"b".to_string()));
            }
            _ => panic!("expected function tool"),
        }
    }
}
