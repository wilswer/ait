use arboard::Clipboard;
use ratatui::{
    style::{Color, Style},
    widgets::Block,
};
use std::error;
use tui_textarea::TextArea;

/// Application result type.
pub type AppResult<T> = std::result::Result<T, Box<dyn error::Error>>;

#[derive(Clone)]
pub enum InputMode {
    Normal,
    Editing,
}

/// App holds the state of the application
pub struct App<'a> {
    /// Input text area
    pub textarea: TextArea<'a>,
    /// Position of cursor in the editor area.
    pub input_mode: InputMode,
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
}

fn styled_textarea() -> TextArea<'static> {
    let mut textarea = TextArea::default();
    textarea.set_block(Block::bordered().title("Input"));
    textarea.set_style(Style::default().fg(Color::Yellow));
    textarea
}

impl Default for App<'_> {
    fn default() -> Self {
        Self {
            textarea: styled_textarea(),
            input_mode: InputMode::Normal,
            current_message: None,
            messages: Vec::new(),
            user_messages: Vec::new(),
            assistant_messages: Vec::new(),
            vertical_scroll: 0,
            running: true,
            clipboard: Clipboard::new().unwrap(),
        }
    }
}

impl App<'_> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles the tick event of the terminal.
    pub fn tick(&self) {}

    pub fn set_input_mode(&mut self, new_input_mode: InputMode) {
        self.input_mode = new_input_mode;
    }

    pub fn increment_vertical_scroll(&mut self) {
        self.vertical_scroll = self.vertical_scroll.saturating_add(1);
    }

    pub fn decrement_vertical_scroll(&mut self) {
        self.vertical_scroll = self.vertical_scroll.saturating_sub(1);
    }

    pub fn submit_message(&mut self) {
        let text = self.textarea.lines().join("\n");
        self.current_message = Some(text.clone());
        self.messages.push(format!("USER:\n---\n{}\n", text));
        self.user_messages.push(text.clone());
        self.textarea = styled_textarea();
    }

    pub async fn receive_message(&mut self, message: String) {
        self.messages
            .push(format!("ASSISTANT:\n---\n{}\n", message));
        self.assistant_messages.push(message);
        self.current_message = None;
    }

    pub fn paste_to_input_textarea(&mut self) {
        if let Ok(clipboard_content) = self.clipboard.get_text() {
            self.textarea.insert_str(clipboard_content);
        }
    }

    pub fn quit(&mut self) {
        self.running = false;
    }
}
