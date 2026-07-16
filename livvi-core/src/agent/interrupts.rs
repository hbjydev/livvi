use anyhow::Result;
use livvi_store::ConversationId;

use crate::{AgentEvent, agent::Agent, context::Context, interrupt::Interrupt};

impl<S: Sync + Send + 'static> Agent<S> {
    pub(super) async fn handle_interrupt(
        &mut self,
        interrupt: Interrupt,
        context: &mut Context,
        conversation_id: &ConversationId,
    ) -> Result<Option<Interrupt>> {
        match interrupt {
            Interrupt::ExternalEvent(..) => {
                self.handle_input_interrupt(interrupt, context, conversation_id)
                    .await
            }
            Interrupt::Reset(..) => {
                self.handle_reset_interrupt(interrupt, context, conversation_id)
                    .await
            }
        }
    }

    async fn handle_input_interrupt(
        &mut self,
        interrupt: Interrupt,
        context: &mut Context,
        conversation_id: &ConversationId,
    ) -> Result<Option<Interrupt>> {
        self.run_turn(interrupt, context, conversation_id).await
    }

    async fn handle_reset_interrupt(
        &mut self,
        _interrupt: Interrupt,
        context: &mut Context,
        conversation_id: &ConversationId,
    ) -> Result<Option<Interrupt>> {
        context.clear();
        let _ = self.output.send(AgentEvent::Status(format!(
            "Context reset for conversation {}",
            conversation_id
        )));
        Ok(None)
    }
}
