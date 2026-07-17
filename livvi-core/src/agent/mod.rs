use std::{num::NonZeroUsize, sync::Arc};

use anyhow::{Context as _, Result, anyhow};
use livvi_store::ConversationId;
use lru::LruCache;
use tokio::sync::{broadcast, mpsc};
use tracing::Instrument;

use crate::{
    AgentEvent,
    compaction::Compactor,
    context::Context,
    interrupt::Interrupt,
    memory::MemoryProvider,
    state::StateMap,
    tool::{ToolHandler, Toolbox},
};

mod interrupts;
mod turns;

pub struct AgentBuilder {
    provider: Option<Box<dyn crate::provider::Provider>>,
    soul: Option<String>,
    instructions: Vec<String>,
    toolbox: Toolbox,
    state: StateMap,
    compactor: Option<Box<dyn Compactor>>,
    memory_provider: Option<Box<dyn MemoryProvider>>,
    store: Option<Arc<dyn livvi_store::LivviStore>>,
    tasks: Vec<crate::plugin::PluginTask>,
    interrupt_tx: mpsc::Sender<Interrupt>,
    interrupt_rx: Option<mpsc::Receiver<Interrupt>>,
}

impl Default for AgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentBuilder {
    pub fn new() -> Self {
        let (interrupt_tx, interrupt_rx) = mpsc::channel(256);
        Self {
            provider: None,
            soul: None,
            instructions: Vec::new(),
            toolbox: Toolbox::new(),
            state: StateMap::new(),
            compactor: None,
            memory_provider: None,
            store: None,
            tasks: Vec::new(),
            interrupt_tx,
            interrupt_rx: Some(interrupt_rx),
        }
    }

    pub fn with_provider(mut self, provider: Box<dyn crate::provider::Provider>) -> Self {
        self.provider = Some(provider);
        self
    }

    /// Insert a piece of state, made available to tools via the `State<T>` extractor.
    pub fn with_state<T: Send + Sync + 'static>(mut self, state: T) -> Self {
        self.state.insert(state);
        self
    }

    /// Register a `#[tool]` function's generated handler.
    pub fn with_tool(mut self, tool: impl ToolHandler) -> Self {
        self.toolbox.add_tool(tool);
        self
    }

    /// A sender feeding the agent's input queue. Clone it to forward interrupts
    /// into the agent from outside (transports, tests, examples).
    pub fn interrupt_sender(&self) -> mpsc::Sender<Interrupt> {
        self.interrupt_tx.clone()
    }

    pub fn with_soul(mut self, soul: String) -> Self {
        self.soul = Some(soul);
        self
    }

    pub fn with_compactor(mut self, compactor: impl Compactor) -> Self {
        self.compactor = Some(Box::new(compactor));
        self
    }

    /// Install the store used for interrupt resolution and tool permissions.
    pub fn with_store(mut self, store: impl livvi_store::LivviStore) -> Self {
        self.store = Some(Arc::new(store));
        self
    }

    pub fn with_memory_provider(mut self, provider: impl MemoryProvider) -> Self {
        self.memory_provider = Some(Box::new(provider));
        self
    }

    /// Register a plugin, letting it contribute state, tools, instructions,
    /// a memory provider, and background tasks.
    pub fn with_plugin(mut self, plugin: impl crate::plugin::Plugin) -> Result<Self> {
        let name = plugin.name().to_string();
        let mut ctx = crate::plugin::PluginContext {
            state: &mut self.state,
            toolbox: &mut self.toolbox,
            instructions: &mut self.instructions,
            memory_provider: &mut self.memory_provider,
            tasks: &mut self.tasks,
            interrupt_tx: self.interrupt_tx.clone(),
        };
        plugin
            .register(&mut ctx)
            .with_context(|| format!("failed to register plugin '{name}'"))?;
        tracing::info!(plugin = %name, "registered plugin");
        Ok(self)
    }

    /// Build the agent.
    ///
    /// Spawns plugin-registered background tasks, so this must be called inside
    /// a Tokio runtime. Returns the agent event stream, the agent, and the set
    /// of spawned plugin tasks.
    pub fn build(
        mut self,
    ) -> Result<(
        broadcast::Receiver<AgentEvent>,
        Agent,
        tokio::task::JoinSet<Result<()>>,
    )> {
        let provider = self.provider.ok_or(anyhow!("Provider is required"))?;
        let soul = self.soul.ok_or(anyhow!("Soul is required"))?;

        if self.memory_provider.is_some() {
            self.toolbox.add_tool(crate::memory::tools::memory_recall);
            self.toolbox.add_tool(crate::memory::tools::memory_remember);
            self.toolbox.add_tool(crate::memory::tools::memory_briefing);
            self.toolbox.add_tool(crate::memory::tools::memory_get);
            self.toolbox.add_tool(crate::memory::tools::memory_list);
            self.toolbox.add_tool(crate::memory::tools::memory_update);
            self.toolbox.add_tool(crate::memory::tools::memory_forget);
        }

        let mut assembled = soul;
        for fragment in &self.instructions {
            assembled.push_str("\n\n");
            assembled.push_str(fragment);
        }
        let soul = format!(
            "{}\n\n{}\n\n{}\n\n{}",
            assembled,
            include_str!("../../prompts/scratchpad.md"),
            include_str!("../../prompts/input-event-loop.md"),
            include_str!("../../prompts/memory.md")
        );

        let compactor = self
            .compactor
            .unwrap_or_else(|| Box::new(crate::compaction::WindowCompactor::default()));

        let input = self
            .interrupt_rx
            .take()
            .expect("interrupt receiver already taken");

        let (tx, rx) = broadcast::channel(256);

        let mut tasks = tokio::task::JoinSet::new();
        for task in self.tasks {
            tasks.spawn(task);
        }

        Ok((
            rx,
            Agent {
                provider: Arc::new(provider),
                state: self.state,
                input,
                output: tx,
                toolbox: self.toolbox,
                soul,
                compactor,
                memory_provider: self.memory_provider,
                store: self.store,
            },
            tasks,
        ))
    }
}

pub struct Agent {
    provider: Arc<Box<dyn crate::provider::Provider>>,
    state: StateMap,
    input: mpsc::Receiver<Interrupt>,
    output: broadcast::Sender<AgentEvent>,
    toolbox: Toolbox,
    soul: String,
    compactor: Box<dyn Compactor>,
    memory_provider: Option<Box<dyn MemoryProvider>>,
    store: Option<Arc<dyn livvi_store::LivviStore>>,
}

impl Agent {
    pub fn builder() -> AgentBuilder {
        AgentBuilder::new()
    }

    pub async fn run(mut self) -> Result<()> {
        let context_lru_capacity = std::env::var("LIVVI_AGENT_CONTEXT_LRU_CAPACITY")
            .ok()
            .and_then(|v| v.parse().ok())
            .and_then(NonZeroUsize::new)
            .unwrap_or(NonZeroUsize::new(1024).unwrap());
        let mut contexts: LruCache<ConversationId, Context> = LruCache::new(context_lru_capacity);

        tracing::info!("Agent started running, beginning loop...");
        loop {
            match self.input.recv().await {
                Some(interrupt) => {
                    let mut next_interrupt = Some(interrupt);
                    while let Some(interrupt) = next_interrupt {
                        let interrupt = match &self.store {
                            Some(store) => {
                                let resolve_span = tracing::info_span!(
                                    "resolve_interrupt",
                                    otel.status_code = tracing::field::Empty,
                                    otel.status_description = tracing::field::Empty,
                                );
                                let result =
                                    crate::resolve::resolve_interrupt(interrupt, store.as_ref())
                                        .instrument(resolve_span.clone())
                                        .await;
                                match result {
                                    Ok(Some(resolved)) => resolved,
                                    // AllowTool: permission granted, nothing to dispatch
                                    Ok(None) => break,
                                    Err(e) => {
                                        resolve_span.record("otel.status_code", "ERROR");
                                        resolve_span.record(
                                            "otel.status_description",
                                            tracing::field::display(&e),
                                        );
                                        tracing::error!("failed to resolve interrupt: {e}");
                                        break;
                                    }
                                }
                            }
                            None => interrupt,
                        };
                        let conversation_id = match &interrupt {
                            Interrupt::ExternalEvent(event) => event
                                .conversation_id
                                .clone()
                                .unwrap_or_else(|| ConversationId::from("global")),
                            Interrupt::Reset(event) => event
                                .conversation_id
                                .clone()
                                .unwrap_or_else(|| ConversationId::from("global")),
                            Interrupt::AllowTool(event) => event
                                .conversation_id
                                .clone()
                                .unwrap_or_else(|| ConversationId::from("global")),
                        };
                        tracing::info!(
                            conversation_id = %conversation_id,
                            interrupt = %interrupt,
                            "received interrupt"
                        );
                        let soul = self.soul.clone();
                        let ctx = contexts.get_or_insert_mut(conversation_id.clone(), || {
                            Context::new(soul, Some(conversation_id.clone()))
                        });
                        next_interrupt = self
                            .handle_interrupt(interrupt, ctx, &conversation_id)
                            .await?;
                    }
                }
                None => {
                    tracing::warn!("Agent input channel disconnected, exiting loop.");
                    break;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        compaction::{Compactor, WindowCompactor},
        interrupt::{ExternalEvent, Interrupt, ResetEvent},
        model::Message,
        provider::{MockProvider, ProviderEvent},
    };
    use anyhow::Result;
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::sync::Arc;

    struct RecordingCompactor {
        calls: Arc<Mutex<Vec<usize>>>,
    }

    #[derive(Default)]
    struct ConversationRecordingCompactor {
        calls: Arc<Mutex<Vec<(String, usize)>>>,
    }

    #[async_trait]
    impl Compactor for RecordingCompactor {
        async fn compact(
            &self,
            messages: &[Message],
            _conversation_id: &livvi_store::ConversationId,
        ) -> Result<Vec<Message>> {
            self.calls.lock().push(messages.len());
            Ok(WindowCompactor::default()
                .compact(messages, _conversation_id)
                .await?)
        }
    }

    #[async_trait]
    impl Compactor for ConversationRecordingCompactor {
        async fn compact(
            &self,
            messages: &[Message],
            conversation_id: &livvi_store::ConversationId,
        ) -> Result<Vec<Message>> {
            self.calls
                .lock()
                .push((conversation_id.0.clone(), messages.len()));
            Ok(WindowCompactor::default()
                .compact(messages, conversation_id)
                .await?)
        }
    }

    #[tokio::test]
    async fn agent_invokes_compactor_each_turn() {
        let provider = MockProvider::new(vec![ProviderEvent::Token("hi".to_string())]);

        let calls = Arc::new(Mutex::new(Vec::new()));
        let compactor = RecordingCompactor {
            calls: calls.clone(),
        };

        let (_rx, mut agent, _tasks) = Agent::builder()
            .with_provider(Box::new(provider))
            .with_soul("test soul".to_string())
            .with_compactor(compactor)
            .build()
            .unwrap();

        let mut ctx = crate::context::Context::new("soul", Some("test".into()));
        agent
            .run_turn(Interrupt::message("hello"), &mut ctx, &"test".into())
            .await
            .unwrap();
        agent
            .run_turn(Interrupt::message("world"), &mut ctx, &"test".into())
            .await
            .unwrap();

        let calls = calls.lock();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|&n| n > 0));
    }

    #[tokio::test]
    async fn agent_nudges_when_required_tool_not_used() {
        use crate::tool::tool;

        #[tool(is_required = true)]
        async fn required_tool() -> Result<(), ()> {
            Ok(())
        }

        let provider = MockProvider::new(vec![ProviderEvent::Token("hi".to_string())]);

        let (_rx, mut agent, _tasks) = Agent::builder()
            .with_provider(Box::new(provider))
            .with_tool(required_tool)
            .with_soul("test soul".to_string())
            .with_compactor(WindowCompactor::default())
            .build()
            .unwrap();

        let mut ctx = crate::context::Context::new("soul", Some("test".into()));
        agent
            .run_turn(Interrupt::message("hello"), &mut ctx, &"test".into())
            .await
            .unwrap();

        let nudges = ctx
            .turns
            .iter()
            .filter(|m| {
                m.role == crate::model::Role::User
                    && m.content.as_deref().unwrap_or("").contains("<system>")
            })
            .count();

        assert_eq!(nudges, 2, "expected two nudges before giving up");
    }

    #[tokio::test]
    async fn agent_nudges_when_scratchpad_text_without_action() {
        use crate::tool::tool;

        #[tool]
        async fn optional_tool() -> Result<(), ()> {
            Ok(())
        }

        let provider = MockProvider::new(vec![ProviderEvent::Token("hi".to_string())]);

        let (_rx, mut agent, _tasks) = Agent::builder()
            .with_provider(Box::new(provider))
            .with_tool(optional_tool)
            .with_soul("test soul".to_string())
            .with_compactor(WindowCompactor::default())
            .build()
            .unwrap();

        let mut ctx = crate::context::Context::new("soul", Some("test".into()));
        agent
            .run_turn(Interrupt::message("hello"), &mut ctx, &"test".into())
            .await
            .unwrap();

        let nudges = ctx
            .turns
            .iter()
            .filter(|m| {
                m.role == crate::model::Role::User
                    && m.content
                        .as_deref()
                        .unwrap_or("")
                        .contains("plain assistant text is not visible")
            })
            .count();

        assert_eq!(
            nudges, 2,
            "expected two nudges before giving up on scratchpad-only text"
        );
    }

    #[tokio::test]
    async fn agent_keeps_per_conversation_contexts_independent() {
        let provider = MockProvider::new(vec![ProviderEvent::Token("hi".to_string())]);

        let compactor = ConversationRecordingCompactor::default();
        let calls = compactor.calls.clone();

        let (_rx, mut agent, _tasks) = Agent::builder()
            .with_provider(Box::new(provider))
            .with_soul("test soul".to_string())
            .with_compactor(compactor)
            .build()
            .unwrap();

        let mut contexts: HashMap<ConversationId, Context> = HashMap::new();
        for (content, conv_id) in [("hello A", "A"), ("hello B", "B"), ("hello A again", "A")] {
            let conversation_id = ConversationId::from(conv_id);
            let interrupt = Interrupt::ExternalEvent(ExternalEvent {
                transport_kind: "internal".to_string(),
                event_type: "message".to_string(),
                content: Some(content.to_string()),
                author: crate::interrupt::ExternalAuthor {
                    transport_kind: "internal".to_string(),
                    transport_id: "user".to_string(),
                    display_name: None,
                    metadata: serde_json::Value::Null,
                },
                conversation: crate::interrupt::ExternalConversation {
                    transport_kind: "internal".to_string(),
                    transport_id: conv_id.to_string(),
                    display_name: None,
                    metadata: serde_json::Value::Null,
                },
                person_id: None,
                conversation_id: Some(conversation_id.clone()),
                metadata: serde_json::Value::Null,
                timestamp: None,
            });
            let ctx = contexts
                .entry(conversation_id.clone())
                .or_insert_with(|| Context::new("test soul", Some(conversation_id.clone())));
            agent
                .handle_interrupt(interrupt, ctx, &conversation_id)
                .await
                .unwrap();
        }

        let calls = calls.lock();
        let a_calls: Vec<usize> = calls
            .iter()
            .filter(|(id, _)| id == "A")
            .map(|(_, count)| *count)
            .collect();
        let b_calls: Vec<usize> = calls
            .iter()
            .filter(|(id, _)| id == "B")
            .map(|(_, count)| *count)
            .collect();

        assert_eq!(a_calls, vec![1, 3]);
        assert_eq!(b_calls, vec![1]);
    }

    #[tokio::test]
    async fn reset_interrupt_clears_context() {
        let provider = MockProvider::new(vec![]);

        let (_rx, mut agent, _tasks) = Agent::builder()
            .with_provider(Box::new(provider))
            .with_soul("test soul".to_string())
            .with_compactor(WindowCompactor::default())
            .build()
            .unwrap();

        let conversation_id = ConversationId::from("test");
        let mut context = Context::new("test soul", Some(conversation_id.clone()));
        context.push_user("hello", None);
        context.push_assistant("hi", None::<String>);

        let reset_interrupt = Interrupt::reset(ResetEvent::new(
            "internal",
            crate::interrupt::ExternalAuthor {
                transport_kind: "internal".to_string(),
                transport_id: "user".to_string(),
                display_name: None,
                metadata: serde_json::Value::Null,
            },
            crate::interrupt::ExternalConversation {
                transport_kind: "internal".to_string(),
                transport_id: "test".to_string(),
                display_name: None,
                metadata: serde_json::Value::Null,
            },
        ));

        agent
            .handle_interrupt(reset_interrupt, &mut context, &conversation_id)
            .await
            .unwrap();

        assert!(context.turns.is_empty());
        assert_eq!(context.system.len(), 1);
    }

    #[tokio::test]
    async fn empty_response_is_replaced_with_no_content() {
        let provider = MockProvider::new(vec![]);

        let (_rx, mut agent, _tasks) = Agent::builder()
            .with_provider(Box::new(provider))
            .with_soul("test soul".to_string())
            .with_compactor(WindowCompactor::default())
            .build()
            .unwrap();

        let mut context = Context::new("test soul", Some("test".into()));
        agent
            .run_turn(Interrupt::message("hello"), &mut context, &"test".into())
            .await
            .unwrap();

        let last = context.turns.last().expect("should have a turn");
        assert_eq!(last.role, crate::model::Role::Assistant);
        assert_eq!(last.content.as_deref(), Some("(no content)"));
    }

    #[tokio::test]
    async fn build_registers_memory_tools_only_with_memory_provider() {
        let provider = MockProvider::new(vec![]);
        let (_rx, agent, _tasks) = Agent::builder()
            .with_provider(Box::new(provider))
            .with_soul("test soul".to_string())
            .with_memory_provider(crate::memory::noop::NoopMemoryProvider)
            .build()
            .unwrap();

        assert!(agent.toolbox.has_tool("memory_recall"));
        assert!(agent.toolbox.has_tool("memory_forget"));

        let provider = MockProvider::new(vec![]);
        let (_rx, agent, _tasks) = Agent::builder()
            .with_provider(Box::new(provider))
            .with_soul("test soul".to_string())
            .build()
            .unwrap();

        assert!(!agent.toolbox.has_tool("memory_recall"));
    }
}
