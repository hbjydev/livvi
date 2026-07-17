//! Identity/conversation resolution for raw interrupts.
//!
//! Transports emit interrupts carrying transport-level identifiers (Discord user IDs,
//! channel IDs, …). The functions in this module resolve those against the store into
//! stable person/conversation IDs before the agent loop dispatches them.

use anyhow::Result;
use livvi_store::LivviStore;

use crate::interrupt::{AllowToolEvent, ExternalEvent, Interrupt, ResetEvent};

/// Resolve a raw interrupt against the store.
///
/// Returns `Ok(None)` when the interrupt was fully handled and must be dropped
/// (the `AllowTool` arm: the permission was recorded, nothing to dispatch).
pub async fn resolve_interrupt(
    interrupt: Interrupt,
    store: &dyn LivviStore,
) -> Result<Option<Interrupt>> {
    match interrupt {
        Interrupt::ExternalEvent(event) => {
            let resolved = resolve_external_event(event, store).await?;
            Ok(Some(Interrupt::external_event(resolved)))
        }
        Interrupt::Reset(event) => {
            let resolved = resolve_reset_event(event, store).await?;
            Ok(Some(Interrupt::reset(resolved)))
        }
        Interrupt::AllowTool(event) => {
            resolve_allow_tool_event(event, store).await?;
            Ok(None)
        }
    }
}

pub async fn resolve_external_event(
    mut event: ExternalEvent,
    store: &dyn LivviStore,
) -> Result<ExternalEvent> {
    let person = store
        .ensure_identity(
            &event.author.transport_kind,
            &event.author.transport_id,
            event.author.display_name.clone(),
            event.author.metadata.clone(),
        )
        .await?;

    if let Some(name) = &event.author.display_name
        && person.display_name.as_ref() != Some(name)
    {
        store.add_also_known_as(&person.id, name.clone()).await?;
    }

    let conversation = store
        .ensure_conversation(
            &event.conversation.transport_kind,
            &event.conversation.transport_id,
            event.conversation.display_name.clone(),
            event.conversation.metadata.clone(),
        )
        .await?;

    store.add_participant(&conversation.id, &person.id).await?;

    event.person_id = Some(person.id);
    event.conversation_id = Some(conversation.id);

    Ok(event)
}

pub async fn resolve_reset_event(
    mut event: ResetEvent,
    store: &dyn LivviStore,
) -> Result<ResetEvent> {
    let person = store
        .ensure_identity(
            &event.author.transport_kind,
            &event.author.transport_id,
            event.author.display_name.clone(),
            event.author.metadata.clone(),
        )
        .await?;

    if let Some(name) = &event.author.display_name
        && person.display_name.as_ref() != Some(name)
    {
        store.add_also_known_as(&person.id, name.clone()).await?;
    }

    let conversation = store
        .ensure_conversation(
            &event.conversation.transport_kind,
            &event.conversation.transport_id,
            event.conversation.display_name.clone(),
            event.conversation.metadata.clone(),
        )
        .await?;

    store.add_participant(&conversation.id, &person.id).await?;

    event.person_id = Some(person.id);
    event.conversation_id = Some(conversation.id);

    Ok(event)
}

pub async fn resolve_allow_tool_event(
    mut event: AllowToolEvent,
    store: &dyn LivviStore,
) -> Result<()> {
    let conversation = store
        .ensure_conversation(
            &event.conversation.transport_kind,
            &event.conversation.transport_id,
            event.conversation.display_name.clone(),
            event.conversation.metadata.clone(),
        )
        .await?;

    store
        .set_tool_permission(&conversation.id, &event.tool_name, true)
        .await?;

    event.conversation_id = Some(conversation.id);

    Ok(())
}

#[cfg(test)]
mod tests {
    use livvi_store::{ConversationStorage, MockStore, PersonStorage};
    use serde_json::json;

    use super::*;
    use crate::interrupt::{ExternalAuthor, ExternalConversation};

    #[tokio::test]
    async fn resolver_creates_person_and_conversation() {
        let store = MockStore::new();

        let event = ExternalEvent {
            transport_kind: "discord".to_string(),
            event_type: "message".to_string(),
            content: Some("hello".to_string()),
            author: ExternalAuthor {
                transport_kind: "discord".to_string(),
                transport_id: "12345".to_string(),
                display_name: Some("hayden".to_string()),
                metadata: json!({ "discriminator": "0001" }),
            },
            conversation: ExternalConversation {
                transport_kind: "discord".to_string(),
                transport_id: "chan-1".to_string(),
                display_name: Some("general".to_string()),
                metadata: json!({ "guild_id": "111", "is_dm": false }),
            },
            person_id: None,
            conversation_id: None,
            metadata: json!({}),
            timestamp: None,
        };

        let resolved = resolve_external_event(event, &store).await.unwrap();

        assert!(resolved.person_id.is_some());
        assert!(resolved.conversation_id.is_some());

        let person = store
            .get_person(resolved.person_id.as_ref().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(person.display_name, Some("hayden".to_string()));

        let participants = store
            .get_participants(resolved.conversation_id.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(participants.len(), 1);
        assert_eq!(participants[0].id, resolved.person_id.unwrap());
    }

    #[tokio::test]
    async fn resolver_adds_alias_when_display_name_differs() {
        let store = MockStore::new();

        let first = ExternalEvent {
            transport_kind: "discord".to_string(),
            event_type: "message".to_string(),
            content: Some("hello".to_string()),
            author: ExternalAuthor {
                transport_kind: "discord".to_string(),
                transport_id: "12345".to_string(),
                display_name: Some("hayden".to_string()),
                metadata: json!({}),
            },
            conversation: ExternalConversation {
                transport_kind: "discord".to_string(),
                transport_id: "chan-1".to_string(),
                display_name: None,
                metadata: json!({}),
            },
            person_id: None,
            conversation_id: None,
            metadata: json!({}),
            timestamp: None,
        };

        let _ = resolve_external_event(first.clone(), &store).await.unwrap();

        let second = ExternalEvent {
            author: ExternalAuthor {
                display_name: Some("hayden2".to_string()),
                ..first.author.clone()
            },
            ..first.clone()
        };

        let resolved = resolve_external_event(second, &store).await.unwrap();

        let person = store
            .get_person(resolved.person_id.as_ref().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(person.display_name, Some("hayden".to_string()));
        assert_eq!(person.also_known_as, vec!["hayden2".to_string()]);
    }
}
