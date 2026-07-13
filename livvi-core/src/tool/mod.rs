use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde_json::Value;

use crate::{context, model};

pub use livvi_core_macros::tool;

/// Errors that can occur while extracting values from a tool call context.
#[derive(Debug)]
pub enum ToolExtractError {
    /// The provided arguments failed JSON Schema validation.
    InvalidArguments(String),

    /// The provided arguments validated but could not be deserialized into the input type.
    Deserialization(serde_json::Error),

    /// The generated schema was invalid and could not be used for validation.
    Schema(anyhow::Error),
}

impl std::fmt::Display for ToolExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolExtractError::InvalidArguments(e) => write!(f, "invalid arguments: {e}"),
            ToolExtractError::Deserialization(e) => {
                write!(f, "failed to deserialize arguments: {e}")
            }
            ToolExtractError::Schema(e) => write!(f, "invalid schema: {e}"),
        }
    }
}

impl std::error::Error for ToolExtractError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ToolExtractError::InvalidArguments(_) => None,
            ToolExtractError::Deserialization(e) => Some(e),
            ToolExtractError::Schema(e) => e.source(),
        }
    }
}

impl From<serde_json::Error> for ToolExtractError {
    fn from(value: serde_json::Error) -> Self {
        ToolExtractError::Deserialization(value)
    }
}

impl From<anyhow::Error> for ToolExtractError {
    fn from(value: anyhow::Error) -> Self {
        ToolExtractError::Schema(value)
    }
}

/// Context passed to tools when they are invoked by the agent loop.
///
/// This is a catch-all container for "things the agent loop needs to give to the tool that
/// aren't its arguments". It is parameterised by the application state type `S`, which is
/// provided by the user when the agent is constructed.
pub struct ToolContext<'a, S> {
    /// The current conversation transcript.
    pub agent_context: &'a context::Context,

    /// The provider-supplied ID for this tool call.
    pub tool_call_id: &'a str,

    /// The user-provided application state.
    pub state: &'a S,
}

/// Extracts a value from a [`ToolContext`] and the raw JSON tool arguments.
///
/// Implemented by extractor types such as [`Input`], [`State`], [`Transcript`], and [`ToolCallId`].
/// Users can also implement this trait for their own extractor types.
pub trait FromToolContext<'a, S>: Sized {
    /// Extract the value from the given context and arguments.
    fn from_tool_context(
        ctx: &'a ToolContext<'a, S>,
        args: &'a Value,
    ) -> Result<Self, ToolExtractError>;
}

/// Extracts the tool input from the raw JSON arguments.
///
/// Exactly one `Input<T>` extractor is allowed per tool. `T` must implement [`serde::Deserialize`]
/// and [`schemars::JsonSchema`]; the latter is used to derive the tool's JSON Schema input.
pub struct Input<T>(pub T);

impl<'a, S, T> FromToolContext<'a, S> for Input<T>
where
    T: serde::de::DeserializeOwned + schemars::JsonSchema,
{
    fn from_tool_context(
        _ctx: &'a ToolContext<'a, S>,
        args: &'a Value,
    ) -> Result<Self, ToolExtractError> {
        let schema = schemars::schema_for!(T);
        let validator = jsonschema::validator_for(schema.as_value())
            .map_err(|e| ToolExtractError::Schema(anyhow::anyhow!(e)))?;
        validator
            .validate(args)
            .map_err(|e| ToolExtractError::InvalidArguments(e.to_string()))?;
        let value = serde_json::from_value(args.clone())?;
        Ok(Input(value))
    }
}

/// Extracts a reference to a piece of the application state.
///
/// `T` is the target type, extracted from the context's state via [`AsRef`]. The function
/// parameter looks like `State(state): State<AppState>` where the agent's state is, for example,
/// `Arc<AppState>`.
pub struct State<'a, T: ?Sized>(pub &'a T);

impl<'a, S, T: ?Sized> FromToolContext<'a, S> for State<'a, T>
where
    S: AsRef<T>,
{
    fn from_tool_context(
        ctx: &'a ToolContext<'a, S>,
        _args: &'a Value,
    ) -> Result<Self, ToolExtractError> {
        Ok(State(ctx.state.as_ref()))
    }
}

/// Extracts a reference to the current conversation transcript.
pub struct Context<'a>(pub &'a context::Context);

impl<'a, S> FromToolContext<'a, S> for Context<'a> {
    fn from_tool_context(
        ctx: &'a ToolContext<'a, S>,
        _args: &'a Value,
    ) -> Result<Self, ToolExtractError> {
        Ok(Context(ctx.agent_context))
    }
}

/// Extracts the provider-supplied tool call ID.
pub struct ToolCallId<'a>(pub &'a str);

impl<'a, S> FromToolContext<'a, S> for ToolCallId<'a> {
    fn from_tool_context(
        ctx: &'a ToolContext<'a, S>,
        _args: &'a Value,
    ) -> Result<Self, ToolExtractError> {
        Ok(ToolCallId(ctx.tool_call_id))
    }
}

/// The output of a tool call before it is turned into a [`model::ToolResult`].
#[derive(Debug, Clone)]
pub enum ToolCallOutput {
    /// The tool call succeeded. The string is the serialized form of the return value.
    Success(String),
    /// The tool call failed. The string is the serialized form of the error.
    Error(String),
}

impl ToolCallOutput {
    /// Attach a tool call ID to this output to produce a full [`model::ToolResult`].
    pub fn into_tool_result(self, id: impl Into<String>) -> model::ToolResult {
        let id = id.into();
        match self {
            ToolCallOutput::Success(content) => model::ToolResult {
                id,
                content,
                is_error: false,
            },
            ToolCallOutput::Error(content) => model::ToolResult {
                id,
                content,
                is_error: true,
            },
        }
    }
}

/// A concrete description of a tool that can be passed to a provider.
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: schemars::Schema,
}

/// Implemented by the generated wrapper for a `#[tool]` function.
///
/// This trait is object-safe and is used by [`Tools`] to store heterogeneous tools.
#[async_trait]
pub trait ToolHandler<S: Send + Sync + 'static>: Send + Sync + 'static {
    /// The static schema for this tool.
    fn schema(&self) -> ToolDefinition;

    /// Invoke the tool with the given context and arguments.
    async fn call(&self, ctx: &ToolContext<'_, S>, args: Value) -> ToolCallOutput;
}

/// A registry of tools, parameterised by the application state type.
pub struct Toolbox<S: Send + Sync + 'static> {
    tools: HashMap<String, Arc<dyn ToolHandler<S>>>,
}

impl<S: Send + Sync + 'static> Clone for Toolbox<S> {
    fn clone(&self) -> Self {
        Toolbox {
            tools: self.tools.clone(),
        }
    }
}

impl<S: Send + Sync + 'static> Toolbox<S> {
    /// Create an empty tool registry.
    pub fn new() -> Self {
        Toolbox {
            tools: HashMap::new(),
        }
    }

    /// The schemas of all registered tools, suitable for passing to a provider.
    pub fn schemas(&self) -> HashMap<String, ToolDefinition> {
        self.tools
            .iter()
            .map(|(tool_name, tool)| (tool_name.clone(), tool.schema()))
            .collect()
    }

    /// Add a tool to the registry.
    ///
    /// The tool is typically a `#[tool]` function; the macro generates a wrapper struct that
    /// implements [`ToolHandler`].
    pub fn add_tool(&mut self, tool: impl ToolHandler<S> + 'static) {
        let schema = tool.schema();
        self.tools.insert(schema.name, Arc::new(tool));
    }

    /// Returns whether a tool with the given name is registered.
    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.tools.contains_key(tool_name)
    }

    /// Returns the tool with the given name, if any.
    pub fn get_tool(&self, tool_name: &str) -> Option<&dyn ToolHandler<S>> {
        self.tools.get(tool_name).map(|tool| tool.as_ref())
    }
}

impl<S: Send + Sync + 'static> Default for Toolbox<S> {
    fn default() -> Self {
        Toolbox::new()
    }
}
