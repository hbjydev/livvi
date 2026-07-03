use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::{AgentEvent, agent::Agent, context::Context, interrupt::Interrupt};

impl<S: Sync + Send + 'static> Agent<S> {
    #[tracing::instrument(skip(self, interrupt, context))]
    pub(super) async fn run_turn(
        &mut self,
        interrupt: Interrupt,
        context: &mut Context,
    ) -> Result<()> {
        info!("Running turn with interrupt: {:?}", interrupt);
        let _ = self.output.send(AgentEvent::Started);

        let (tx, mut rx) = mpsc::channel(256);

        debug!("Beginning provider stream");
        let handle = self.provider.stream(
            tx,
            context.clone(),
            self.toolbox.schemas(),
        );

        let output = self.output.clone();

        let streaming_handle = tokio::spawn(async move {
            match rx.recv().await {
                Some(event) => match event {
                    crate::provider::ProviderEvent::Token(token) => {
                        info!("Received token from provider: {:?}", token);
                        let _ = output.send(AgentEvent::Token(token));
                    }
                    crate::provider::ProviderEvent::ThinkingToken(token) => {
                        info!("Received thinking token from provider: {:?}", token);
                        let _ = output.send(AgentEvent::ThinkingToken(token));
                    },
                    _ => todo!()
                }
                None => {},
            }
        });

        handle.await?;
        streaming_handle.await?;
        debug!("Provider stream completed");

        Ok(())
    }
}
