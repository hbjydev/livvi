use livvi_core::tool::{Input, State, tool};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::WebState;

fn default_fetch_max_length() -> usize {
    10000
}

fn default_fetch_format() -> String {
    "text".to_string()
}

/// Input to [`web_fetch`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebFetchInput {
    /// The URL to fetch.
    pub url: String,

    /// Maximum number of characters to return. `0` means unlimited.
    /// Defaults to 10,000.
    #[serde(default = "default_fetch_max_length")]
    pub max_length: usize,

    /// Format for the returned content: `"text"` (default, extracted plain
    /// text) or `"html"` (raw HTML).
    #[serde(default = "default_fetch_format")]
    pub format: String,
}

/// Output of [`web_fetch`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebFetchOutput {
    /// The requested URL.
    pub url: String,

    /// Page title, if one could be extracted.
    pub title: Option<String>,

    /// The fetched content, truncated to `max_length` if necessary.
    pub content: String,

    /// Content-Type returned by the server, without the charset suffix.
    pub content_type: Option<String>,

    /// Length of the returned content in characters.
    pub length: usize,
}

/// Fetch and read the content of a web page.
///
/// Returns the page title, content, and content type. HTML pages are converted
/// to readable plain text. Use this when you need details from a specific URL.
#[tool]
pub async fn web_fetch(
    Input(input): Input<WebFetchInput>,
    State(state): State<'_, WebState>,
) -> Result<WebFetchOutput, String> {
    let url = reqwest::Url::parse(&input.url).map_err(|e| format!("invalid URL: {e}"))?;

    let response = state
        .client
        .get(url.clone())
        .send()
        .await
        .map_err(|e| format!("fetch failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("fetch returned HTTP {status}"));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or(s).trim().to_lowercase());

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("failed to read response body: {e}"))?;

    let (title, content) = match input.format.as_str() {
        "html" => (None, String::from_utf8_lossy(&bytes).to_string()),
        _ => {
            let is_html = content_type
                .as_deref()
                .map(|ct| ct.starts_with("text/html") || ct.starts_with("application/xhtml"))
                .unwrap_or(false)
                || is_likely_html(&bytes);

            if is_html {
                extract_text_from_html(&bytes, &input.url).await?
            } else {
                (None, String::from_utf8_lossy(&bytes).to_string())
            }
        }
    };

    let content = truncate_text(content, input.max_length);
    let length = content.len();

    Ok(WebFetchOutput {
        url: input.url,
        title,
        content,
        content_type,
        length,
    })
}

fn is_likely_html(bytes: &bytes::Bytes) -> bool {
    if let Ok(text) = std::str::from_utf8(bytes) {
        let lower = text.to_ascii_lowercase();
        lower.starts_with("<!doctype html") || lower.contains("<html")
    } else {
        false
    }
}

async fn extract_text_from_html(
    bytes: &bytes::Bytes,
    url: &str,
) -> Result<(Option<String>, String), String> {
    let url = url.to_string();
    let bytes = bytes.clone();

    tokio::task::spawn_blocking(move || {
        let url = url::Url::parse(&url).map_err(|e| format!("invalid URL: {e}"))?;
        let mut cursor = std::io::Cursor::new(bytes);
        let product = readability::extractor::extract(&mut cursor, &url)
            .map_err(|e| format!("failed to extract readable content: {e}"))?;

        let title = if product.title.is_empty() {
            None
        } else {
            Some(product.title)
        };

        let text = html2text::from_read(product.content.as_bytes(), 120)
            .map_err(|e| format!("failed to render HTML to text: {e}"))?;

        let text = normalize_whitespace(&text);

        Ok((title, text))
    })
    .await
    .map_err(|e| format!("HTML extraction panicked: {e}"))?
}

fn normalize_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_blank = true;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_blank {
                result.push('\n');
                prev_blank = true;
            }
        } else {
            result.push_str(trimmed);
            result.push('\n');
            prev_blank = false;
        }
    }
    result.trim_end().to_string()
}

fn truncate_text(text: String, max_length: usize) -> String {
    if max_length == 0 || text.len() <= max_length {
        return text;
    }

    text.chars().take(max_length).collect::<String>() + "\n\n[content truncated]"
}

#[cfg(test)]
mod tests {
    use super::*;
    use livvi_core::context::Context as AgentContext;
    use livvi_core::tool::Toolbox;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SIMPLE_HTML: &str = r#"
        <!DOCTYPE html>
        <html>
        <head><title>Hello Page</title></head>
        <body>
            <nav>Home | About</nav>
            <main>
                <h1>Welcome</h1>
                <p>This is the main content.</p>
            </main>
            <footer>Footer text</footer>
        </body>
        </html>
    "#;

    #[tokio::test]
    async fn web_fetch_extracts_text_from_html() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/page"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(SIMPLE_HTML, "text/html"))
            .mount(&server)
            .await;

        let state = WebState::new(None);
        let mut toolbox = Toolbox::<WebState>::new();
        toolbox.add_tool(web_fetch);

        let result = toolbox
            .get_tool("web_fetch")
            .unwrap()
            .call(
                &livvi_core::tool::ToolContext {
                    agent_context: &AgentContext::new("soul", None),
                    tool_call_id: "call-1",
                    state: &state,
                    memory_provider: None,
                },
                serde_json::json!({"url": format!("{}/page", server.uri()), "max_length": 0}),
            )
            .await;

        let success = match result {
            livvi_core::tool::ToolCallOutput::Success(s) => s,
            other => panic!("expected success, got {other:?}"),
        };
        let output: WebFetchOutput = serde_json::from_str(&success).unwrap();
        assert_eq!(output.title, Some("Hello Page".to_string()));
        assert_eq!(output.content_type, Some("text/html".to_string()));
        assert!(
            output.content.contains("main content"),
            "content: {}",
            output.content
        );
        assert!(
            !output.content.contains("Footer text"),
            "nav/footer should be stripped: {}",
            output.content
        );
    }

    #[tokio::test]
    async fn web_fetch_returns_raw_html_when_requested() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/page"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(SIMPLE_HTML, "text/html"))
            .mount(&server)
            .await;

        let state = WebState::new(None);
        let mut toolbox = Toolbox::<WebState>::new();
        toolbox.add_tool(web_fetch);

        let result = toolbox
            .get_tool("web_fetch")
            .unwrap()
            .call(
                &livvi_core::tool::ToolContext {
                    agent_context: &AgentContext::new("soul", None),
                    tool_call_id: "call-1",
                    state: &state,
                    memory_provider: None,
                },
                serde_json::json!({
                    "url": format!("{}/page", server.uri()),
                    "format": "html",
                    "max_length": 0
                }),
            )
            .await;

        let success = match result {
            livvi_core::tool::ToolCallOutput::Success(s) => s,
            other => panic!("expected success, got {other:?}"),
        };
        let output: WebFetchOutput = serde_json::from_str(&success).unwrap();
        assert!(output.content.contains("Hello Page"));
        assert!(output.content.contains("<!DOCTYPE html>"));
    }

    #[tokio::test]
    async fn web_fetch_returns_plain_text() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/plain.txt"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain")
                    .set_body_string("hello world"),
            )
            .mount(&server)
            .await;

        let state = WebState::new(None);
        let mut toolbox = Toolbox::<WebState>::new();
        toolbox.add_tool(web_fetch);

        let result = toolbox
            .get_tool("web_fetch")
            .unwrap()
            .call(
                &livvi_core::tool::ToolContext {
                    agent_context: &AgentContext::new("soul", None),
                    tool_call_id: "call-1",
                    state: &state,
                    memory_provider: None,
                },
                serde_json::json!({"url": format!("{}/plain.txt", server.uri()), "max_length": 0}),
            )
            .await;

        let success = match result {
            livvi_core::tool::ToolCallOutput::Success(s) => s,
            other => panic!("expected success, got {other:?}"),
        };
        let output: WebFetchOutput = serde_json::from_str(&success).unwrap();
        assert_eq!(output.content, "hello world");
        assert_eq!(output.content_type, Some("text/plain".to_string()));
    }

    #[tokio::test]
    async fn web_fetch_errors_on_non_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let state = WebState::new(None);
        let mut toolbox = Toolbox::<WebState>::new();
        toolbox.add_tool(web_fetch);

        let result = toolbox
            .get_tool("web_fetch")
            .unwrap()
            .call(
                &livvi_core::tool::ToolContext {
                    agent_context: &AgentContext::new("soul", None),
                    tool_call_id: "call-1",
                    state: &state,
                    memory_provider: None,
                },
                serde_json::json!({"url": format!("{}/missing", server.uri())}),
            )
            .await;

        assert!(
            matches!(result, livvi_core::tool::ToolCallOutput::Error(_)),
            "expected error on 404"
        );
    }

    #[test]
    fn web_fetch_input_defaults() {
        let input: WebFetchInput = serde_json::from_value(serde_json::json!({
            "url": "https://example.com"
        }))
        .unwrap();
        assert_eq!(input.max_length, 10000);
        assert_eq!(input.format, "text");
    }

    #[test]
    fn truncate_text_respects_max_length() {
        let text = "abcdef".to_string();
        let truncated = truncate_text(text, 3);
        assert_eq!(truncated, "abc\n\n[content truncated]");
    }

    #[test]
    fn truncate_text_passes_through_when_unlimited() {
        let text = "abcdef".to_string();
        assert_eq!(truncate_text(text, 0), "abcdef");
    }

    #[test]
    fn is_likely_html_detects_doctype() {
        let bytes = bytes::Bytes::from_static(b"<!DOCTYPE html><html></html>");
        assert!(is_likely_html(&bytes));
    }

    #[test]
    fn is_likely_html_false_for_plain_text() {
        let bytes = bytes::Bytes::from_static(b"hello world");
        assert!(!is_likely_html(&bytes));
    }
}
