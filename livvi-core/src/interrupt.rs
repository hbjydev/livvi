use std::fmt::{self, Debug, Display};

use livvi_store::{ConversationId, PersonId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

/// A transport-specific author identity, before canonical resolution to a [`Person`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalAuthor {
    pub transport_kind: String,
    pub transport_id: String,
    pub display_name: Option<String>,
    pub metadata: Value,
}

/// A transport-specific conversation identity, before canonical resolution to a
/// [`Conversation`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalConversation {
    pub transport_kind: String,
    pub transport_id: String,
    pub display_name: Option<String>,
    pub metadata: Value,
}

/// An event originating from an external transport (Discord, Bluesky, etc.).
///
/// The `person_id` and `conversation_id` fields are optional because the raw
/// transport event may need to be resolved against a [`LivviStore`] before it
/// reaches the agent loop. When present, they identify the canonical
/// [`Person`] and [`Conversation`] records in storage.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalEvent {
    pub transport_kind: String,
    pub event_type: String,
    pub content: Option<String>,
    pub author: ExternalAuthor,
    pub conversation: ExternalConversation,
    pub person_id: Option<PersonId>,
    pub conversation_id: Option<ConversationId>,
    pub metadata: Value,
    pub timestamp: Option<OffsetDateTime>,
}

impl Debug for ExternalEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExternalEvent")
            .field("transport", &self.transport_kind)
            .field("type", &self.event_type)
            .field("author", &self.person_id.as_ref().map(|p| short_id(&p.0)))
            .field(
                "conv",
                &self.conversation_id.as_ref().map(|c| short_id(&c.0)),
            )
            .finish()
    }
}

impl Display for ExternalEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.transport_kind, self.event_type)?;
        if let Some(person) = &self.person_id {
            write!(f, " person={}", short_id(&person.0))?;
        } else if let Some(name) = &self.author.display_name {
            write!(f, " author={}", name)?;
        } else {
            write!(f, " author={}", self.author.transport_id)?;
        }
        if let Some(conv) = &self.conversation_id {
            write!(f, " conv={}", short_id(&conv.0))?;
        } else if let Some(name) = &self.conversation.display_name {
            write!(f, " conv={}", name)?;
        } else {
            write!(f, " conv={}", self.conversation.transport_id)?;
        }
        Ok(())
    }
}

fn short_id(id: &str) -> &str {
    if id.is_ascii() && id.len() >= 8 {
        &id[..8]
    } else {
        id
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ResetEvent {
    pub transport_kind: String,
    pub author: ExternalAuthor,
    pub conversation: ExternalConversation,
    pub person_id: Option<PersonId>,
    pub conversation_id: Option<ConversationId>,
    pub metadata: Value,
    pub timestamp: Option<OffsetDateTime>,
}

impl Debug for ResetEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResetEvent")
            .field("transport", &self.transport_kind)
            .field("author", &self.person_id.as_ref().map(|p| short_id(&p.0)))
            .field(
                "conv",
                &self.conversation_id.as_ref().map(|c| short_id(&c.0)),
            )
            .finish()
    }
}

impl Display for ResetEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.reset", self.transport_kind)?;
        if let Some(person) = &self.person_id {
            write!(f, " person={}", short_id(&person.0))?;
        } else if let Some(name) = &self.author.display_name {
            write!(f, " author={}", name)?;
        } else {
            write!(f, " author={}", self.author.transport_id)?;
        }
        if let Some(conv) = &self.conversation_id {
            write!(f, " conv={}", short_id(&conv.0))?;
        } else if let Some(name) = &self.conversation.display_name {
            write!(f, " conv={}", name)?;
        } else {
            write!(f, " conv={}", self.conversation.transport_id)?;
        }
        Ok(())
    }
}

impl ResetEvent {
    pub fn new(
        transport_kind: impl Into<String>,
        author: ExternalAuthor,
        conversation: ExternalConversation,
    ) -> Self {
        Self {
            transport_kind: transport_kind.into(),
            author,
            conversation,
            person_id: None,
            conversation_id: None,
            metadata: Value::Null,
            timestamp: Some(OffsetDateTime::now_utc()),
        }
    }
}

/// A request to grant permission for a tool in a specific conversation.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct AllowToolEvent {
    pub transport_kind: String,
    pub author: ExternalAuthor,
    pub conversation: ExternalConversation,
    pub tool_name: String,
    pub person_id: Option<PersonId>,
    pub conversation_id: Option<ConversationId>,
    pub metadata: Value,
    pub timestamp: Option<OffsetDateTime>,
}

impl Debug for AllowToolEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AllowToolEvent")
            .field("transport", &self.transport_kind)
            .field("tool", &self.tool_name)
            .field("author", &self.person_id.as_ref().map(|p| short_id(&p.0)))
            .field(
                "conv",
                &self.conversation_id.as_ref().map(|c| short_id(&c.0)),
            )
            .finish()
    }
}

impl Display for AllowToolEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.allow_tool {}", self.transport_kind, self.tool_name)?;
        if let Some(person) = &self.person_id {
            write!(f, " person={}", short_id(&person.0))?;
        } else if let Some(name) = &self.author.display_name {
            write!(f, " author={}", name)?;
        } else {
            write!(f, " author={}", self.author.transport_id)?;
        }
        if let Some(conv) = &self.conversation_id {
            write!(f, " conv={}", short_id(&conv.0))?;
        } else if let Some(name) = &self.conversation.display_name {
            write!(f, " conv={}", name)?;
        } else {
            write!(f, " conv={}", self.conversation.transport_id)?;
        }
        Ok(())
    }
}

impl AllowToolEvent {
    pub fn new(
        transport_kind: impl Into<String>,
        author: ExternalAuthor,
        conversation: ExternalConversation,
        tool_name: impl Into<String>,
    ) -> Self {
        Self {
            transport_kind: transport_kind.into(),
            author,
            conversation,
            tool_name: tool_name.into(),
            person_id: None,
            conversation_id: None,
            metadata: Value::Null,
            timestamp: Some(OffsetDateTime::now_utc()),
        }
    }
}

impl ExternalEvent {
    /// Format the event as an XML message for the model's context.
    ///
    /// Standard fields (`source`, `author_id`, `person_id`, etc.) are emitted as
    /// XML attributes. Any extra metadata in `author.metadata` and
    /// `conversation.metadata` is flattened into attributes as well, prefixed with
    /// `<source>__author__` and `<source>__conversation__` so metadata from
    /// different transports cannot collide.
    pub fn to_xml_message(&self) -> String {
        let mut attrs: Vec<(String, String)> = vec![
            ("source".to_string(), xml_escape(&self.transport_kind)),
            ("event_type".to_string(), xml_escape(&self.event_type)),
            (
                "author_id".to_string(),
                xml_escape(&self.author.transport_id),
            ),
            (
                "author_name".to_string(),
                xml_escape(self.author.display_name.as_deref().unwrap_or("")),
            ),
            (
                "person_id".to_string(),
                xml_escape(
                    &self
                        .person_id
                        .as_ref()
                        .map(|id| id.to_string())
                        .unwrap_or_default(),
                ),
            ),
            (
                "conversation_id".to_string(),
                xml_escape(
                    &self
                        .conversation_id
                        .as_ref()
                        .map(|id| id.to_string())
                        .unwrap_or_default(),
                ),
            ),
            (
                "conversation_transport_id".to_string(),
                xml_escape(&self.conversation.transport_id),
            ),
        ];

        flatten_json(
            &self.author.metadata,
            &format!("{}__author", self.transport_kind),
            &mut attrs,
        );
        flatten_json(
            &self.conversation.metadata,
            &format!("{}__conversation", self.transport_kind),
            &mut attrs,
        );

        let attrs_str = attrs
            .into_iter()
            .map(|(k, v)| format!(r#"{}="{}""#, k, v))
            .collect::<Vec<_>>()
            .join(" ");

        format!(
            "<external_event {}>{}</external_event>",
            attrs_str,
            xml_escape(self.content.as_deref().unwrap_or(""))
        )
    }
}

fn flatten_json(value: &Value, prefix: &str, attrs: &mut Vec<(String, String)>) {
    if let Some(obj) = value.as_object() {
        for (key, val) in obj {
            let full_key = format!("{}__{}", prefix, key);
            match val {
                Value::String(s) => attrs.push((full_key, xml_escape(s))),
                Value::Number(n) => attrs.push((full_key, n.to_string())),
                Value::Bool(b) => attrs.push((full_key, b.to_string())),
                Value::Null => {}
                Value::Object(_) => flatten_json(val, &full_key, attrs),
                Value::Array(_) => {}
            }
        }
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[derive(Clone, PartialEq)]
pub enum Interrupt {
    ExternalEvent(ExternalEvent),
    Reset(ResetEvent),
    AllowTool(AllowToolEvent),
}

impl Debug for Interrupt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Interrupt::ExternalEvent(event) => Display::fmt(event, f),
            Interrupt::Reset(event) => Display::fmt(event, f),
            Interrupt::AllowTool(event) => Display::fmt(event, f),
        }
    }
}

impl Display for Interrupt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Interrupt::ExternalEvent(event) => Display::fmt(event, f),
            Interrupt::Reset(event) => Display::fmt(event, f),
            Interrupt::AllowTool(event) => Display::fmt(event, f),
        }
    }
}

impl Interrupt {
    /// Create an interrupt from a raw external event.
    pub fn external_event(event: ExternalEvent) -> Self {
        Interrupt::ExternalEvent(event)
    }

    /// Create an interrupt that wipes the in-memory context for a conversation.
    pub fn reset(event: ResetEvent) -> Self {
        Interrupt::Reset(event)
    }

    /// Create an interrupt that grants permission for a tool in a conversation.
    pub fn allow_tool(event: AllowToolEvent) -> Self {
        Interrupt::AllowTool(event)
    }

    /// Convenience constructor for simple text input in tests and examples.
    ///
    /// This creates an [`ExternalEvent`] with a synthetic "internal" transport
    /// identity and no resolved person or conversation.
    pub fn message(content: impl Into<String>) -> Self {
        let content = content.into();
        Interrupt::ExternalEvent(ExternalEvent {
            transport_kind: "internal".to_string(),
            event_type: "message".to_string(),
            content: Some(content),
            author: ExternalAuthor {
                transport_kind: "internal".to_string(),
                transport_id: "user".to_string(),
                display_name: None,
                metadata: Value::Null,
            },
            conversation: ExternalConversation {
                transport_kind: "internal".to_string(),
                transport_id: "default".to_string(),
                display_name: None,
                metadata: Value::Null,
            },
            person_id: None,
            conversation_id: None,
            metadata: Value::Null,
            timestamp: None,
        })
    }
}
