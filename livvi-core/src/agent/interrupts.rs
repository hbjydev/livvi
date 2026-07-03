use anyhow::Result;

use crate::{agent::Agent, context::Context, interrupt::Interrupt};

impl<S: Sync + Send + 'static> Agent<S> {
    #[tracing::instrument(skip(self, context))]
    pub(super) async fn handle_interrupt(
        &mut self,
        interrupt: Interrupt,
        context: &mut Context,
    ) -> Result<()> {
        tracing::info!("Handling interrupt: {:?}", interrupt);

        match interrupt {
            Interrupt::Message(..) => self.handle_input_interrupt(interrupt, context).await?,
        }

        Ok(())
    }

    #[tracing::instrument(skip(self, context))]
    async fn handle_input_interrupt(
        &mut self,
        interrupt: Interrupt,
        context: &mut Context,
    ) -> Result<()> {
        tracing::info!("Handling input interrupt: {:?}", interrupt);
        self.run_turn(interrupt, context).await?;
        Ok(())
    }
}
