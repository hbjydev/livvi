use livvi_core::tool::{Input, State, tool};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::WebState;

fn default_search_limit() -> usize {
    10
}

/// Input to [`web_search`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebSearchInput {
    /// The search query.
    pub query: String,

    /// Maximum number of results to return (default 10).
    #[serde(default = "default_search_limit")]
    pub limit: usize,
}

/// One search result returned by [`web_search`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebSearchResult {
    /// Title of the result.
    pub title: String,

    /// URL of the result.
    pub url: String,

    /// Short snippet or summary of the result.
    pub content: String,
}

/// Output of [`web_search`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebSearchOutput {
    /// Matching search results, up to the requested limit.
    pub results: Vec<WebSearchResult>,
}

#[derive(Debug, Clone, Deserialize)]
struct SearxResult {
    title: Option<String>,
    url: Option<String>,
    content: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SearxResponse {
    results: Vec<SearxResult>,
}

/// Search the web using a configured SearxNG instance.
///
/// Returns a list of search results with titles, URLs, and short snippets.
/// Use this when you need current information that is not in your training
/// data.
#[tool]
pub async fn web_search(
    Input(input): Input<WebSearchInput>,
    State(state): State<'_, WebState>,
) -> Result<WebSearchOutput, String> {
    let base = state
        .searxng_url
        .as_ref()
        .ok_or("web_search is not configured: no SearxNG URL set")?;

    let url = reqwest::Url::parse_with_params(
        &format!("{}/search", base.trim_end_matches('/')),
        &[
            ("q", input.query.as_str()),
            ("format", "json"),
            ("categories", "general"),
        ],
    )
    .map_err(|e| format!("failed to build search URL: {e}"))?;

    tracing::debug!(%url, "sending web_search request");

    let response = state
        .client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("web_search request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("web_search returned HTTP {status}"));
    }

    let body: SearxResponse = response
        .json()
        .await
        .map_err(|e| format!("failed to parse web_search response: {e}"))?;

    let results = body
        .results
        .into_iter()
        .filter_map(|r| {
            Some(WebSearchResult {
                title: r.title.unwrap_or_default(),
                url: r.url.unwrap_or_default(),
                content: r.content.unwrap_or_default(),
            })
            .filter(|r| !r.url.is_empty())
        })
        .take(input.limit)
        .collect();

    Ok(WebSearchOutput { results })
}

#[cfg(test)]
mod tests {
    use super::*;
    use livvi_core::context::Context as AgentContext;
    use livvi_core::tool::Toolbox;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn web_search_returns_results() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "rust programming"))
            .and(query_param("format", "json"))
            .and(query_param("categories", "general"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "query": "rust programming",
                "results": [
                    {
                        "title": "Rust Language",
                        "url": "https://www.rust-lang.org/",
                        "content": "A language empowering everyone to build reliable software."
                    },
                    {
                        "title": "The Rust Book",
                        "url": "https://doc.rust-lang.org/book/",
                        "content": "The official book on Rust."
                    }
                ]
            })))
            .mount(&server)
            .await;

        let state = WebState::new(Some(server.uri()));
        let mut toolbox = Toolbox::<WebState>::new();
        toolbox.add_tool(web_search);

        let result = toolbox
            .get_tool("web_search")
            .unwrap()
            .call(
                &livvi_core::tool::ToolContext {
                    agent_context: &AgentContext::new("soul", None),
                    tool_call_id: "call-1",
                    state: &state,
                    memory_provider: None,
                },
                serde_json::json!({"query": "rust programming", "limit": 1}),
            )
            .await;

        let success = match result {
            livvi_core::tool::ToolCallOutput::Success(s) => s,
            other => panic!("expected success, got {other:?}"),
        };
        let output: WebSearchOutput = serde_json::from_str(&success).unwrap();
        assert_eq!(output.results.len(), 1);
        assert_eq!(output.results[0].title, "Rust Language");
    }

    #[tokio::test]
    async fn web_search_errors_without_config() {
        let state = WebState::new(None);
        let mut toolbox = Toolbox::<WebState>::new();
        toolbox.add_tool(web_search);

        let result = toolbox
            .get_tool("web_search")
            .unwrap()
            .call(
                &livvi_core::tool::ToolContext {
                    agent_context: &AgentContext::new("soul", None),
                    tool_call_id: "call-1",
                    state: &state,
                    memory_provider: None,
                },
                serde_json::json!({"query": "rust"}),
            )
            .await;

        assert!(
            matches!(result, livvi_core::tool::ToolCallOutput::Error(_)),
            "expected error when SearxNG is not configured"
        );
    }

    #[test]
    fn web_search_input_defaults_limit() {
        let input: WebSearchInput = serde_json::from_value(serde_json::json!({
            "query": "hello"
        }))
        .unwrap();
        assert_eq!(input.limit, 10);
    }
}
