use arboard::Clipboard;
use ratatui::{
    style::{Color, Style},
    widgets::Block,
};
use std::error;
use tui_textarea::TextArea;

use crate::ai::MODELS;
use crate::models::ModelList;

/// Application result type.
pub type AppResult<T> = std::result::Result<T, Box<dyn error::Error>>;

#[derive(Clone)]
pub enum AppMode {
    Normal,
    Editing,
    ModelSelection,
}

/// App holds the state of the application
pub struct App<'a> {
    /// Input text area
    pub input_textarea: TextArea<'a>,
    /// Position of cursor in the editor area.
    pub app_mode: AppMode,
    /// Current message to process
    pub current_message: Option<String>,
    /// History of recorded messages
    pub messages: Vec<String>,
    /// History of recorded messages
    pub user_messages: Vec<String>,
    /// History of recorded messages
    pub assistant_messages: Vec<String>,
    /// Vertical scroll
    pub vertical_scroll: usize,
    /// Is the application running?
    pub running: bool,
    /// Is the application running?
    pub clipboard: Clipboard,
    /// List of models
    pub model_list: ModelList,
    /// Selected model name
    pub selected_model_name: String,
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
            current_message: None,
            messages: Vec::new(),
            user_messages: Vec::new(),
            assistant_messages: Vec::new(),
            vertical_scroll: 0,
            running: true,
            clipboard: Clipboard::new().unwrap(),
            model_list: ModelList::from_iter(MODELS.iter().map(|&model| {
                if model == "gpt-4o-mini" {
                    (model, true)
                } else {
                    (model, false)
                }
            })),
            selected_model_name: "gpt-4o-mini".to_string(),
        }
    }
}

impl App<'_> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles the tick event of the terminal.
    pub fn tick(&self) {}

    pub fn set_app_mode(&mut self, new_app_mode: AppMode) {
        self.app_mode = new_app_mode;
    }

    pub fn increment_vertical_scroll(&mut self) {
        let max_scroll = self
            .messages
            .join("\n")
            .split('\n')
            .collect::<Vec<&str>>()
            .len()
            + 3 * (self.messages.len())
            - 1;
        if self.vertical_scroll < max_scroll {
            self.vertical_scroll += 1;
        }
    }

    pub fn decrement_vertical_scroll(&mut self) {
        self.vertical_scroll = self.vertical_scroll.saturating_sub(1);
    }

    pub fn submit_message(&mut self) {
        let text = self.input_textarea.lines().join("\n");
        if text.is_empty() {
            return;
        }
        self.current_message = Some(text.clone());
        self.messages.push(text.clone());
        self.user_messages.push(text);
        self.input_textarea = styled_input_textarea();
        self.set_app_mode(AppMode::Normal);
    }

    pub async fn receive_message(&mut self, message: String) {
        self.messages.push(message.clone());
        self.assistant_messages.push(message);
        self.current_message = None;
    }

    pub fn paste_to_input_textarea(&mut self) {
        if let Ok(clipboard_content) = self.clipboard.get_text() {
            self.input_textarea.insert_str(clipboard_content);
        }
    }

    pub fn yank_latest_assistant_message(&mut self) {
        if let Some(message) = self.assistant_messages.last() {
            self.clipboard.set_text(message.clone()).unwrap();
        }
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn select_none(&mut self) {
        self.model_list.state.select(None);
    }

    pub fn select_next(&mut self) {
        self.model_list.state.select_next();
    }
    pub fn select_previous(&mut self) {
        self.model_list.state.select_previous();
    }

    pub fn select_first(&mut self) {
        self.model_list.state.select_first();
    }

    pub fn select_last(&mut self) {
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
}
