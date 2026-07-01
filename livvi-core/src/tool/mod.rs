use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait Tool {
    fn name(&self) -> String;
    fn schema(&self) -> String;
    async fn call(&self) -> Result<String>;
}

pub struct Tools(HashMap<String, Arc<dyn Tool>>);

impl Tools {
    pub fn new() -> Self {
        Tools(HashMap::new())
    }

    pub fn add_tool(&mut self, tool: impl Tool + 'static) {
        self.0.insert(tool.name(), Arc::new(tool));
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

    use crate::tool::Tool;

    #[derive(Debug, Clone)]
    pub struct TestTool;

    #[async_trait]
    impl Tool for TestTool {
        fn name(&self) -> String {
            "test_tool".to_string()
        }

        fn schema(&self) -> String {
            "test_schema".to_string()
        }

        async fn call(&self) -> Result<String> {
            Ok("test_result".to_string())
        }
    }

    #[test]
    pub fn test_add_tool() {
        let mut tools = super::Tools::new();
        let tool = TestTool {};
        tools.add_tool(tool.clone());
        assert!(tools.has_tool("test_tool"));
        assert_eq!(tools.get_tool("test_tool").unwrap().name(), tool.name());
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
        assert_eq!(tools.get_tool("test_tool").unwrap().name(), tool.name());
        assert!(tools.get_tool("non_existent_tool").is_none());
    }
}
