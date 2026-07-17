use anyhow::{Context, Result, anyhow};
use livvi_core::{agent::Agent, compaction::WindowCompactor, summarizer::Summarizer};
use livvi_discord::DiscordPlugin;
use livvi_lcm::{LcmCompactor, LcmConfig, LcmSqliteStore};
use livvi_memini::MeminiPlugin;
use livvi_openai::OpenAIChatCompletionsProvider;
use livvi_store::LivviSqliteStore;
use livvi_web::WebPlugin;
use opentelemetry::global;
use opentelemetry_sdk::{Resource, propagation::TraceContextPropagator, trace::SdkTracerProvider};
use std::env;
use std::io::IsTerminal;
use std::sync::Arc;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing().with_context(|| "failed to initialize tracing")?;
    info!("Starting Livvi...");

    let openai_api_key = env::var("LIVVI_OPENAI_API_KEY").ok();
    let openai_model =
        env::var("LIVVI_OPENAI_MODEL_NAME").unwrap_or_else(|_| "gpt-4o-mini".to_string());
    let openai_base_url = env::var("LIVVI_OPENAI_API_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

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

    let database_url =
        env::var("LIVVI_DATABASE_URL").unwrap_or_else(|_| "sqlite:livvi.db?mode=rwc".to_string());
    let store = LivviSqliteStore::connect(&database_url).await?;

    let mut builder = Agent::builder()
        .with_provider(provider)
        .with_compactor(compactor)
        .with_store(store.clone())
        .with_soul(include_str!("../../SOUL.md").to_string());

    if let Some(plugin) = DiscordPlugin::from_env() {
        builder = builder.with_plugin(plugin)?;
    } else {
        warn!(
            "No Discord token configured (LIVVI_DISCORD_TOKEN or DISCORD_TOKEN); Discord transport disabled"
        );
    }
    if let Some(plugin) = MeminiPlugin::from_env() {
        info!("memini memory provider enabled");
        builder = builder.with_plugin(plugin)?;
    } else {
        info!(
            "memini not configured (LIVVI_MEMINI_BASE_URL/LIVVI_MEMINI_API_KEY); memory tools disabled"
        );
    }
    let web_plugin = WebPlugin::from_env();
    if web_plugin.has_search() {
        info!("web_search and web_fetch enabled via LIVVI_SEARXNG_URL");
    } else {
        info!("LIVVI_SEARXNG_URL not set; web_fetch enabled, web_search disabled");
    }
    builder = builder.with_plugin(web_plugin)?;

    // Hold a sender so the agent's input channel stays open even with no transports.
    let _interrupt_tx = builder.interrupt_sender();

    let (_agent_events, agent, mut plugin_tasks) = builder.build()?;

    let agent_handle = tokio::spawn(async move { agent.run().await });

    let result = tokio::select! {
        _ = shutdown_signal() => {
            info!("Shutdown signal received, terminating...");
            Ok(())
        }
        result = agent_handle => {
            match result {
                Ok(Ok(())) => {
                    warn!("agent loop exited");
                    Ok(())
                }
                Ok(Err(e)) => Err(e.context("agent loop error")),
                Err(e) => Err(anyhow!("agent loop task panicked: {e}")),
            }
        }
        result = plugin_tasks.join_next(), if !plugin_tasks.is_empty() => {
            match result {
                Some(Ok(Ok(()))) => {
                    warn!("a plugin task exited");
                    Ok(())
                }
                Some(Ok(Err(e))) => Err(e.context("plugin task failed")),
                Some(Err(e)) => Err(anyhow!("plugin task panicked: {e}")),
                None => Ok(()),
            }
        }
    };

    result
}

fn init_tracing() -> Result<()> {
    let resource = Resource::builder().with_service_name("livvi").build();

    let span_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .build()
        .with_context(|| "failed building span exporter")?;

    let tracer_provider = SdkTracerProvider::builder()
        .with_resource(resource.clone())
        .with_batch_exporter(span_exporter)
        .build();

    global::set_text_map_propagator(TraceContextPropagator::new());
    global::set_tracer_provider(tracer_provider);

    let env_filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing_subscriber::filter::LevelFilter::INFO.into())
        .from_env_lossy();

    let otel_layer = tracing_opentelemetry::layer().with_tracer(global::tracer("livvi"));

    let log_format = env::var("LIVVI_LOG_FORMAT").unwrap_or_default();
    let fmt_layer: Box<dyn tracing_subscriber::Layer<tracing_subscriber::Registry> + Send + Sync> =
        if log_format.eq_ignore_ascii_case("json") {
            Box::new(tracing_subscriber::fmt::layer().json())
        } else {
            Box::new(
                tracing_subscriber::fmt::layer()
                    .with_target(false)
                    .with_ansi(std::io::stderr().is_terminal())
                    .compact()
                    .pretty(),
            )
        };

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(otel_layer)
        .with(env_filter)
        .init();

    Ok(())
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
        resolve::resolve_external_event,
        summarizer::Summarizer,
    };
    use livvi_lcm::{LcmCompactor, LcmConfig, LcmSqliteStore, LcmStore};
    use livvi_store::MockStore;
    use serde_json::json;
    use std::sync::Arc;

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

        let builder = Agent::builder()
            .with_provider(Box::new(provider))
            .with_soul("test soul".to_string())
            .with_compactor(compactor);
        let input_tx = builder.interrupt_sender();
        let (_rx, agent, _tasks) = builder.build().unwrap();

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
}
