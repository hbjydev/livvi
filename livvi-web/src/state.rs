use reqwest::Client;

/// Application state shared with the web tools.
///
/// Holds the HTTP client and the configured SearxNG instance URL. Create one
/// with [`WebState::new`] and register it as `AsRef<WebState>` in the daemon's
/// app state.
#[derive(Debug, Clone)]
pub struct WebState {
    /// Base URL of the SearxNG instance, e.g. `http://localhost:8080`.
    ///
    /// If `None`, `web_search` will report that it is not configured.
    pub searxng_url: Option<String>,

    /// Shared HTTP client used for both search and fetch requests.
    pub client: Client,
}

impl WebState {
    /// Build a new web state with the default HTTP client.
    ///
    /// Pass `Some(url)` to enable `web_search`, or `None` to disable it while
    /// keeping `web_fetch` available.
    pub fn new(searxng_url: Option<String>) -> Self {
        Self {
            searxng_url,
            client: Client::new(),
        }
    }

    /// Build a new web state with a specific HTTP client.
    pub fn with_client(searxng_url: Option<String>, client: Client) -> Self {
        Self {
            searxng_url,
            client,
        }
    }
}

impl AsRef<WebState> for WebState {
    fn as_ref(&self) -> &WebState {
        self
    }
}
