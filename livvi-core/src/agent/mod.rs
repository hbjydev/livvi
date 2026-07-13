use std::{num::NonZeroUsize, sync::Arc};

use anyhow::{Result, anyhow};
use livvi_store::ConversationId;
use lru::LruCache;
use tokio::sync::{broadcast, mpsc};

use crate::{
    AgentEvent, compaction::Compactor, context::Context, interrupt::Interrupt,
    memory::MemoryProvider, tool::Toolbox,
};

mod interrupts;
mod tools;
mod turns;

pub struct AgentBuilder<S: Sync + Send + 'static> {
    provider: Option<Box<dyn crate::provider::Provider>>,
    state: Option<Arc<S>>,
    input: Option<mpsc::Receiver<Interrupt>>,
    soul: Option<String>,
    toolbox: Option<Toolbox<S>>,
    compactor: Option<Box<dyn Compactor>>,
    memory_provider: Option<Box<dyn MemoryProvider>>,
    memory_namespace: Option<String>,
}

impl<S: Sync + Send + 'static> Default for AgentBuilder<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Sync + Send + 'static> AgentBuilder<S> {
    pub fn new() -> Self {
        Self {
            provider: None,
            state: None,
            input: None,
            soul: None,
            toolbox: None,
            compactor: None,
            memory_provider: None,
            memory_namespace: None,
        }
    }

    pub fn with_provider(mut self, provider: Box<dyn crate::provider::Provider>) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn with_state(mut self, state: S) -> Self {
        self.state = Some(Arc::new(state));
        self
    }

    pub fn with_toolbox(mut self, toolbox: Toolbox<S>) -> Self {
        self.toolbox = Some(toolbox);
        self
    }

    pub fn with_input(mut self, input: mpsc::Receiver<Interrupt>) -> Self {
        self.input = Some(input);
        self
    }

    pub fn with_soul(mut self, soul: String) -> Self {
        self.soul = Some(soul);
        self
    }

    pub fn with_compactor(mut self, compactor: impl Compactor) -> Self {
        self.compactor = Some(Box::new(compactor));
        self
    }

    pub fn with_memory_provider(mut self, provider: impl MemoryProvider) -> Self {
        self.memory_provider = Some(Box::new(provider));
        self
    }

    pub fn with_memory_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.memory_namespace = Some(namespace.into());
        self
    }

    pub fn build(self) -> Result<(broadcast::Receiver<AgentEvent>, Agent<S>)> {
        let provider = self.provider.ok_or(anyhow!("Provider is required"))?;
        let state = self.state.ok_or(anyhow!("State is required"))?;
        let toolbox = self.toolbox.ok_or(anyhow!("Toolbox is required"))?;
        let input = self
            .input
            .ok_or(anyhow!("Input mpsc receiver is required"))?;

        let soul = self.soul.ok_or(anyhow!("Soul is required"))?;

        let compactor = self
            .compactor
            .unwrap_or_else(|| Box::new(crate::compaction::WindowCompactor::default()));

        let memory_namespace = self.memory_namespace.unwrap_or_else(|| "livvi".to_string());

        let (tx, rx) = broadcast::channel(256);

        Ok((
            rx,
            Agent {
                provider: Arc::new(provider),
                state,
                input,
                output: tx,
                toolbox,
                soul,
                compactor,
                memory_provider: self.memory_provider,
                memory_namespace,
            },
        ))
    }
}

pub struct Agent<S: Sync + Send + 'static> {
    provider: Arc<Box<dyn crate::provider::Provider>>,
    state: Arc<S>,
    input: mpsc::Receiver<Interrupt>,
    output: broadcast::Sender<AgentEvent>,
    toolbox: Toolbox<S>,
    soul: String,
    compactor: Box<dyn Compactor>,
    memory_provider: Option<Box<dyn MemoryProvider>>,
    memory_namespace: String,
}

impl<S: Sync + Send + 'static> Agent<S> {
    pub fn builder() -> AgentBuilder<S> {
        AgentBuilder::new()
    }

    pub async fn run(mut self) -> Result<()> {
        let context_lru_capacity = std::env::var("LIVVI_AGENT_CONTEXT_LRU_CAPACITY")
            .ok()
            .and_then(|v| v.parse().ok())
            .and_then(NonZeroUsize::new)
            .unwrap_or(NonZeroUsize::new(1024).unwrap());
        let mut contexts: LruCache<ConversationId, Context> = LruCache::new(context_lru_capacity);
        let memory_namespace = self.memory_namespace.clone();

        tracing::info!("Agent started running, beginning loop...");
        loop {
            match self.input.recv().await {
                Some(interrupt) => {
                    let mut next_interrupt = Some(interrupt);
                    while let Some(interrupt) = next_interrupt {
                        let conversation_id = match &interrupt {
                            Interrupt::ExternalEvent(event) => event
                                .conversation_id
                                .clone()
                                .unwrap_or_else(|| ConversationId::from("global")),
                        };
                        let soul = self.soul.clone();
                        let ctx = contexts.get_or_insert_mut(conversation_id.clone(), || {
                            Context::new(soul, Some(conversation_id.clone()))
                        });
                        next_interrupt = self
                            .handle_interrupt(interrupt, ctx, &conversation_id, &memory_namespace)
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
        interrupt::{ExternalEvent, Interrupt},
        model::Message,
        provider::{MockProvider, ProviderEvent},
        tool::Toolbox,
    };
    use anyhow::Result;
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::mpsc;

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
        let toolbox = Toolbox::<()>::new();
        let (_input_tx, input_rx) = mpsc::channel(4);

        let calls = Arc::new(Mutex::new(Vec::new()));
        let compactor = RecordingCompactor {
            calls: calls.clone(),
        };

        let (_rx, mut agent) = Agent::builder()
            .with_provider(Box::new(provider))
            .with_input(input_rx)
            .with_state(())
            .with_toolbox(toolbox)
            .with_soul("test soul".to_string())
            .with_compactor(compactor)
            .build()
            .unwrap();

        let mut ctx = crate::context::Context::new("soul", Some("test".into()));
        agent
            .run_turn(
                Interrupt::message("hello"),
                &mut ctx,
                &"test".into(),
                "livvi",
            )
            .await
            .unwrap();
        agent
            .run_turn(
                Interrupt::message("world"),
                &mut ctx,
                &"test".into(),
                "livvi",
            )
            .await
            .unwrap();

        let calls = calls.lock();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|&n| n > 0));
    }

    #[tokio::test]
    async fn agent_keeps_per_conversation_contexts_independent() {
        let provider = MockProvider::new(vec![ProviderEvent::Token("hi".to_string())]);
        let toolbox = Toolbox::<()>::new();
        let (_input_tx, input_rx) = mpsc::channel(4);

        let compactor = ConversationRecordingCompactor::default();
        let calls = compactor.calls.clone();

        let (_rx, mut agent) = Agent::builder()
            .with_provider(Box::new(provider))
            .with_input(input_rx)
            .with_state(())
            .with_toolbox(toolbox)
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
                .handle_interrupt(interrupt, ctx, &conversation_id, "livvi")
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
}
