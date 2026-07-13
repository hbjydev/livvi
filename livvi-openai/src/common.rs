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
    let value = schema.as_value();
    function_parameters_from_value(value, value)
}

fn function_parameters_from_value(value: &Value, root: &Value) -> FunctionParameters {
    let value = normalize_schema(value, root);
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
                    .map(|(k, v)| (k.clone(), Box::new(json_schema_define_from_value(v, root))))
                    .collect()
            }),
        required: value.get("required").and_then(|r| r.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        }),
    }
}

fn json_schema_define_from_value(value: &Value, root: &Value) -> JSONSchemaDefine {
    let value = normalize_schema(value, root);
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
                    .map(|(k, v)| (k.clone(), Box::new(json_schema_define_from_value(v, root))))
                    .collect()
            }),
        required: value.get("required").and_then(|r| r.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        }),
        items: value
            .get("items")
            .map(|v| Box::new(json_schema_define_from_value(v, root))),
    }
}

fn normalize_schema<'a>(value: &'a Value, root: &'a Value) -> &'a Value {
    let value = resolve_ref(value, root);
    flatten_nullable(value, root)
}

fn resolve_ref<'a>(value: &'a Value, root: &'a Value) -> &'a Value {
    if let Some(Value::String(ref_path)) = value.get("$ref") {
        let mut current = root;
        for part in ref_path.split('/').filter(|p| !p.is_empty() && *p != "#") {
            if let Some(obj) = current.as_object() {
                if let Some(next) = obj.get(part) {
                    current = next;
                } else {
                    return value;
                }
            } else {
                return value;
            }
        }
        return current;
    }
    value
}

fn flatten_nullable<'a>(value: &'a Value, root: &'a Value) -> &'a Value {
    if let Some(obj) = value.as_object()
        && let Some(any_of) = obj.get("anyOf").or_else(|| obj.get("oneOf"))
        && let Some(arr) = any_of.as_array()
    {
        for v in arr {
            let v = resolve_ref(v, root);
            if !is_null_schema(v) {
                return v;
            }
        }
    }
    value
}

fn is_null_schema(value: &Value) -> bool {
    if let Some(Value::String(s)) = value.get("type") {
        return s == "null";
    }
    if let Some(arr) = value.get("type").and_then(|v| v.as_array()) {
        return arr.iter().all(|v| v.as_str() == Some("null"));
    }
    false
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

    #[derive(schemars::JsonSchema)]
    #[allow(dead_code)]
    enum Color {
        Red,
        Green,
        Blue,
    }

    #[derive(schemars::JsonSchema)]
    #[allow(dead_code)]
    struct TagInput {
        query: String,
        colors: Option<Vec<Color>>,
        primary: Option<Color>,
    }

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

    #[test]
    fn tool_to_function_resolves_enum_refs() {
        let schema = ToolDefinition {
            name: "tag".to_string(),
            description: "Tag stuff".to_string(),
            input_schema: schemars::schema_for!(TagInput),
        };

        let func = tool_to_function(schema);
        let props = func.parameters.properties.expect("expected properties");

        let colors = props.get("colors").expect("expected colors property");
        assert_eq!(colors.schema_type, Some(JSONSchemaType::Array));
        let items = colors.items.as_ref().expect("expected items");
        assert_eq!(items.schema_type, Some(JSONSchemaType::String));
        assert_eq!(
            items.enum_values,
            Some(vec![
                "Red".to_string(),
                "Green".to_string(),
                "Blue".to_string()
            ])
        );

        let primary = props.get("primary").expect("expected primary property");
        assert_eq!(primary.schema_type, Some(JSONSchemaType::String));
        assert_eq!(
            primary.enum_values,
            Some(vec![
                "Red".to_string(),
                "Green".to_string(),
                "Blue".to_string()
            ])
        );
    }
}
