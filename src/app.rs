use ::dirs::home_dir;
use anyhow::{Context, Result};
#[cfg(not(target_os = "linux"))]
use arboard::Clipboard;

use std::fs;

use ratatui::{
    style::{Color, Style},
    widgets::Block,
};
use tui_textarea::TextArea;

use crate::{
    ai::MODELS,
    chats::ChatList,
    snippets::{find_fenced_code_snippets, SnippetItem},
    storage::{
        create_db_conversation, delete_conversation, insert_message, list_all_conversations,
        list_all_messages,
    },
};
use crate::{models::ModelList, snippets::SnippetList};

#[derive(Debug, Clone)]
pub enum Message {
    User(String),
    Assistant(String),
    Error(String),
}

impl From<String> for Message {
    fn from(message: String) -> Self {
        Message::User(message)
    }
}

impl From<&str> for Message {
    fn from(message: &str) -> Self {
        Message::User(message.to_string())
    }
}

impl AsRef<str> for Message {
    fn as_ref(&self) -> &str {
        match self {
            Message::User(message) => message.as_str(),
            Message::Assistant(message) => message.as_str(),
            Message::Error(message) => message.as_str(),
        }
    }
}
/// Application result type.
pub type AppResult<T> = Result<T>;

#[derive(Debug, Clone)]
pub enum AppMode {
    Normal,
    Editing,
    ModelSelection,
    SnippetSelection,
    ShowHistory,
    Help,
}

/// App holds the state of the application
pub struct App<'a> {
    /// Input text area
    pub input_textarea: TextArea<'a>,
    /// Position of cursor in the editor area.
    pub app_mode: AppMode,
    /// Conversation ID for chat database.
    pub conversation_id: Option<i64>,
    /// System prompt
    pub system_prompt: &'a str,
    /// Has unprocessed messages
    pub has_unprocessed_messages: bool,
    /// History of recorded messages
    pub messages: Vec<Message>,
    /// Vertical scroll
    pub vertical_scroll: usize,
    /// Is the application running?
    pub running: bool,
    /// System clipboard.
    /// Not enabled on Linux because of an issue with the `arboard` crate,
    /// see <https://github.com/1Password/arboard/issues/153>
    #[cfg(not(target_os = "linux"))]
    pub clipboard: Clipboard,
    /// List of models
    pub model_list: ModelList,
    /// Selected model name
    pub selected_model_name: String,
    /// Discovered snippets
    pub snippet_list: SnippetList,
    /// List of chats
    pub chat_list: ChatList,
}

fn styled_input_textarea() -> TextArea<'static> {
    let mut input_textarea = TextArea::default();
    input_textarea.set_block(Block::bordered().title("Input"));
    input_textarea.set_style(Style::default().fg(Color::Yellow));
    input_textarea
}

impl Default for App<'_> {
    fn default() -> Self {
        Self {
            input_textarea: styled_input_textarea(),
            app_mode: AppMode::Normal,
            system_prompt: "You are a helpful, friendly assistant.",
            conversation_id: None,
            has_unprocessed_messages: false,
            messages: Vec::new(),
            // user_messages: Vec::new(),
            // assistant_messages: Vec::new(),
            vertical_scroll: 0,
            running: true,
            #[cfg(not(target_os = "linux"))]
            clipboard: Clipboard::new().unwrap(),
            model_list: ModelList::from_iter(MODELS.map(|(provider, model)| {
                if model == "gpt-4o-mini" {
                    (provider, model, true)
                } else {
                    (provider, model, false)
                }
            })),
            selected_model_name: "gpt-4o-mini".to_string(),
            snippet_list: SnippetList::from_iter([].iter().map(|&snippet| (snippet, false))),
            chat_list: ChatList::from_iter([].iter().map(|&chat| (chat, "".to_string(), false))),
        }
    }
}

impl<'a> App<'a> {
    pub fn new(system_prompt: &'a str) -> Self {
        Self {
            system_prompt,
            ..Default::default()
        }
    }

    /// Handles the tick event of the terminal.
    pub fn tick(&self) {}

    pub fn set_app_mode(&mut self, new_app_mode: AppMode) {
        self.app_mode = new_app_mode;
    }

    pub fn create_conversation(&mut self) -> AppResult<i64> {
        let conv_id = create_db_conversation(self.system_prompt)
            .context("Failed to create conversation in db")?;
        self.conversation_id = Some(conv_id);
        Ok(conv_id)
    }

    fn write_chat_log(&self) -> AppResult<()> {
        let mut chat_log = String::new();
        for message in self.messages.iter() {
            match message {
                Message::User(message) => {
                    chat_log.push_str(&format!("User: {}\n", message));
                }
                Message::Assistant(message) => {
                    chat_log.push_str(&format!("Assistant: {}\n", message));
                }
                Message::Error(message) => {
                    chat_log.push_str(&format!("Error: {}\n", message));
                }
            }
        }
        let mut path = home_dir().context("Cannot find home directory")?;
        path.push(".cache/ait");
        fs::create_dir_all(&path).context("Could not create cache directory")?;
        path.push("latest-chat.log");
        fs::write(&path, chat_log).context("Unable to write chat log")?;
        Ok(())
    }

    pub fn increment_vertical_scroll(&mut self) -> AppResult<()> {
        let (width, _) = crossterm::terminal::size().context("Unable to get terminal size")?;
        let max_scroll = self
            .messages
            .iter()
            .map(|m| textwrap::wrap(m.as_ref(), width as usize - 5).join("\n"))
            .collect::<Vec<String>>()
            .join("\n")
            .split('\n')
            .collect::<Vec<&str>>()
            .len()
            + 3 * (self.messages.len())
            - 1;
        if self.vertical_scroll < max_scroll {
            self.vertical_scroll += 1;
        }
        Ok(())
    }

    pub fn decrement_vertical_scroll(&mut self) {
        self.vertical_scroll = self.vertical_scroll.saturating_sub(1);
    }

    pub fn submit_message(&mut self) -> AppResult<()> {
        let text = self.input_textarea.lines().join("\n");
        if text.is_empty() {
            return Ok(());
        }
        let n_user_messages = self
            .messages
            .iter()
            .filter(|m| matches!(m, Message::User(_)))
            .count();
        let n_assistant_messages = self
            .messages
            .iter()
            .filter(|m| matches!(m, Message::Assistant(_)))
            .count();
        if n_user_messages != n_assistant_messages {
            return Ok(());
        }

        self.has_unprocessed_messages = true;
        self.input_textarea = styled_input_textarea();
        self.set_app_mode(AppMode::Normal);
        self.write_chat_log()
            .context("Unable to write submitted message to chat log")?;
        let message = Message::User(text);
        if let Some(id) = self.conversation_id {
            insert_message(id, &message)?;
        } else {
            let id = self.create_conversation()?;
            insert_message(id, &message)?;
        }
        self.messages.push(message);
        Ok(())
    }

    pub fn set_models(&mut self, models: Vec<(String, String)>) {
        self.model_list = ModelList::from_iter(models.into_iter().map(|(provider, model)| {
            if model == "gpt-4o-mini" {
                (provider, model, true)
            } else {
                (provider, model, false)
            }
        }));
    }

    pub async fn receive_message(&mut self, message: Message) -> AppResult<()> {
        let message_content = message.as_ref();
        let discovered_snippets =
            find_fenced_code_snippets(message_content.split('\n').map(|s| s.to_string()).collect());
        let snippet_items: Vec<SnippetItem> = discovered_snippets
            .iter()
            .map(|snippet| snippet.to_string().into())
            .collect();
        self.snippet_list.items.extend(snippet_items);
        self.has_unprocessed_messages = false;
        self.write_chat_log()
            .context("Unable to write received message to chat log")?;
        if let Some(id) = self.conversation_id {
            insert_message(id, &message)?;
        } else {
            let id = self.create_conversation()?;
            insert_message(id, &message)?;
        }
        self.messages.push(message);
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    pub fn paste_to_input_textarea(&mut self) {
        if let Ok(clipboard_content) = self.clipboard.get_text() {
            self.input_textarea.insert_str(clipboard_content);
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn yank_latest_assistant_message(&mut self) {
        let assistant_messages = self.messages.iter().filter_map(|m| match m {
            Message::Assistant(message) => Some(message),
            _ => None,
        });
        if let Some(message) = assistant_messages.last() {
            self.clipboard.set_text(message).unwrap();
        }
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn select_no_model(&mut self) {
        self.model_list.state.select(None);
    }

    pub fn select_next_model(&mut self) {
        self.model_list.state.select_next();
    }
    pub fn select_previous_model(&mut self) {
        self.model_list.state.select_previous();
    }

    pub fn select_first_model(&mut self) {
        self.model_list.state.select_first();
    }

    pub fn select_last_model(&mut self) {
        self.model_list.state.select_last();
    }

    /// Changes the status of the selected list item
    pub fn set_model(&mut self) {
        if let Some(i) = self.model_list.state.selected() {
            for item in self.model_list.items.iter_mut() {
                item.selected = false;
            }
            self.model_list.items[i].selected = true;
            self.selected_model_name = self.model_list.items[i].name.to_string();
        }
    }

    pub fn select_no_snippet(&mut self) {
        self.snippet_list.state.select(None);
    }

    pub fn select_next_snippet(&mut self) {
        self.snippet_list.state.select_next();
    }
    pub fn select_previous_snippet(&mut self) {
        self.snippet_list.state.select_previous();
    }

    pub fn select_first_snippet(&mut self) {
        self.snippet_list.state.select_first();
    }

    pub fn select_last_snippet(&mut self) {
        self.snippet_list.state.select_last();
    }

    pub fn get_snippet_text(&self) -> Option<&String> {
        self.snippet_list
            .state
            .selected()
            .map(|i| &self.snippet_list.items[i].text)
    }

    #[cfg(not(target_os = "linux"))]
    /// Copy the selected snippet to the clipboard.
    pub fn copy_snippet(&mut self) -> AppResult<()> {
        if let Some(i) = self.snippet_list.state.selected() {
            for item in self.snippet_list.items.iter_mut() {
                item.selected = false;
            }
            self.snippet_list.items[i].selected = true;
            self.clipboard
                .set_text(&self.snippet_list.items[i].text)
                .context("Unable to copy snippet to clipboard")?;
        }
        Ok(())
    }

    pub fn select_no_chat(&mut self) {
        self.chat_list.state.select(None);
    }

    pub fn select_next_chat(&mut self) {
        self.chat_list.state.select_next();
    }
    pub fn select_previous_chat(&mut self) {
        self.chat_list.state.select_previous();
    }

    pub fn select_first_chat(&mut self) {
        self.chat_list.state.select_first();
    }

    pub fn select_last_chat(&mut self) {
        self.chat_list.state.select_last();
    }

    pub fn set_chat_list(&mut self) -> AppResult<()> {
        let chats = list_all_conversations()?;
        let chats = chats
            .into_iter()
            .map(|(id, started_at)| (id, started_at, false))
            .collect::<Vec<(i64, String, bool)>>();
        self.chat_list = ChatList::from_iter(chats);
        Ok(())
    }

    pub fn delete_chat(&mut self) -> AppResult<()> {
        if let Some(i) = self.chat_list.state.selected() {
            let chat_id = self.chat_list.items[i].chat_id;
            delete_conversation(chat_id)?;
            self.chat_list.items.remove(i);
            self.messages.clear();
            self.messages = list_all_messages(chat_id)?;
            self.conversation_id = None;
        }
        Ok(())
    }

    pub fn get_selected_chat_id(&self) -> Option<&i64> {
        if self.chat_list.items.is_empty() {
            return None;
        }
        self.chat_list
            .state
            .selected()
            .map(|i| &self.chat_list.items[i].chat_id)
    }

    pub fn set_chat(&mut self) -> AppResult<()> {
        if let Some(i) = self.chat_list.state.selected() {
            for item in self.chat_list.items.iter_mut() {
                item.selected = false;
            }
            self.chat_list.items[i].selected = true;
            self.conversation_id = Some(self.chat_list.items[i].chat_id);
            self.messages.clear();
            self.messages = list_all_messages(self.chat_list.items[i].chat_id)?;
            self.snippet_list.clear();
            for message in self.messages.iter() {
                let message_content = message.as_ref();
                let discovered_snippets = find_fenced_code_snippets(
                    message_content.split('\n').map(|s| s.to_string()).collect(),
                );
                let snippet_items: Vec<SnippetItem> = discovered_snippets
                    .iter()
                    .map(|snippet| snippet.to_string().into())
                    .collect();
                self.snippet_list.items.extend(snippet_items);
            }
            self.vertical_scroll = 0;
        }
        Ok(())
    }
}
