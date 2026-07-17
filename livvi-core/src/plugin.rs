use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::interrupt::Interrupt;
use crate::memory::MemoryProvider;
use crate::tool::ToolHandler;

/// A boxed background future spawned when the agent is built (e.g. a transport loop).
pub type PluginTask = Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>;

/// Mutable view of the agent under construction, handed to each [`Plugin`].
///
/// Constructed by `AgentBuilder::with_plugin`; never constructed directly.
pub struct PluginContext<'a> {
    pub(crate) state: &'a mut crate::state::StateMap,
    pub(crate) toolbox: &'a mut crate::tool::Toolbox,
    pub(crate) instructions: &'a mut Vec<String>,
    pub(crate) memory_provider: &'a mut Option<Box<dyn MemoryProvider>>,
    pub(crate) tasks: &'a mut Vec<PluginTask>,
    pub(crate) interrupt_tx: mpsc::Sender<Interrupt>,
}

impl PluginContext<'_> {
    /// Make `state` available to tools via the `State<T>` extractor.
    pub fn insert_state<T: Send + Sync + 'static>(&mut self, state: T) {
        self.state.insert(state);
    }

    /// Register a `#[tool]` function's generated handler.
    pub fn add_tool(&mut self, tool: impl ToolHandler) {
        self.toolbox.add_tool(tool);
    }

    /// Append a fragment to the agent's system prompt, joined with the base soul
    /// and other fragments by `\n\n`.
    pub fn add_instructions(&mut self, instructions: impl Into<String>) {
        self.instructions.push(instructions.into());
    }

    /// Install the memory provider. Registers the `memory_*` tools at build time.
    /// If one was already installed, it is replaced (a warning is logged).
    pub fn set_memory_provider(&mut self, provider: impl MemoryProvider) {
        if self.memory_provider.is_some() {
            tracing::warn!("memory provider replaced by plugin");
        }
        *self.memory_provider = Some(Box::new(provider));
    }

    /// A sender feeding the agent's input queue. Transports clone it to forward interrupts.
    pub fn interrupt_sender(&self) -> mpsc::Sender<Interrupt> {
        self.interrupt_tx.clone()
    }

    /// Register a background task (e.g. a transport) to be spawned by `build()`.
    pub fn spawn_task(&mut self, task: impl Future<Output = Result<()>> + Send + 'static) {
        self.tasks.push(Box::pin(task));
    }
}

/// A self-registering Livvi component (transport, memory backend, tool bundle, …).
pub trait Plugin: Send + Sync + 'static {
    /// Short human-readable name used in logs, e.g. `"discord"`.
    fn name(&self) -> &str;

    /// Contribute state, tools, instructions, providers, and background tasks.
    fn register(self, ctx: &mut PluginContext) -> Result<()>;
}
