use anyhow::Result;
use livvi_store::ConversationId;

use crate::{agent::Agent, context::Context, interrupt::Interrupt};

impl<S: Sync + Send + 'static> Agent<S> {
    #[tracing::instrument(skip(self, context))]
    pub(super) async fn handle_interrupt(
        &mut self,
        interrupt: Interrupt,
        context: &mut Context,
        conversation_id: &ConversationId,
        memory_namespace: &str,
    ) -> Result<Option<Interrupt>> {
        tracing::info!("Handling interrupt: {:?}", interrupt);

        match interrupt {
            Interrupt::ExternalEvent(..) => {
                self.handle_input_interrupt(interrupt, context, conversation_id, memory_namespace)
                    .await
            }
        }
    }

    #[tracing::instrument(skip(self, context))]
    async fn handle_input_interrupt(
        &mut self,
        interrupt: Interrupt,
        context: &mut Context,
        conversation_id: &ConversationId,
        memory_namespace: &str,
    ) -> Result<Option<Interrupt>> {
        tracing::info!("Handling input interrupt: {:?}", interrupt);
        self.run_turn(interrupt, context, conversation_id, memory_namespace)
            .await
    }
}
