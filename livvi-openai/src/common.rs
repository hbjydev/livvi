use std::borrow::Cow;

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
    let value = value.as_ref();
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
    let value = value.as_ref();
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

fn normalize_schema<'a>(value: &'a Value, root: &'a Value) -> Cow<'a, Value> {
    let value = resolve_ref(value, root);
    flatten_nullable(value, root)
}

fn resolve_ref<'a>(value: &'a Value, root: &'a Value) -> &'a Value {
    if let Some(Value::String(ref_path)) = value.get("$ref") {
        if ref_path.is_empty() {
            return value;
        }
        let pointer_path = ref_path.trim_start_matches('#');
        return root.pointer(pointer_path).unwrap_or(value);
    }
    value
}

fn flatten_nullable<'a>(value: &'a Value, root: &'a Value) -> Cow<'a, Value> {
    let Some(obj) = value.as_object() else {
        return Cow::Borrowed(value);
    };
    let Some(any_of) = obj.get("anyOf").or_else(|| obj.get("oneOf")) else {
        return Cow::Borrowed(value);
    };
    let Some(arr) = any_of.as_array() else {
        return Cow::Borrowed(value);
    };

    let mut null_count = 0;
    let mut non_null = Vec::new();
    for v in arr {
        let v = resolve_ref(v, root);
        if is_null_schema(v) {
            null_count += 1;
        } else {
            non_null.push(v);
        }
    }

    if null_count == 0 || non_null.len() != 1 {
        return Cow::Borrowed(value);
    }

    let inner = non_null[0];

    let metadata_keys = ["description", "title", "default"];
    let has_metadata = metadata_keys.iter().any(|k| obj.contains_key(*k));
    if !has_metadata {
        return Cow::Borrowed(inner);
    }

    let Some(inner_obj) = inner.as_object() else {
        return Cow::Borrowed(inner);
    };

    let mut merged = obj.clone();
    merged.remove("anyOf");
    merged.remove("oneOf");
    for (k, v) in inner_obj {
        merged.insert(k.clone(), v.clone());
    }
    Cow::Owned(Value::Object(merged))
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

    #[test]
    fn flatten_nullable_preserves_wrapper_metadata() {
        let schema = serde_json::json!({
            "description": "A color",
            "title": "Color",
            "default": "Red",
            "anyOf": [
                { "type": "string", "enum": ["Red", "Green", "Blue"] },
                { "type": "null" }
            ]
        });

        let flattened = flatten_nullable(&schema, &schema);
        assert_eq!(
            flattened.get("description").and_then(|v| v.as_str()),
            Some("A color")
        );
        assert_eq!(
            flattened.get("type").and_then(|v| v.as_str()),
            Some("string")
        );
        assert_eq!(
            flattened
                .get("enum")
                .and_then(|v| v.as_array())
                .map(|a| a.len()),
            Some(3)
        );
    }

    #[test]
    fn flatten_nullable_keeps_multi_variant_one_of() {
        let schema = serde_json::json!({
            "oneOf": [
                { "type": "string", "enum": ["Red"] },
                { "type": "string", "enum": ["Green"] },
                { "type": "string", "enum": ["Blue"] }
            ]
        });

        let flattened = flatten_nullable(&schema, &schema);
        assert!(flattened.get("oneOf").is_some());
    }
}
