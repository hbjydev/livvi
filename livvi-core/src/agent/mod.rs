use std::sync::Arc;

use anyhow::{Result, anyhow};
use tokio::sync::{broadcast, mpsc};

use crate::{
    AgentEvent, LIVVI_BASE_SOUL_MD, context::Context, interrupt::Interrupt, tool::Toolbox,
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

    pub fn build(self) -> Result<(broadcast::Receiver<AgentEvent>, Agent<S>)> {
        let provider = self.provider.ok_or(anyhow!("Provider is required"))?;
        let state = self.state.ok_or(anyhow!("State is required"))?;
        let toolbox = self.toolbox.ok_or(anyhow!("Toolbox is required"))?;
        let input = self
            .input
            .ok_or(anyhow!("Input mpsc receiver is required"))?;

        let soul = self.soul.ok_or(anyhow!("Soul is required"))?;

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
