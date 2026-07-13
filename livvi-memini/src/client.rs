use anyhow::{Context as _, Result, anyhow};
use livvi_core::memory::{
    Briefing, BriefingRequest, ListRequest, Memory, RecallRequest, RememberRequest, ScoredMemory,
    UpdateRequest,
};
use reqwest::StatusCode;
use serde::Deserialize;
use std::fmt::Display;

#[derive(Debug, Clone)]
pub struct MeminiClient {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl MeminiClient {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
        }
    }

    /// Build a request with the required Memini headers.
    fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        namespace: &str,
        home_namespace: Option<&str>,
    ) -> reqwest::RequestBuilder {
        let mut builder = self
            .client
            .request(method, format!("{}{path}", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("X-Memini-Namespace", namespace);

        if let Some(home) = home_namespace {
            builder = builder.header("X-Memini-Home", home);
        }

        builder
    }

    pub async fn remember(
        &self,
        namespace: &str,
        home_namespace: Option<&str>,
        request: RememberRequest,
    ) -> Result<Memory> {
        let response = self
            .request(
                reqwest::Method::POST,
                "/v1/memories",
                namespace,
                home_namespace,
            )
            .json(&request)
            .send()
            .await
            .map_err(|e| anyhow!("failed to send remember request: {e}"))?;

        handle_response(response).await
    }

    pub async fn recall(
        &self,
        namespace: &str,
        home_namespace: Option<&str>,
        request: RecallRequest,
    ) -> Result<Vec<ScoredMemory>> {
        #[derive(Deserialize)]
        struct SearchResponse {
            results: Vec<ScoredMemory>,
        }

        let response = self
            .request(
                reqwest::Method::POST,
                "/v1/search",
                namespace,
                home_namespace,
            )
            .json(&request)
            .send()
            .await
            .map_err(|e| anyhow!("failed to send recall request: {e}"))?;

        let body: SearchResponse = handle_response(response).await?;
        Ok(body.results)
    }

    pub async fn briefing(
        &self,
        namespace: &str,
        home_namespace: Option<&str>,
        request: BriefingRequest,
    ) -> Result<Briefing> {
        let query = briefing_query_pairs(&request);
        let response = self
            .request(
                reqwest::Method::GET,
                "/v1/namespaces/briefing",
                namespace,
                home_namespace,
            )
            .query(&query)
            .send()
            .await
            .map_err(|e| anyhow!("failed to send briefing request: {e}"))?;

        handle_response(response).await
    }

    pub async fn get(
        &self,
        namespace: &str,
        home_namespace: Option<&str>,
        id: &str,
    ) -> Result<Option<Memory>> {
        let response = self
            .request(
                reqwest::Method::GET,
                &format!("/v1/memories/{id}"),
                namespace,
                home_namespace,
            )
            .send()
            .await
            .map_err(|e| anyhow!("failed to send get request: {e}"))?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        handle_response(response).await.map(Some)
    }

    pub async fn list(
        &self,
        namespace: &str,
        home_namespace: Option<&str>,
        request: ListRequest,
    ) -> Result<Vec<Memory>> {
        #[derive(Deserialize)]
        struct ListResponse {
            memories: Vec<Memory>,
        }

        let query = list_query_pairs(&request);
        let response = self
            .request(
                reqwest::Method::GET,
                "/v1/memories",
                namespace,
                home_namespace,
            )
            .query(&query)
            .send()
            .await
            .map_err(|e| anyhow!("failed to send list request: {e}"))?;

        let body: ListResponse = handle_response(response).await?;
        Ok(body.memories)
    }

    pub async fn forget(
        &self,
        namespace: &str,
        home_namespace: Option<&str>,
        id: &str,
    ) -> Result<()> {
        let response = self
            .request(
                reqwest::Method::DELETE,
                &format!("/v1/memories/{id}"),
                namespace,
                home_namespace,
            )
            .send()
            .await
            .map_err(|e| anyhow!("failed to send forget request: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow!("forget failed ({status}): {text}"));
        }

        Ok(())
    }

    pub async fn update(
        &self,
        namespace: &str,
        home_namespace: Option<&str>,
        request: UpdateRequest,
    ) -> Result<Memory> {
        self.remember(namespace, home_namespace, request.into())
            .await
    }
}

async fn handle_response<T: for<'de> Deserialize<'de>>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(anyhow!("Memini request failed ({status}): {text}"));
    }

    response
        .json::<T>()
        .await
        .with_context(|| "failed to parse Memini response")
}

fn opt(pairs: &mut Vec<(String, String)>, key: &str, value: Option<impl Display>) {
    if let Some(value) = value {
        pairs.push((key.to_string(), value.to_string()));
    }
}

fn opt_vec(pairs: &mut Vec<(String, String)>, key: &str, value: Option<&Vec<impl Display>>) {
    if let Some(value) = value.filter(|v| !v.is_empty()) {
        pairs.push((
            key.to_string(),
            value
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(","),
        ));
    }
}

fn metadata_pairs(
    pairs: &mut Vec<(String, String)>,
    prefix: &str,
    meta: &serde_json::Map<String, serde_json::Value>,
) {
    for (key, value) in meta {
        if let Some(s) = value.as_str() {
            pairs.push((prefix.to_string(), format!("{key}={s}")));
        }
    }
}

fn briefing_query_pairs(request: &BriefingRequest) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    opt(&mut pairs, "per_section", request.per_section);
    opt(&mut pairs, "per_section_pinned", request.per_section_pinned);
    opt(&mut pairs, "per_section_facts", request.per_section_facts);
    opt(
        &mut pairs,
        "per_section_procedures",
        request.per_section_procedures,
    );
    opt(&mut pairs, "per_section_recent", request.per_section_recent);
    opt(&mut pairs, "scope", request.scope);
    if let Some(namespaces) = request.namespaces.as_ref().filter(|v| !v.is_empty()) {
        pairs.push((
            "namespaces".to_string(),
            namespaces
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(","),
        ));
    }
    pairs
}

fn list_query_pairs(request: &ListRequest) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    opt_vec(&mut pairs, "tier", request.tiers.as_ref());
    opt_vec(&mut pairs, "level", request.levels.as_ref());
    if let Some(tags) = request.tags.as_ref().filter(|v| !v.is_empty()) {
        pairs.push(("tag".to_string(), tags.join(",")));
    }
    if let Some(meta) = request.metadata.as_ref().filter(|m| !m.is_empty()) {
        metadata_pairs(&mut pairs, "meta", meta);
    }
    opt(&mut pairs, "include_expired", request.include_expired);
    opt(&mut pairs, "include_superseded", request.include_superseded);
    opt(&mut pairs, "limit", request.limit);
    opt(&mut pairs, "sort", request.sort.as_ref());
    opt(&mut pairs, "order", request.order.as_ref());
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use livvi_core::memory::{Level, Scope, Tier};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[tokio::test]
    async fn remember_parses_created_memory() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "id": "mem-1",
            "namespace": "livvi/conversations/conv-1",
            "content": "hello world",
            "tier": "episodic",
            "tags": ["livvi_turn"],
            "metadata": {},
            "importance": 0.5,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        });

        Mock::given(method("POST"))
            .and(path("/v1/memories"))
            .respond_with(ResponseTemplate::new(201).set_body_json(body))
            .mount(&server)
            .await;

        let client = MeminiClient::new(server.uri(), "key");
        let request = RememberRequest {
            content: "hello world".to_string(),
            tier: Tier::Episodic,
            summary: None,
            tags: vec!["livvi_turn".to_string()],
            metadata: serde_json::Map::new(),
            importance: None,
            level: None,
            ttl_seconds: None,
            id: None,
            valid_from: None,
            valid_to: None,
            confidence: None,
            visibility: None,
        };

        let memory = client
            .remember("livvi/conversations/conv-1", None, request)
            .await
            .unwrap();
        assert_eq!(memory.id, "mem-1");
        assert_eq!(memory.tier, Tier::Episodic);
    }

    #[tokio::test]
    async fn recall_parses_scored_results() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "results": [
                {
                    "memory": {
                        "id": "mem-1",
                        "namespace": "livvi/conversations/conv-1",
                        "content": "a fact",
                        "tier": "semantic",
                        "tags": [],
                        "metadata": {},
                        "importance": 0.8,
                        "created_at": "2026-01-01T00:00:00Z",
                        "updated_at": "2026-01-01T00:00:00Z"
                    },
                    "score": 0.95,
                    "from": "livvi"
                }
            ]
        });

        Mock::given(method("POST"))
            .and(path("/v1/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let client = MeminiClient::new(server.uri(), "key");
        let request = RecallRequest {
            query: "fact".to_string(),
            tiers: None,
            levels: None,
            tags: None,
            metadata: None,
            exclude_metadata: None,
            include_fresh_turns: None,
            query_rewrite: None,
            limit: None,
            include_expired: None,
            include_superseded: None,
            scope: Some(Scope::Full),
            namespaces: None,
            as_of: None,
            min_score: None,
        };

        let results = client
            .recall("livvi/conversations/conv-1", None, request)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].memory.id, "mem-1");
        assert_eq!(results[0].score, 0.95);
    }

    #[test]
    fn list_query_pairs_joins_enums() {
        let request = ListRequest {
            tiers: Some(vec![Tier::Episodic, Tier::Semantic]),
            levels: Some(vec![Level::Explicit]),
            tags: Some(vec!["tag1".to_string(), "tag2".to_string()]),
            metadata: None,
            include_expired: Some(true),
            include_superseded: None,
            limit: Some(10),
            sort: Some("created_at".to_string()),
            order: Some("desc".to_string()),
        };

        let pairs = list_query_pairs(&request);
        let map: std::collections::HashMap<&str, &str> = pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(map.get("tier"), Some(&"episodic,semantic"));
        assert_eq!(map.get("tag"), Some(&"tag1,tag2"));
        assert_eq!(map.get("limit"), Some(&"10"));
    }
}
