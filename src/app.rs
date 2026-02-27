use anyhow::{Context, Result};
#[cfg(not(target_os = "linux"))]
use arboard::Clipboard;
use syntect::highlighting::Theme;
use syntect::parsing::SyntaxSet;

use std::fs;
use std::{borrow::Cow, fs::read_to_string, io};

use ratatui::{
    buffer::Buffer,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders},
};
use ratatui_explorer::{File, FileExplorer};
use tui_textarea::TextArea;

use crate::{
    ai::MODELS,
    chats::ChatList,
    snippets::{find_fenced_code_snippets, load_theme, SnippetItem},
    storage::{
        create_db_conversation, delete_conversation, delete_message, get_cache_dir, insert_message,
        list_all_conversations, list_all_messages,
    },
    ui::style_message,
};
use crate::{models::ModelList, snippets::SnippetList};

pub fn get_file_content(file: &File) -> io::Result<Cow<'_, str>> {
    // If the path is a file, read its content.
    if file.is_file() {
        read_to_string(file.path()).map(Into::into)
    } else if file.is_dir() {
        Ok("".into())
    } else {
        Ok("<not a regular file>".into())
    }
}

fn get_theme() -> ratatui_explorer::Theme {
    ratatui_explorer::Theme::default()
        .with_block(Block::default().borders(Borders::ALL))
        .with_dir_style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .with_highlight_dir_style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
                .bg(Color::DarkGray),
        )
        .with_scroll_padding(1)
}

#[derive(Debug, Clone, Default)]
pub struct Selection {
    pub start: Option<(u16, u16)>, // (column, row)
    pub end: Option<(u16, u16)>,
}

impl Selection {
    pub fn get_selected_text(&self, buffer: &Buffer) -> Option<String> {
        // Need both start and end points to make a selection
        let (start, end) = match (self.start, self.end) {
            (Some(start), Some(end)) => (start, end),
            _ => return None,
        };

        // Calculate bounds (handles selection in any direction)
        let start_row = start.1.min(end.1);
        let end_row = start.1.max(end.1);
        let start_col = start.0.min(end.0);
        let end_col = start.0.max(end.0);

        let mut selected_text = String::new();

        for row in start_row..=end_row {
            // Add newline between rows, but not before first row
            if row > start_row {
                selected_text.push('\n');
            }

            for col in start_col..=end_col {
                let cell = buffer.cell((col, row));
                if let Some(cell) = cell {
                    selected_text.push_str(cell.symbol());
                }
            }
        }

        Some(selected_text)
    }

    pub fn iter_selected_cells(&self) -> Option<impl Iterator<Item = (u16, u16)>> {
        let (start, end) = match (self.start, self.end) {
            (Some(start), Some(end)) => (start, end),
            _ => return None,
        };

        let start_row = start.1.min(end.1);
        let end_row = start.1.max(end.1);
        let start_col = start.0.min(end.0);
        let end_col = start.0.max(end.0);

        Some(
            (start_row..=end_row)
                .flat_map(move |row| (start_col..=end_col).map(move |col| (col, row))),
        )
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    User(String),
    Assistant(String),
}

#[derive(Debug, Clone)]
pub enum PartialMessage {
    Start,
    Continue(String),
    End,
}

pub fn partial_messages_to_string(partial_messages: Vec<PartialMessage>) -> String {
    let mut result = String::new();

    for message in partial_messages {
        match message {
            PartialMessage::Start => (), // Do nothing for Start
            PartialMessage::Continue(s) => result.push_str(&s),
            PartialMessage::End => (), // Do nothing for End
        }
    }

    result
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
    ExploreFiles,
    ShowContext,
    Help,
    Notify { notification: Notification },
}

#[derive(Debug, Clone)]
pub enum Notification {
    Info(String),
    Error(String),
}

#[derive(Debug, Clone, Copy)]
pub struct TerminalSize {
    pub width: u16,
    pub height: u16,
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
    /// Help text scroll
    pub help_scroll: usize,
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
    /// Selected text
    pub selection: Selection,
    /// Highlighting theme
    pub theme: Theme,
    /// Terminal size
    pub size: Option<TerminalSize>,
    /// Cached highlighted lines
    pub cached_lines: Vec<Line<'a>>,
    /// Is the app receiving streaming messages
    pub is_streaming: bool,
    /// Is the app waiting for a response
    pub is_waiting_for_response: bool,
    /// Spinner animation frame counter
    pub spinner_frame: usize,
    /// File explorer
    pub file_explorer: FileExplorer,
    /// Current context
    pub current_context: Option<Vec<File>>,
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
            help_scroll: 0,
            running: true,
            #[cfg(not(target_os = "linux"))]
            clipboard: Clipboard::new().unwrap(),
            model_list: ModelList::from_iter(MODELS.map(|(provider, model)| {
                if model == "gemini-3-pro-preview" {
                    (provider, model, true)
                } else {
                    (provider, model, false)
                }
            })),
            selected_model_name: "gemini-3.1-pro-preview".to_string(),
            snippet_list: SnippetList::from_iter([].iter().map(|&snippet| (snippet, false, None))),
            chat_list: ChatList::from_iter([].iter().map(|&chat| (chat, "".to_string(), false))),
            selection: Selection::default(),
            theme: load_theme(),
            size: None,
            cached_lines: Vec::new(),
            is_streaming: false,
            is_waiting_for_response: false,
            spinner_frame: 0,
            file_explorer: FileExplorer::with_theme(get_theme())
                .expect("Could not construct file explorer."),
            current_context: None,
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
    pub fn tick(&mut self) {
        if self.is_waiting_for_response {
            self.spinner_frame = self.spinner_frame.wrapping_add(1);
        }
    }

    pub fn set_app_mode(&mut self, new_app_mode: AppMode) {
        self.app_mode = new_app_mode;
    }

    pub fn create_conversation(&mut self) -> AppResult<i64> {
        let conv_id = create_db_conversation(self.system_prompt)
            .context("Failed to create conversation in db")?;
        self.conversation_id = Some(conv_id);
        Ok(conv_id)
    }

    pub fn set_terminal_size(&mut self, width: u16, height: u16) {
        self.size = Some(TerminalSize { width, height });
    }

    pub fn add_cached_lines(&mut self, message: Message) {
        if let Some(TerminalSize { width, height: _ }) = self.size {
            self.cached_lines.extend(style_message(
                message,
                (width - 3) as usize,
                self.theme.clone(),
            ));
        }
    }

    pub fn recache_lines(&mut self, messages: Vec<Message>) {
        self.cached_lines.clear();
        if let Some(TerminalSize { width, height: _ }) = self.size {
            for message in messages {
                self.cached_lines.extend(style_message(
                    message,
                    (width - 3) as usize,
                    self.theme.clone(),
                ));
            }
        }
    }

    fn write_chat_log(&self) -> AppResult<()> {
        let mut chat_log = String::new();
        for message in self.messages.iter() {
            match message {
                Message::User(message) => {
                    chat_log.push_str(&format!("User: {message}\n"));
                }
                Message::Assistant(message) => {
                    chat_log.push_str(&format!("Assistant: {message}\n"));
                }
            }
        }
        let cache_dir = get_cache_dir()?;
        fs::create_dir_all(&cache_dir).context("Could not create cache directory")?;
        let mut path = cache_dir;
        path.push("latest-chat.log");
        fs::write(&path, chat_log).context("Unable to write chat log")?;
        Ok(())
    }

    pub fn add_to_context(&mut self, new_context: File) {
        if let Some(mut current_context) = self.current_context.clone() {
            if !current_context.contains(&new_context) {
                current_context.push(new_context);
                self.current_context = Some(current_context)
            }
        } else {
            self.current_context = Some(vec![new_context]);
        }
    }

    pub fn remove_from_context(&mut self, context: &File) {
        if let Some(mut current_context) = self.current_context.clone() {
            if let Some(idx) = current_context.iter().position(|f| f == context) {
                current_context.remove(idx);
                self.current_context = Some(current_context)
            }
        };
    }

    fn get_max_scroll(&self) -> AppResult<usize> {
        let (width, _) =
            crossterm::terminal::size().context("Could not get terminal size from crossterm")?;
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
            - 2;

        Ok(max_scroll)
    }

    pub fn increment_vertical_scroll(&mut self) -> AppResult<()> {
        let max_scroll = self.get_max_scroll().context("Unable to get max scroll")?;
        if self.vertical_scroll < max_scroll {
            self.vertical_scroll += 1;
        }
        Ok(())
    }

    pub fn decrement_vertical_scroll(&mut self) -> AppResult<()> {
        self.vertical_scroll = self.vertical_scroll.saturating_sub(1);
        Ok(())
    }

    pub fn scroll_to_top(&mut self) {
        self.vertical_scroll = 0;
    }

    pub fn scroll_to_bottom(&mut self) -> AppResult<()> {
        self.vertical_scroll = self.get_max_scroll().context("Unable to get max scroll")?;
        Ok(())
    }

    pub fn increment_help_scroll(&mut self, max_scroll: usize) {
        if self.help_scroll < max_scroll {
            self.help_scroll += 1;
        }
    }

    pub fn decrement_help_scroll(&mut self) {
        self.help_scroll = self.help_scroll.saturating_sub(1);
    }

    pub fn reset_help_scroll(&mut self) {
        self.help_scroll = 0;
    }

    pub fn submit_message(&mut self) -> AppResult<()> {
        let mut text = self.input_textarea.lines().join("\n");
        if let Some(context) = &self.current_context {
            let ps = SyntaxSet::load_defaults_newlines();
            let mut additional_context = "\n\nINFO FOR LLMs\nThe user also provided the following context, please use it (if relevant) when providing an answer:".to_string();
            for file in context {
                let extension = if let Some((_, extension)) = file.name().split_once(".") {
                    extension
                } else {
                    ""
                };
                let syntax_name = if let Some(syntax) = ps.find_syntax_by_extension(extension) {
                    syntax.name.to_string()
                } else {
                    "Plain Text".to_string()
                };
                let context_str = get_file_content(file)?;
                additional_context.push_str(&format!(
                    "\n---\nFile name: {}\nContent:\n```{}\n{}\n```",
                    file.name(),
                    syntax_name.to_lowercase(),
                    context_str
                ));
            }
            text.push_str(&additional_context);
            self.current_context = None;
        }
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
        self.add_cached_lines(message.clone());
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
        self.is_streaming = false;
        if let Some(Message::Assistant(_)) = self.messages.last() {
            self.messages.pop();
        }
        let message_content = message.as_ref();
        let discovered_snippets =
            find_fenced_code_snippets(message_content.split('\n').map(|s| s.to_string()).collect());
        let snippet_items: Vec<SnippetItem> = discovered_snippets
            .into_iter()
            .map(|snippet| snippet.into())
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
        self.add_cached_lines(message.clone());
        self.messages.push(message);
        Ok(())
    }

    pub async fn receive_incomplete_message(&mut self, captured_content: &str) -> AppResult<()> {
        if captured_content.is_empty() {
            self.messages.push(Message::Assistant("".to_string()));
        }
        if let Some(Message::Assistant(last)) = self.messages.last_mut() {
            *last = captured_content.to_string();
        }
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
        let mut assistant_messages = self.messages.iter().filter_map(|m| match m {
            Message::Assistant(message) => Some(message),
            _ => None,
        });
        if let Some(message) = assistant_messages.next_back() {
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

    pub fn get_snippet(&self) -> Option<&SnippetItem> {
        self.snippet_list
            .state
            .selected()
            .map(|i| &self.snippet_list.items[i])
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

    pub fn delete_selected_chat(&mut self) -> AppResult<()> {
        if let Some(i) = self.chat_list.state.selected() {
            let chat_id = self.chat_list.items[i].chat_id;
            delete_conversation(chat_id)?;
            self.chat_list.items.remove(i);
            self.messages.clear();
            self.cached_lines.clear();
            self.messages = list_all_messages(chat_id)?;
            self.conversation_id = None;
            self.recache_lines(self.messages.clone());
        }
        Ok(())
    }

    pub fn delete_chat_by_id(&mut self, id: i64) -> AppResult<()> {
        delete_conversation(id)?;
        Ok(())
    }

    pub fn new_chat(&mut self) {
        if !self.messages.is_empty() {
            self.messages.clear();
            self.cached_lines.clear();
            self.conversation_id = None;
            self.has_unprocessed_messages = false;
            self.is_waiting_for_response = false;
            self.is_streaming = false;
            self.snippet_list = SnippetList::new();
        }
    }

    pub fn redo_last_message(&mut self) -> AppResult<()> {
        self.has_unprocessed_messages = false;
        while let Some(m) = self.messages.pop() {
            if let Some(chat_id) = self.conversation_id {
                delete_message(chat_id, &m)?;
            }
            match m {
                Message::User(s) => {
                    self.input_textarea = styled_input_textarea();
                    self.input_textarea.insert_str(s);
                    break;
                }
                _ => {
                    continue;
                }
            }
        }
        self.recache_lines(self.messages.clone());

        // Clear snippet list and find fenced code snippets
        self.snippet_list.clear();
        for message in self.messages.iter() {
            let message_content = message.as_ref();
            let discovered_snippets = find_fenced_code_snippets(
                message_content.split('\n').map(|s| s.to_string()).collect(),
            );
            let snippet_items: Vec<SnippetItem> = discovered_snippets
                .into_iter()
                .map(|snippet| snippet.into())
                .collect();
            self.snippet_list.items.extend(snippet_items);
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
            for message in self.messages.iter_mut() {
                let message_content = message.as_ref();
                let discovered_snippets = find_fenced_code_snippets(
                    message_content.split('\n').map(|s| s.to_string()).collect(),
                );
                let snippet_items: Vec<SnippetItem> = discovered_snippets
                    .into_iter()
                    .map(|snippet| snippet.into())
                    .collect();
                self.snippet_list.items.extend(snippet_items);
            }
            self.recache_lines(self.messages.clone());
            self.vertical_scroll = 0;
        }
        Ok(())
    }
}
