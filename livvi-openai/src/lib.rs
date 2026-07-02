pub mod chat_completions;
pub mod common;
pub mod responses;

pub use chat_completions::OpenAIChatCompletionsProvider;
pub use responses::OpenAIResponsesProvider;

/// Backwards-compatible alias for [`OpenAIResponsesProvider`].
pub type OpenAIProvider = OpenAIResponsesProvider;
