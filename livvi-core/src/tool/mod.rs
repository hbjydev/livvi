use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

pub use livvi_core_macros::ToolSchema;

/// A concrete description of a tool that can be passed to a provider.
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: schemars::Schema,
}

/// Static schema metadata for a tool. This is normally implemented via the
/// `ToolSchema` derive macro.
///
/// The methods take `&self` so that the trait remains dyn-compatible; `Tool`
/// requires `ToolSchema` and is used as `dyn Tool` by the `Tools` registry.
pub trait ToolSchema {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn input_schema(&self) -> schemars::Schema;
}

#[async_trait]
pub trait Tool: Send + Sync + ToolSchema {
    fn schema(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: self.input_schema(),
        }
    }

    fn validate_input(&self, args: &Value) -> Result<()> {
        let validator = jsonschema::validator_for(self.schema().input_schema.as_value())?;

        if !validator.is_valid(args) {
            anyhow::bail!(
                "Invalid arguments for tool {}: {:?}",
                self.schema().name,
                args
            )
        } else {
            Ok(())
        }
    }

    async fn call(&self, args: Value) -> Result<String>;
}

#[derive(Clone, Default)]
pub struct Tools(HashMap<String, Arc<dyn Tool>>);

impl Tools {
    pub fn new() -> Self {
        Tools(HashMap::new())
    }

    pub fn schemas(&self) -> Vec<ToolDefinition> {
        self.0.values().map(|tool| tool.schema()).collect()
    }

    pub fn add_tool(&mut self, tool: impl Tool + 'static) {
        self.0.insert(tool.schema().name, Arc::new(tool));
    }

    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.0.iter().any(|tool| tool.0 == tool_name)
    }

    pub fn get_tool(&self, tool_name: &str) -> Option<&dyn Tool> {
        self.0
            .iter()
            .into_iter()
            .find(|tool| tool.0 == tool_name)
            .map(|t| t.1.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use async_trait::async_trait;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    use crate::tool::{Tool, ToolSchema};

    #[derive(Debug, Clone, ToolSchema)]
    #[tool { input = TestToolInput }]
    /// A test tool
    pub struct TestTool;

    #[derive(Serialize, Deserialize, JsonSchema)]
    pub struct TestToolInput;

    #[async_trait]
    impl Tool for TestTool {
        async fn call(&self, _args: Value) -> Result<String> {
            Ok("test_result".to_string())
        }
    }

    #[test]
    pub fn test_add_tool() {
        let mut tools = super::Tools::new();
        let tool = TestTool {};
        tools.add_tool(tool.clone());
        assert!(tools.has_tool("test_tool"));
        assert_eq!(
            tools.get_tool("test_tool").unwrap().schema().name,
            tool.schema().name
        );
    }

    #[test]
    pub fn test_has_tool() {
        let mut tools = super::Tools::new();
        let tool = TestTool {};
        tools.add_tool(tool);
        assert!(tools.has_tool("test_tool"));
        assert!(!tools.has_tool("non_existent_tool"));
    }

    #[test]
    pub fn test_get_tool() {
        let mut tools = super::Tools::new();
        let tool = TestTool {};
        tools.add_tool(tool.clone());
        assert_eq!(
            tools.get_tool("test_tool").unwrap().schema().name,
            tool.schema().name
        );
        assert!(tools.get_tool("non_existent_tool").is_none());
    }

    #[test]
    pub fn test_derive_name() {
        assert_eq!((TestTool {}).name(), "test_tool");
    }

    #[test]
    pub fn test_derive_description() {
        assert_eq!((TestTool {}).description(), "A test tool");
    }

    #[test]
    pub fn test_derive_input_schema() {
        let schema = (TestTool {}).input_schema();
        let value = schema.as_value();
        assert_eq!(value.get("type").and_then(|v| v.as_str()), Some("null"));
    }

    #[test]
    pub fn test_definition_from_schema() {
        let tool = TestTool {};
        let definition = tool.schema();
        assert_eq!(definition.name, "test_tool");
        assert_eq!(definition.description, "A test tool");
    }

    #[derive(Debug, Clone, ToolSchema)]
    #[tool {
        name = "overridden",
        input = OverriddenToolInput,
        description = "explicit description",
    }]
    /// ignored doc comment
    pub struct OverriddenTool;

    #[derive(Serialize, Deserialize, JsonSchema)]
    pub struct OverriddenToolInput;

    #[async_trait]
    impl Tool for OverriddenTool {
        async fn call(&self, _args: Value) -> Result<String> {
            Ok("overridden".to_string())
        }
    }

    #[test]
    pub fn test_overridden_name() {
        assert_eq!((OverriddenTool {}).name(), "overridden");
    }

    #[test]
    pub fn test_overridden_description() {
        assert_eq!((OverriddenTool {}).description(), "explicit description");
    }
}
