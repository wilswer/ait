use std::fmt::Display;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use std::{borrow::Cow, fs::read_to_string, io};

use anyhow::{Context, Result, anyhow};
#[cfg(not(target_os = "linux"))]
use arboard::Clipboard;
use genai::ModelSpec;
use genai::chat::ContentPart;
use syntect::highlighting::Theme;
use syntect::parsing::SyntaxSet;

use ratatui::{
    buffer::Buffer,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, ListState},
};
use ratatui_explorer::{File, FileExplorer, FileExplorerBuilder};
use ratatui_textarea::TextArea;
use tiktoken_rs::cl100k_base;

use crate::config::ModelConfig;
use crate::models::{ModelItem, generate_model_spec};
use crate::ui::messages_to_lines;
use crate::{
    ai::MODELS,
    chats::ChatList,
    snippets::{EMBEDDED_THEME, SnippetItem, find_fenced_code_snippets, load_theme},
    storage::{
        create_db_conversation, delete_conversation, delete_message, get_cache_dir, insert_message,
        list_all_messages, list_conversations, touch_conversation,
    },
    ui::style_message,
};
use crate::{models::ModelList, snippets::SnippetList};

pub const RECACHE_COOLDOWN: u64 = 250;

/// Async actions reported back to the main event loop by background tasks.
#[derive(Debug, Clone)]
pub enum Action {
    StreamStart,
    StreamPartial(String),
    StreamComplete(String),
    StreamCancelled(String),
    Error(String),
    ModelsLoaded(Vec<(String, String)>),
    /// A single file was validated and its tokens estimated in the background.
    /// The file is added to the context with the estimated token count
    /// (`Some` for text files, `None` for recognized binary files).
    ContextFileAdded {
        file: File,
        est_tokens: Option<usize>,
    },
    /// Signals that a background context-add operation finished, switching
    /// the app to the given notification.
    ContextAddDone {
        notification: Notification,
    },
}

pub fn estimate_tokens(text: &str) -> AppResult<usize> {
    let bpe = cl100k_base()?;
    let base_count = bpe.encode_ordinary(text).len();
    Ok(base_count)
}

pub fn get_file_content(path: &PathBuf) -> io::Result<Cow<'_, str>> {
    read_to_string(path).map(Into::into)
}

/// Returns true for binary file types that are added to context as-is (no token
/// estimation is possible).
pub fn is_binary_file(name: &str) -> bool {
    [".pdf", ".jpg", ".png"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// Reads a file (if it is a text file) and estimates its token count.
///
/// Returns `Ok(Some(count))` for readable text files, `Ok(None)` for recognized
/// binary files (`pdf`/`jpg`/`png`), and `Err` for files that are neither valid
/// UTF-8 text nor a recognized binary type.
pub fn estimate_file_tokens(path: &PathBuf) -> AppResult<Option<usize>> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if is_binary_file(&name) {
        tracing::debug!(
            path = %path.display(),
            "skipped token estimation: recognized binary file"
        );
        return Ok(None);
    }
    match get_file_content(path) {
        Ok(content) => Ok(Some(estimate_tokens(content.as_ref())?)),
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "skipped file during token estimation: could not read as UTF-8 text"
            );
            Err(anyhow!(
                "Could not read file \"{}\" as text: {}",
                path.display(),
                e
            ))
        }
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
pub enum UserContent {
    Input,
    Context,
}

#[derive(Debug, Clone)]
pub enum Message {
    User(Vec<ContentPart>),
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
        Message::User(vec![ContentPart::from_text(message)])
    }
}

impl From<&str> for Message {
    fn from(message: &str) -> Self {
        Message::User(vec![ContentPart::from_text(message)])
    }
}

impl Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Message::User(content) => {
                for part in content {
                    if let ContentPart::Text(text) = part {
                        write!(f, "{}", text)?;
                    }
                }
                Ok(())
            }
            Message::Assistant(text) => write!(f, "{}", text),
        }
    }
}

/// Application result type.
pub type AppResult<T> = Result<T>;

pub const THINKING_EFFORTS: [&str; 6] = ["None", "Low", "Medium", "High", "XHigh", "Max"];

#[derive(Debug, Clone, PartialEq, Default)]
pub enum ThinkingEffort {
    #[default]
    None,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl ThinkingEffort {
    pub fn as_str(&self) -> &'static str {
        match self {
            ThinkingEffort::None => "None",
            ThinkingEffort::Low => "Low",
            ThinkingEffort::Medium => "Medium",
            ThinkingEffort::High => "High",
            ThinkingEffort::XHigh => "XHigh",
            ThinkingEffort::Max => "Max",
        }
    }

    pub fn from_index(i: usize) -> Self {
        match i {
            1 => ThinkingEffort::Low,
            2 => ThinkingEffort::Medium,
            3 => ThinkingEffort::High,
            4 => ThinkingEffort::XHigh,
            5 => ThinkingEffort::Max,
            _ => ThinkingEffort::None,
        }
    }

    pub fn to_index(&self) -> usize {
        match self {
            ThinkingEffort::None => 0,
            ThinkingEffort::Low => 1,
            ThinkingEffort::Medium => 2,
            ThinkingEffort::High => 3,
            ThinkingEffort::XHigh => 4,
            ThinkingEffort::Max => 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    Editing,
    ModelSelection,
    FilterModels,
    ThinkingEffortSelection,
    SnippetSelection,
    ShowHistory,
    FilterHistory,
    ExploreFiles,
    ShowContext,
    Help,
    Notify { notification: Notification },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Notification {
    TokenEstimate((Option<usize>, String)),
    Info(String),
    Error(String),
}

#[derive(Debug, Clone, Copy)]
pub struct TerminalSize {
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone)]
pub struct ContextContent {
    pub file: File,
    pub est_tokens: Option<usize>,
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
    /// Selected model
    pub selected_model: ModelSpec,
    /// Discovered snippets
    pub snippet_list: SnippetList,
    /// List of chats
    pub chat_list: ChatList,
    /// Selected text
    pub selection: Selection,
    /// Highlighting theme index
    pub theme_index: usize,
    /// Highlighting theme
    pub theme: Theme,
    /// Terminal size
    pub size: Option<TerminalSize>,
    /// Cached highlighted lines
    pub cached_lines: Vec<Line<'a>>,
    /// Does the app need to recache the syntax highlighting?
    pub needs_recache: bool,
    /// Time of last recaching of syntax highlighting
    pub last_recache: Instant,
    /// Is the app receiving streaming messages
    pub is_streaming: bool,
    /// Is the app waiting for a response
    pub is_waiting_for_response: bool,
    /// Spinner animation frame counter
    pub spinner_frame: usize,
    /// File explorer
    pub file_explorer: FileExplorer,
    /// Current context
    pub current_context: Option<Vec<ContextContent>>,
    /// Search bar.
    pub search_bar: TextArea<'a>,
    /// Toggle for syntax highlighting.
    pub do_highlight: bool,
    /// Selected thinking effort
    pub thinking_effort: ThinkingEffort,
    /// List state for thinking effort selection
    pub thinking_effort_state: ListState,
    /// Is the app loading available models?
    pub is_loading_models: bool,
}

pub fn styled_textarea(title: &'static str) -> TextArea<'static> {
    let mut input_textarea = TextArea::default();
    input_textarea.set_block(Block::bordered().title(title));
    input_textarea.set_style(Style::default().fg(Color::Yellow));
    input_textarea
}

impl Default for App<'_> {
    fn default() -> Self {
        Self {
            input_textarea: styled_textarea("Input"),
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
                if model == "gemini-3.1-pro-preview" {
                    (provider, model, true)
                } else {
                    (provider, model, false)
                }
            })),
            selected_model: "gemini-3.1-pro-preview".into(),
            snippet_list: SnippetList::from_iter([].iter().map(|&snippet| (snippet, false, None))),
            chat_list: ChatList::from_iter([].iter().map(|&chat| (chat, "".to_string(), false))),
            selection: Selection::default(),
            theme_index: 0,
            theme: load_theme(0),
            size: None,
            cached_lines: Vec::new(),
            needs_recache: false,
            last_recache: Instant::now() - Duration::from_secs(1),
            is_streaming: false,
            is_waiting_for_response: false,
            spinner_frame: 0,
            file_explorer: FileExplorerBuilder::default()
                .show_hidden(true)
                .theme(get_theme())
                .build()
                .expect("Could not construct file explorer."),
            current_context: None,
            search_bar: styled_textarea("Search"),
            do_highlight: true,
            thinking_effort: ThinkingEffort::None,
            thinking_effort_state: {
                let mut s = ListState::default();
                s.select_first();
                s
            },
            is_loading_models: true,
        }
    }
}

impl<'a> App<'a> {
    pub fn new(system_prompt: &'a str, default_model: ModelConfig) -> Self {
        let model_list = ModelList::from_iter(MODELS.map(|(provider, name)| {
            if name == default_model.name {
                (provider, name, true)
            } else {
                (provider, name, false)
            }
        }));
        Self {
            system_prompt,
            selected_model: generate_model_spec(
                default_model.name.as_str(),
                default_model.provider.as_str(),
            ),
            model_list,
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
            // line width inside the chat block: terminal width minus the outer
            // layout margins (2) and the chat block borders (2).
            self.cached_lines.extend(style_message(
                message,
                width.saturating_sub(4) as usize,
                self.theme.clone(),
            ));
        }
    }

    /// Returns an estimate for token usage of all messages sent and receved from the LLM.
    pub fn estimate_messages_tokens(&self) -> usize {
        let count: usize = self
            .messages
            .iter()
            .map(|m| estimate_tokens(&m.to_string()).unwrap_or(0))
            .sum();
        count
    }

    /// Estimate the tokens for the provided text.
    pub fn estimate_tokens(&self, text: &str) -> usize {
        estimate_tokens(text).unwrap_or(0)
    }

    pub fn next_theme(&mut self) {
        if self.theme_index == EMBEDDED_THEME.len() - 1 {
            self.theme_index = 0;
        } else {
            self.theme_index += 1;
        }
        self.theme = load_theme(self.theme_index);
    }

    pub fn previous_theme(&mut self) {
        if self.theme_index == 0 {
            self.theme_index = EMBEDDED_THEME.len() - 1;
        } else {
            self.theme_index -= 1;
        }
        self.theme = load_theme(self.theme_index);
    }

    pub fn toggle_highlighting(&mut self) {
        self.do_highlight = !self.do_highlight;
    }

    pub fn recache_lines(&mut self, messages: Vec<Message>) {
        self.cached_lines.clear();
        if let Some(TerminalSize { width, height: _ }) = self.size {
            for message in messages {
                self.cached_lines.extend(style_message(
                    message,
                    width.saturating_sub(4) as usize,
                    self.theme.clone(),
                ));
            }
        }
    }

    fn write_chat_log(&self) -> AppResult<()> {
        let mut chat_log = String::new();
        for message in self.messages.iter() {
            match message {
                Message::User(_) => {
                    chat_log.push_str(&format!("User: {}\n", message));
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

    pub fn add_to_context(&mut self, new_context: File, est_tokens: Option<usize>) {
        if let Some(mut current_context) = self.current_context.clone() {
            if !current_context
                .iter()
                .map(|c| c.file.to_owned())
                .collect::<Vec<File>>()
                .contains(&new_context)
            {
                current_context.push(ContextContent {
                    file: new_context,
                    est_tokens,
                });
                self.current_context = Some(current_context)
            }
        } else {
            self.current_context = Some(vec![ContextContent {
                file: new_context,
                est_tokens,
            }]);
        }
    }

    pub fn remove_from_context(&mut self, context: &File) {
        if let Some(mut current_context) = self.current_context.clone()
            && let Some(idx) = current_context.iter().position(|f| &f.file == context)
        {
            current_context.remove(idx);
            self.current_context = Some(current_context)
        };
    }

    fn get_max_scroll(&self) -> AppResult<usize> {
        let TerminalSize { width, height } =
            self.size.ok_or(anyhow!("Could not get terminal size"))?;
        // Bubble lines are pre-wrapped to fit the chat block, and the chat
        // paragraph is rendered without wrapping, so the line count is simply
        // the number of generated lines.
        let total_lines = if !self.is_streaming && self.do_highlight {
            self.cached_lines.len()
        } else {
            messages_to_lines(&self.messages, width.saturating_sub(4) as usize).len()
        };
        let sub = if self.is_streaming {
            (height - 4) as usize
        } else if self.has_unprocessed_messages {
            (height - 8) as usize
        } else {
            2
        };
        Ok(total_lines.saturating_sub(sub))
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
        let text = self.input_textarea.lines().join("\n");
        if text.is_empty() {
            return Ok(());
        }
        let mut content_parts = Vec::new();
        if let Some(context) = &self.current_context {
            let ps = SyntaxSet::load_defaults_newlines();
            let additional_context = "<context>\nINFO FOR LLMs\nThe user provided the following context, please use it (if relevant) when providing an answer:".to_string();
            content_parts.push(ContentPart::from_text(additional_context));
            for c in context {
                let extension = if let Some((_, extension)) = c.file.name.split_once(".") {
                    extension
                } else {
                    ""
                };
                match extension {
                    "pdf" | "jpg" | "png" => {
                        content_parts.push(ContentPart::from_text(format!(
                            "\n---\nFile name: {}\nContent:\n<binary file>",
                            &c.file.name
                        )));
                        content_parts.push(ContentPart::from_binary_file(c.file.path.clone())?);
                    }
                    _ => {
                        let syntax_name =
                            if let Some(syntax) = ps.find_syntax_by_extension(extension) {
                                syntax.name.to_string()
                            } else {
                                "Plain Text".to_string()
                            };
                        if c.file.is_file() {
                            let context_str = get_file_content(&c.file.path)?;
                            content_parts.push(ContentPart::from_text(format!(
                                "\n---\nFile name: {}\nContent:\n```{}\n{}\n```",
                                &c.file.name,
                                syntax_name.to_lowercase(),
                                context_str
                            )));
                        }
                    }
                }
            }
            self.current_context = None;
            content_parts.push(ContentPart::from_text("\n</context>\n"));
        }
        content_parts.push(ContentPart::from_text(&text));
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
        self.reset_input_textarea();
        self.set_app_mode(AppMode::Normal);
        self.write_chat_log()
            .context("Unable to write submitted message to chat log")?;
        let message = Message::User(content_parts);
        if let Some(id) = self.conversation_id {
            insert_message(id, &message)?;
        } else {
            let id = self.create_conversation()?;
            insert_message(id, &message)?;
        }
        self.add_cached_lines(message.clone());
        self.messages.push(message);

        self.scroll_to_bottom()
            .context("Scrolling to bottom failed.")?;

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
        let message_content = message.to_string();
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
            touch_conversation(id)?;
        } else {
            let id = self.create_conversation()?;
            insert_message(id, &message)?;
            touch_conversation(id)?;
        }
        self.add_cached_lines(message.clone());
        self.messages.push(message);
        Ok(())
    }

    pub async fn receive_incomplete_message(&mut self, captured_content: &str) -> AppResult<()> {
        // If we are already scrolled to the bottom, continue scrolling.
        let do_scroll =
            self.vertical_scroll == self.get_max_scroll().context("Could not get max scroll.")?;
        if captured_content.is_empty() {
            self.messages.push(Message::Assistant("".to_string()));
        }
        if let Some(Message::Assistant(last)) = self.messages.last_mut() {
            *last = captured_content.to_string();
        }
        if do_scroll {
            self.scroll_to_bottom()
                .context("Could not set max scroll in incomplete message.")?;
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

    pub fn select_next_thinking_effort(&mut self) {
        self.thinking_effort_state.select_next();
    }

    pub fn select_previous_thinking_effort(&mut self) {
        self.thinking_effort_state.select_previous();
    }

    pub fn select_first_thinking_effort(&mut self) {
        self.thinking_effort_state.select_first();
    }

    pub fn select_last_thinking_effort(&mut self) {
        self.thinking_effort_state
            .select(Some(THINKING_EFFORTS.len() - 1));
    }

    pub fn set_thinking_effort(&mut self) {
        if let Some(i) = self.thinking_effort_state.selected() {
            self.thinking_effort = ThinkingEffort::from_index(i);
        }
    }

    /// Returns indices into `model_list.items` that match the current search bar
    /// query. When the query is empty, returns all indices.
    pub fn filtered_model_indices(&self) -> Vec<usize> {
        let query = self.search_bar.lines().first().cloned().unwrap_or_default();
        let query_lower = query.to_lowercase();
        if query_lower.is_empty() {
            return (0..self.model_list.items.len()).collect();
        }
        self.model_list
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                format!("{}: {}", item.provider, item.name)
                    .to_lowercase()
                    .contains(&query_lower)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Changes the status of the selected list item
    pub fn set_model(&mut self) {
        let indices = self.filtered_model_indices();
        if let Some(sel) = self.model_list.state.selected()
            && let Some(&actual_idx) = indices.get(sel)
        {
            for item in self.model_list.items.iter_mut() {
                item.selected = false;
            }
            self.model_list.items[actual_idx].selected = true;
            let ModelItem { provider, name, .. } = self.model_list.items[actual_idx].clone();
            let model_spec = generate_model_spec(name.as_str(), provider.as_str());
            self.selected_model = model_spec;
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

    pub fn set_chat_list(&mut self, query_filter: Option<String>) -> AppResult<()> {
        let chats = list_conversations(query_filter)?;
        let chats = chats
            .into_iter()
            .map(|(id, started_at)| {
                if Some(&id) == self.get_selected_chat_id() {
                    (id, started_at, true)
                } else {
                    (id, started_at, false)
                }
            })
            .collect::<Vec<(i64, String, bool)>>();
        self.chat_list = ChatList::from_iter(chats);
        Ok(())
    }

    pub fn delete_selected_chat(&mut self) -> AppResult<()> {
        if let Some(i) = self.chat_list.state.selected() {
            let chat_id = self.chat_list.items[i].chat_id;
            delete_conversation(chat_id)?;
            self.chat_list.items.remove(i);
            let new_chat_index = if i >= self.chat_list.items.len() {
                i - 1
            } else {
                i
            };
            self.chat_list.items[new_chat_index].selected = true;
            self.chat_list.state.select(Some(new_chat_index));
            let new_chat_id = self.chat_list.items[new_chat_index].chat_id;
            self.messages.clear();
            self.cached_lines.clear();
            self.messages = list_all_messages(new_chat_id)?;
            self.conversation_id = Some(new_chat_id);
            self.needs_recache = true;
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

    pub fn reset_searchbar(&mut self) {
        self.search_bar = styled_textarea("Search")
    }

    pub fn reset_input_textarea(&mut self) {
        self.input_textarea = styled_textarea("Input")
    }

    pub fn redo_last_message(&mut self) -> AppResult<()> {
        self.has_unprocessed_messages = false;
        while let Some(m) = self.messages.pop() {
            if let Some(chat_id) = self.conversation_id {
                delete_message(chat_id, &m)?;
            }
            match m {
                Message::User(_) => {
                    self.reset_input_textarea();
                    let message_text = m.to_string();
                    // TODO: A bit fugly, should be a better way to do this.
                    if let Some((_, user_input)) = message_text.split_once("\n</context>\n") {
                        self.input_textarea.insert_str(user_input);
                    } else {
                        self.input_textarea.insert_str(m.to_string());
                    }
                    break;
                }
                _ => {
                    continue;
                }
            }
        }
        self.needs_recache = true;

        // Clear snippet list and find fenced code snippets
        self.snippet_list.clear();
        for message in self.messages.iter() {
            let message_content = message.to_string();
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
            self.reset_searchbar();
            for item in self.chat_list.items.iter_mut() {
                item.selected = false;
            }
            self.chat_list.items[i].selected = true;
            self.conversation_id = Some(self.chat_list.items[i].chat_id);
            self.messages.clear();
            self.messages = list_all_messages(self.chat_list.items[i].chat_id)?;
            self.snippet_list.clear();
            for message in self.messages.iter_mut() {
                let message_content = message.to_string();
                let discovered_snippets = find_fenced_code_snippets(
                    message_content.split('\n').map(|s| s.to_string()).collect(),
                );
                let snippet_items: Vec<SnippetItem> = discovered_snippets
                    .into_iter()
                    .map(|snippet| snippet.into())
                    .collect();
                self.snippet_list.items.extend(snippet_items);
            }
            self.needs_recache = true;
            self.vertical_scroll = 0;
        }
        Ok(())
    }
}
