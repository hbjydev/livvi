use anyhow::Result;
use livvi_core::tool::{Input, State, tool};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::DiscordState;

/// Attachments larger than this are not downloaded.
const MAX_ATTACHMENT_BYTES: u64 = 1024 * 1024;

fn default_max_length() -> usize {
    10000
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscordReadAttachmentsInput {
    /// The Discord ID of the channel the message is in.
    pub channel_id: String,
    /// The Discord ID of the message whose attachments to read.
    pub message_id: String,
    /// Maximum number of characters to return per attachment. `0` means
    /// unlimited. Defaults to 10,000.
    #[serde(default = "default_max_length")]
    pub max_length: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadAttachment {
    /// The name of the uploaded file.
    pub filename: String,
    /// The file's media type, if Discord reported one.
    pub content_type: Option<String>,
    /// The size of the file in bytes.
    pub size: u32,
    /// The file's text content, if it could be read.
    pub content: Option<String>,
    /// Why the content is absent, if it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordReadAttachmentsOutput {
    /// Every attachment on the message, in upload order.
    pub attachments: Vec<ReadAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordReadAttachmentsError {
    pub message: String,
}

/// Read the attachments of a Discord message.
///
/// Downloads and returns the text content of each text-like attachment on the
/// message (plain text, markdown, code, JSON, XML, CSV, logs, and similar).
/// Non-text attachments (images, audio, video, archives) are listed with their
/// metadata but their content is not returned.
#[tool(allowed_by_default = true)]
pub async fn discord_read_attachments(
    Input(input): Input<DiscordReadAttachmentsInput>,
    State(state): State<'_, DiscordState>,
) -> Result<DiscordReadAttachmentsOutput, DiscordReadAttachmentsError> {
    let channel_id = input
        .channel_id
        .parse::<u64>()
        .map_err(|e| DiscordReadAttachmentsError {
            message: format!("Invalid channel_id: {e}"),
        })?;

    let message_id = input
        .message_id
        .parse::<u64>()
        .map_err(|e| DiscordReadAttachmentsError {
            message: format!("Invalid message_id: {e}"),
        })?;

    let message = state
        .get_message(channel_id, message_id)
        .await
        .map_err(|e| DiscordReadAttachmentsError {
            message: format!("Failed to fetch Discord message: {e}"),
        })?;

    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        let mut entry = ReadAttachment {
            filename: attachment.filename.clone(),
            content_type: attachment.content_type.clone(),
            size: attachment.size,
            content: None,
            reason: None,
        };

        if !is_text_like(attachment.content_type.as_deref(), &attachment.filename) {
            entry.reason = Some("not a text attachment".to_string());
        } else if u64::from(attachment.size) > MAX_ATTACHMENT_BYTES {
            entry.reason = Some(format!(
                "attachment too large ({} bytes, max {MAX_ATTACHMENT_BYTES})",
                attachment.size
            ));
        } else {
            match attachment.download().await {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    entry.content = Some(truncate_text(text.into_owned(), input.max_length));
                }
                Err(e) => {
                    entry.reason = Some(format!("failed to download: {e}"));
                }
            }
        }

        attachments.push(entry);
    }

    Ok(DiscordReadAttachmentsOutput { attachments })
}

/// Decide whether an attachment is text we can meaningfully read, based on its
/// reported content type, falling back to the filename extension when the
/// content type is missing or generic.
fn is_text_like(content_type: Option<&str>, filename: &str) -> bool {
    if let Some(ct) = content_type {
        let ct = ct.split(';').next().unwrap_or(ct).trim().to_lowercase();

        if ct.starts_with("text/") {
            return true;
        }

        if ct != "application/octet-stream" {
            return ct.starts_with("application/")
                && (ct.ends_with("/json")
                    || ct.ends_with("+json")
                    || ct.ends_with("/xml")
                    || ct.ends_with("+xml")
                    || matches!(
                        ct.as_str(),
                        "application/javascript"
                            | "application/x-javascript"
                            | "application/yaml"
                            | "application/x-yaml"
                            | "application/toml"
                            | "application/sql"
                            | "application/x-sh"
                            | "application/x-shellscript"
                            | "application/x-ndjson"
                            | "application/rtf"
                    ));
        }
    }

    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "txt"
            | "md"
            | "markdown"
            | "log"
            | "json"
            | "jsonl"
            | "ndjson"
            | "csv"
            | "tsv"
            | "xml"
            | "yaml"
            | "yml"
            | "toml"
            | "ini"
            | "cfg"
            | "conf"
            | "env"
            | "rs"
            | "py"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "css"
            | "html"
            | "htm"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "java"
            | "go"
            | "rb"
            | "sh"
            | "bash"
            | "zsh"
            | "sql"
            | "diff"
            | "patch"
            | "tex"
            | "svg"
            | "srt"
            | "vtt"
    )
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
    use livvi_core::tool::{ToolCallOutput, Toolbox};

    #[test]
    fn input_defaults_max_length() {
        let input: DiscordReadAttachmentsInput = serde_json::from_value(serde_json::json!({
            "channel_id": "1",
            "message_id": "2"
        }))
        .unwrap();
        assert_eq!(input.max_length, 10000);
    }

    #[test]
    fn text_like_by_content_type() {
        assert!(is_text_like(Some("text/plain"), "notes.bin"));
        assert!(is_text_like(Some("text/markdown; charset=utf-8"), "README"));
        assert!(is_text_like(Some("application/json"), "data.bin"));
        assert!(is_text_like(Some("application/ld+json"), "data.bin"));
        assert!(is_text_like(Some("application/xml"), "data.bin"));
        assert!(is_text_like(Some("application/atom+xml"), "data.bin"));
        assert!(is_text_like(Some("application/javascript"), "app.bin"));
        assert!(is_text_like(Some("application/x-yaml"), "config.bin"));
        assert!(!is_text_like(Some("image/png"), "photo.txt"));
        assert!(!is_text_like(Some("application/zip"), "archive.txt"));
        assert!(!is_text_like(Some("audio/mpeg"), "song.txt"));
    }

    #[test]
    fn text_like_falls_back_to_extension() {
        // Generic or missing content types defer to the filename.
        assert!(is_text_like(Some("application/octet-stream"), "main.rs"));
        assert!(is_text_like(None, "notes.md"));
        assert!(is_text_like(None, "data.JSON"));
        assert!(!is_text_like(Some("application/octet-stream"), "photo.png"));
        assert!(!is_text_like(None, "archive.zip"));
        assert!(!is_text_like(None, "README"));
    }

    #[test]
    fn truncate_text_respects_max_length() {
        let truncated = truncate_text("abcdef".to_string(), 3);
        assert_eq!(truncated, "abc\n\n[content truncated]");
    }

    #[test]
    fn truncate_text_passes_through_when_unlimited_or_short() {
        assert_eq!(truncate_text("abcdef".to_string(), 0), "abcdef");
        assert_eq!(truncate_text("abc".to_string(), 3), "abc");
    }

    #[tokio::test]
    async fn invalid_ids_error_before_any_network_call() {
        let state = DiscordState::new("not-a-real-token");
        let mut toolbox = Toolbox::<DiscordState>::new();
        toolbox.add_tool(discord_read_attachments);

        let result = toolbox
            .get_tool("discord_read_attachments")
            .unwrap()
            .call(
                &livvi_core::tool::ToolContext {
                    agent_context: &AgentContext::new("soul", None),
                    tool_call_id: "call-1",
                    state: &state,
                    memory_provider: None,
                },
                serde_json::json!({"channel_id": "abc", "message_id": "2"}),
            )
            .await;

        assert!(
            matches!(result, ToolCallOutput::Error(_)),
            "expected error for invalid channel_id, got {result:?}"
        );
    }
}
