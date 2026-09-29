use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::mcp_app::AppCall;

const TITLE_MAX_CHARS: usize = 40;
pub(crate) const DEFAULT_TITLE: &str = "New chat";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Set when the assistant turn failed; the partial content is kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// A tool call whose result is shown by an MCP App view below the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<AppCall>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            error: None,
            app: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            error: None,
            app: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    /// Unix epoch milliseconds.
    pub created_at: u64,
    /// Unix epoch milliseconds.
    pub updated_at: u64,
    pub messages: Vec<Message>,
}

impl Conversation {
    pub fn new() -> Self {
        let now = now_millis();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            title: DEFAULT_TITLE.to_owned(),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
        }
    }

    pub(crate) fn touch(&mut self) {
        self.updated_at = now_millis();
    }

    /// Derives the title from the first user message if it has not been set yet.
    pub(crate) fn ensure_title(&mut self) {
        if self.title != DEFAULT_TITLE {
            return;
        }
        if let Some(first) = self.messages.iter().find(|m| m.role == Role::User) {
            let line = first.content.lines().next().unwrap_or_default().trim();
            let mut title: String = line.chars().take(TITLE_MAX_CHARS).collect();
            if line.chars().count() > TITLE_MAX_CHARS {
                title.push('…');
            }
            if !title.is_empty() {
                self.title = title;
            }
        }
    }
}

impl Default for Conversation {
    fn default() -> Self {
        Self::new()
    }
}

/// Provider-independent request for one assistant turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatRequest {
    pub model: String,
    pub system_prompt: String,
    pub max_tokens: u32,
    pub messages: Vec<Message>,
}

pub(crate) fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_is_first_user_line_truncated() {
        let mut c = Conversation::new();
        c.messages
            .push(Message::user(format!("{}\nsecond line", "あ".repeat(50))));
        c.ensure_title();
        assert_eq!(c.title, format!("{}…", "あ".repeat(40)));
    }

    #[test]
    fn title_is_kept_once_set() {
        let mut c = Conversation::new();
        c.messages.push(Message::user("first"));
        c.ensure_title();
        c.messages[0].content = "changed".into();
        c.ensure_title();
        assert_eq!(c.title, "first");
    }

    #[test]
    fn message_error_is_omitted_when_none() {
        let json = serde_json::to_string(&Message::user("hi")).unwrap();
        assert_eq!(json, r#"{"role":"user","content":"hi"}"#);
    }
}
