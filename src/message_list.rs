use std::fmt::Display;

use genai::chat::ContentPart;
use ratatui::{
    text::{Line, Span},
    widgets::{ListItem, ListState},
};

use crate::app::Message;

pub struct MessageList {
    pub items: Vec<MessageItem>,
    pub state: ListState,
}

impl MessageList {
    pub fn empty() -> Self {
        MessageList {
            items: Vec::new(),
            state: ListState::default(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Role {
    User,
    Assistant,
}

impl Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::User => f.write_str("User")?,
            Role::Assistant => f.write_str("Assistant")?,
        };
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct MessageItem {
    pub role: Role,
    pub content: String,
    pub selected: bool,
}

impl MessageItem {
    pub fn new(content: &str, selected: bool, role: Role) -> Self {
        Self {
            content: content.to_string(),
            selected,
            role,
        }
    }
}

impl From<&MessageItem> for ListItem<'_> {
    fn from(value: &MessageItem) -> Self {
        let line = Line::from(Span::raw(format!("{}: {}", value.role, value.content)));
        ListItem::new(line)
    }
}

impl From<&Message> for MessageItem {
    fn from(value: &Message) -> Self {
        match value {
            Message::User(_) => Self::new(value.to_string().as_str(), false, Role::User),
            Message::Assistant(text, _, _, _) => Self::new(text, false, Role::Assistant),
        }
    }
}

impl From<MessageItem> for Message {
    fn from(value: MessageItem) -> Self {
        match value.role {
            Role::User => Message::User(vec![ContentPart::from_text(value.content)]),
            Role::Assistant => Message::Assistant(value.content, None, None, None),
        }
    }
}

impl From<&MessageItem> for Message {
    fn from(value: &MessageItem) -> Self {
        match value.role {
            Role::User => Message::User(vec![ContentPart::from_text(value.content.to_string())]),
            Role::Assistant => Message::Assistant(value.content.to_string(), None, None, None),
        }
    }
}
