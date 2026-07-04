use std::sync::Arc;

use anyhow::{Result, anyhow};
use tokio::sync::{broadcast, mpsc};

use crate::{
    AgentEvent, LIVVI_BASE_SOUL_MD, compaction::Compactor, context::Context, interrupt::Interrupt,
    tool::Toolbox,
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
}

impl<S: Sync + Send + 'static> Agent<S> {
    pub fn builder() -> AgentBuilder<S> {
        AgentBuilder::new()
    }

    pub async fn run(mut self) -> Result<()> {
        let mut ctx = Context::new(format!("{}\n\n{}", LIVVI_BASE_SOUL_MD, self.soul,));

        tracing::info!("Agent started running, beginning loop...");
        loop {
            match self.input.recv().await {
                Some(interrupt) => {
                    self.handle_interrupt(interrupt, &mut ctx).await?;
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
        interrupt::Interrupt,
        model::Message,
        provider::{MockProvider, ProviderEvent},
        tool::Toolbox,
    };
    use parking_lot::Mutex;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    struct RecordingCompactor {
        calls: Arc<Mutex<Vec<usize>>>,
    }

    impl Compactor for RecordingCompactor {
        fn compact(&self, messages: &[Message]) -> Vec<Message> {
            self.calls.lock().push(messages.len());
            WindowCompactor::default().compact(messages)
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

        let mut ctx = crate::context::Context::new("soul");
        agent
            .run_turn(Interrupt::message("hello"), &mut ctx)
            .await
            .unwrap();
        agent
            .run_turn(Interrupt::message("world"), &mut ctx)
            .await
            .unwrap();

        let calls = calls.lock();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|&n| n > 0));
    }
}
