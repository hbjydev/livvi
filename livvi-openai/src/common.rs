use serde_json::Value;

use livvi_core::tool::ToolDefinition;
use openai_api_rs::v1::types::{Function, FunctionParameters, JSONSchemaDefine, JSONSchemaType};

pub fn tool_to_function(tool: ToolDefinition) -> Function {
    Function {
        name: tool.name,
        description: Some(tool.description),
        parameters: schema_to_function_parameters(tool.input_schema),
    }
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

    #[test]
    fn tool_to_function_maps_schemars_schema() {
        #[allow(dead_code)]
        #[derive(schemars::JsonSchema)]
        struct CalcInput {
            a: i32,
            b: i32,
        }

        let schema = ToolDefinition {
            name: "calc".to_string(),
            description: "Adds two numbers".to_string(),
            input_schema: schemars::schema_for!(CalcInput),
        };

        let func = tool_to_function(schema);

        assert_eq!(func.name, "calc");
        assert_eq!(func.description.as_deref(), Some("Adds two numbers"));
        assert_eq!(func.parameters.schema_type, JSONSchemaType::Object);

        let props = func.parameters.properties.expect("expected properties");
        assert!(props.contains_key("a"));
        assert!(props.contains_key("b"));

        assert_eq!(props["a"].schema_type, Some(JSONSchemaType::Number));
        assert_eq!(props["b"].schema_type, Some(JSONSchemaType::Number));

        let required = func.parameters.required.expect("expected required");
        assert!(required.contains(&"a".to_string()));
        assert!(required.contains(&"b".to_string()));
    }
}
