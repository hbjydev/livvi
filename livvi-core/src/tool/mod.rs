use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: schemars::Schema,
}

#[async_trait]
pub trait Tool {
    fn schema(&self) -> ToolSchema;
    async fn call(&self, args: Value) -> Result<String>;
}

pub struct Tools(HashMap<String, Arc<dyn Tool>>);

impl Tools {
    pub fn new() -> Self {
        Tools(HashMap::new())
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

impl Default for Tools {
    fn default() -> Self {
        Self::new()
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

    #[derive(Debug, Clone)]
    pub struct TestTool;

    #[derive(Serialize, Deserialize, JsonSchema)]
    pub struct TestToolInput;

    #[async_trait]
    impl Tool for TestTool {
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: "test_tool".to_string(),
                description: "A test tool".to_string(),
                input_schema: schemars::schema_for!(TestToolInput),
            }
        }

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
}
