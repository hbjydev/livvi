use anyhow::Result;
use livvi_core::{
    agent::Agent,
    async_trait,
    compaction::WindowCompactor,
    interrupt::{ExternalEvent, Interrupt},
    memory::MemoryProvider,
    summarizer::Summarizer,
    tool::Toolbox,
};
use livvi_discord::tools::{discord_react, discord_send};
use livvi_discord::{DISCORD_INSTRUCTIONS, DiscordState, DiscordTransport};
use livvi_lcm::{LcmCompactor, LcmConfig, LcmSqliteStore};
use livvi_memini::MeminiMemoryProvider;
use livvi_memini::tools::{
    memory_briefing, memory_forget, memory_get, memory_list, memory_recall, memory_remember,
    memory_update,
};
use livvi_openai::OpenAIChatCompletionsProvider;
use livvi_store::{LivviSqliteStore, LivviStore};
use std::env;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub struct AppState {
    pub discord: DiscordState,
    pub memory: Arc<dyn MemoryProvider>,
}

impl AsRef<DiscordState> for AppState {
    fn as_ref(&self) -> &DiscordState {
        &self.discord
    }
}

impl AsRef<dyn MemoryProvider> for AppState {
    fn as_ref(&self) -> &dyn MemoryProvider {
        &*self.memory
    }
}

struct NoopMemoryProvider;

#[async_trait]
impl MemoryProvider for NoopMemoryProvider {
    async fn remember(
        &self,
        _ctx: livvi_core::memory::MemoryContext,
        _request: livvi_core::memory::RememberRequest,
    ) -> anyhow::Result<Option<livvi_core::memory::Memory>> {
        Err(anyhow::anyhow!("memory provider not configured"))
    }

    async fn recall(
        &self,
        _ctx: livvi_core::memory::MemoryContext,
        _request: livvi_core::memory::RecallRequest,
    ) -> anyhow::Result<Vec<livvi_core::memory::ScoredMemory>> {
        Ok(vec![])
    }

    async fn briefing(
        &self,
        _ctx: livvi_core::memory::MemoryContext,
        _request: livvi_core::memory::BriefingRequest,
    ) -> anyhow::Result<livvi_core::memory::Briefing> {
        Ok(livvi_core::memory::Briefing {
            namespace: String::new(),
            scope_header: None,
            facts: vec![],
            procedures: vec![],
            recent: vec![],
            pinned: vec![],
            children: None,
        })
    }

    async fn get(
        &self,
        _ctx: livvi_core::memory::MemoryContext,
        _id: &str,
    ) -> anyhow::Result<Option<livvi_core::memory::Memory>> {
        Ok(None)
    }

    async fn list(
        &self,
        _ctx: livvi_core::memory::MemoryContext,
        _request: livvi_core::memory::ListRequest,
    ) -> anyhow::Result<Vec<livvi_core::memory::Memory>> {
        Ok(vec![])
    }

    async fn forget(
        &self,
        _ctx: livvi_core::memory::MemoryContext,
        _id: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn update(
        &self,
        _ctx: livvi_core::memory::MemoryContext,
        _request: livvi_core::memory::UpdateRequest,
    ) -> anyhow::Result<Option<livvi_core::memory::Memory>> {
        Err(anyhow::anyhow!("memory provider not configured"))
    }

    fn clone_dyn(&self) -> Box<dyn MemoryProvider> {
        Box::new(NoopMemoryProvider)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    info!("Starting Livvi...");

    let discord_token = env::var("LIVVI_DISCORD_TOKEN")
        .or_else(|_| env::var("DISCORD_TOKEN"))
        .ok();

    let openai_api_key = env::var("LIVVI_OPENAI_API_KEY").ok();
    let openai_model =
        env::var("LIVVI_OPENAI_MODEL_NAME").unwrap_or_else(|_| "gpt-4o-mini".to_string());
    let openai_base_url = env::var("LIVVI_OPENAI_API_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

    let memini_base_url = env::var("LIVVI_MEMINI_BASE_URL").ok();
    let memini_api_key = env::var("LIVVI_MEMINI_API_KEY").ok();
    let memini_namespace =
        env::var("LIVVI_MEMINI_NAMESPACE").unwrap_or_else(|_| "livvi".to_string());

    let memini_configured = memini_base_url.as_ref().is_some_and(|u| !u.is_empty())
        && memini_api_key.as_ref().is_some_and(|k| !k.is_empty());

    // Without a Discord token there is no way to feed the agent loop, so just
    // wait for a shutdown signal.
    let discord_token = match discord_token {
        Some(token) => token,
        None => {
            warn!(
                "No Discord token configured (LIVVI_DISCORD_TOKEN or DISCORD_TOKEN); \
                 waiting for shutdown signal..."
            );
            shutdown_signal().await;
            return Ok(());
        }
    };

    let database_url =
        env::var("LIVVI_DATABASE_URL").unwrap_or_else(|_| "sqlite:livvi.db?mode=rwc".to_string());
    let store = LivviSqliteStore::connect(&database_url).await?;

    let (raw_tx, mut raw_rx) = mpsc::channel::<Interrupt>(256);
    let (resolved_tx, resolved_rx) = mpsc::channel::<Interrupt>(256);

    let discord_state = Arc::new(DiscordState::new(&discord_token));
    let transport = DiscordTransport::new(&discord_token, raw_tx).await?;

    let memory_provider: Arc<dyn MemoryProvider> = if memini_configured {
        Arc::new(MeminiMemoryProvider::new(
            livvi_memini::MeminiClient::new(
                memini_base_url.expect("base url checked above"),
                memini_api_key.expect("api key checked above"),
            ),
            &memini_namespace,
        ))
    } else {
        Arc::new(NoopMemoryProvider)
    };

    let app_state = AppState {
        discord: (*discord_state).clone(),
        memory: memory_provider,
    };

    let (provider, compactor): (
        Box<dyn livvi_core::provider::Provider>,
        Box<dyn livvi_core::compaction::Compactor>,
    ) = match openai_api_key {
        Some(key) => {
            let provider =
                OpenAIChatCompletionsProvider::new(&key, &openai_base_url, &openai_model)?;
            let compactor: Box<dyn livvi_core::compaction::Compactor> =
                if env::var("LIVVI_LCM_ENABLE")
                    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false)
                {
                    let lcm_database_url = env::var("LIVVI_LCM_DATABASE_URL")
                        .unwrap_or_else(|_| "sqlite:lcm.db?mode=rwc".to_string());
                    let store = Arc::new(LcmSqliteStore::connect(&lcm_database_url).await?);
                    let summarizer: Arc<dyn Summarizer> = Arc::new(provider.clone());
                    Box::new(LcmCompactor::new(summarizer, store, LcmConfig::from_env()))
                } else {
                    Box::new(WindowCompactor::default())
                };
            (Box::new(provider), compactor)
        }
        None => {
            warn!("LIVVI_OPENAI_API_KEY not set; using mock provider");
            (
                Box::new(livvi_core::provider::MockProvider::new(vec![])),
                Box::new(WindowCompactor::default()),
            )
        }
    };

    let memory_for_agent = memini_configured.then(|| app_state.memory.clone_dyn());

    let mut builder = Agent::builder()
        .with_provider(provider)
        .with_state(app_state)
        .with_toolbox({
            let mut toolbox = Toolbox::new();
            toolbox.add_tool(discord_send);
            toolbox.add_tool(discord_react);
            if memini_configured {
                toolbox.add_tool(memory_recall);
                toolbox.add_tool(memory_remember);
                toolbox.add_tool(memory_briefing);
                toolbox.add_tool(memory_get);
                toolbox.add_tool(memory_list);
                toolbox.add_tool(memory_update);
                toolbox.add_tool(memory_forget);
            }
            toolbox
        })
        .with_soul(format!(
            "{}\n\n{}",
            include_str!("../../SOUL.md"),
            DISCORD_INSTRUCTIONS
        ))
        .with_input(resolved_rx)
        .with_compactor(compactor);

    if let Some(memory_provider) = memory_for_agent {
        builder = builder.with_memory_provider(memory_provider);
    }

    let (_agent_events, agent) = builder.build()?;

    let agent_handle = tokio::spawn(async move {
        if let Err(e) = agent.run().await {
            tracing::error!("agent loop error: {e}");
        }
    });

    let resolver_handle = tokio::spawn(async move {
        while let Some(interrupt) = raw_rx.recv().await {
            match resolve_interrupt(interrupt, &store).await {
                Ok(Some(resolved)) => {
                    if resolved_tx.send(resolved).await.is_err() {
                        break;
                    }
                }
                Ok(None) => {}
                Err(e) => error!("failed to resolve interrupt: {e}"),
            }
        }
    });

    let mut discord_handle = tokio::spawn(async move {
        if let Err(e) = transport.run().await {
            tracing::error!("Discord transport error: {e}");
        }
    });

    tokio::select! {
        _ = shutdown_signal() => {
            info!("Shutdown signal received, terminating...");
        }
        _ = agent_handle => {
            warn!("agent loop exited");
        }
        _ = resolver_handle => {
            warn!("event resolver exited");
        }
        _ = &mut discord_handle => {
            warn!("Discord transport exited");
        }
    }

    Ok(())
}

async fn resolve_interrupt(
    interrupt: Interrupt,
    store: &impl LivviStore,
) -> Result<Option<Interrupt>> {
    let Interrupt::ExternalEvent(event) = interrupt;

    let resolved = resolve_external_event(event, store).await?;
    Ok(Some(Interrupt::external_event(resolved)))
}

async fn resolve_external_event(
    mut event: ExternalEvent,
    store: &impl LivviStore,
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

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = async {
                if let Some(sigterm) = sigterm.as_mut() {
                    sigterm.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use livvi_core::{
        agent::Agent,
        async_trait,
        interrupt::{ExternalAuthor, ExternalConversation, ExternalEvent, Interrupt},
        provider::{MockProvider, ProviderEvent},
        summarizer::Summarizer,
        tool::Toolbox,
    };
    use livvi_lcm::{LcmCompactor, LcmConfig, LcmSqliteStore, LcmStore};
    use livvi_store::{ConversationStorage, MockStore, PersonStorage};
    use serde_json::json;
    use std::sync::Arc;

    use super::*;

    #[derive(Clone)]
    struct MockSummarizer;

    #[async_trait]
    impl Summarizer for MockSummarizer {
        async fn summarize(
            &self,
            _prompt: Vec<livvi_core::model::Message>,
        ) -> anyhow::Result<String> {
            Ok("mock summary".to_string())
        }
    }

    fn make_event(
        content: &str,
        conversation_id: Option<livvi_store::ConversationId>,
    ) -> ExternalEvent {
        ExternalEvent {
            transport_kind: "internal".to_string(),
            event_type: "message".to_string(),
            content: Some(content.to_string()),
            author: ExternalAuthor {
                transport_kind: "internal".to_string(),
                transport_id: "user".to_string(),
                display_name: None,
                metadata: json!({}),
            },
            conversation: ExternalConversation {
                transport_kind: "internal".to_string(),
                transport_id: "test".to_string(),
                display_name: None,
                metadata: json!({}),
            },
            person_id: None,
            conversation_id,
            metadata: json!({}),
            timestamp: None,
        }
    }

    #[tokio::test]
    async fn daemon_persists_lcm_history() {
        let store = MockStore::new();
        let lcm_store = Arc::new(LcmSqliteStore::connect("sqlite::memory:").await.unwrap());

        let raw_event = make_event("hello", None);
        let resolved = resolve_external_event(raw_event, &store).await.unwrap();
        let conversation_id = resolved.conversation_id.clone().unwrap();

        let provider = MockProvider::new(vec![ProviderEvent::Token("ok".to_string())]);
        let summarizer: Arc<dyn Summarizer> = Arc::new(MockSummarizer);
        let config = LcmConfig {
            fresh_tail_count: 4,
            chunk_threshold: 50,
            condensation_count: 2,
            max_depth: 3,
        };
        let compactor = LcmCompactor::new(summarizer, lcm_store.clone(), config);

        let (input_tx, input_rx) = tokio::sync::mpsc::channel(16);
        let (_rx, agent) = Agent::builder()
            .with_provider(Box::new(provider))
            .with_state(())
            .with_toolbox(Toolbox::new())
            .with_soul("test soul".to_string())
            .with_input(input_rx)
            .with_compactor(compactor)
            .build()
            .unwrap();

        let send_conversation_id = conversation_id.clone();
        for i in 0..12 {
            input_tx
                .send(Interrupt::ExternalEvent(make_event(
                    &format!(
                        "user message {} with a lot of content to exceed the threshold",
                        i
                    ),
                    Some(send_conversation_id.clone()),
                )))
                .await
                .unwrap();
        }
        drop(input_tx);

        agent.run().await.unwrap();

        let messages = lcm_store.load_messages(&conversation_id).await.unwrap();
        let summaries = lcm_store.load_summaries(&conversation_id).await.unwrap();

        assert!(
            messages.len() >= 12,
            "expected at least 12 raw messages, got {}",
            messages.len()
        );
        assert!(
            !summaries.is_empty(),
            "expected at least one persisted summary"
        );
    }

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
