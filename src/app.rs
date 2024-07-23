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
#[derive(Clone)]
pub struct App<'a> {
    /// Input text area
    pub text_area: TextArea<'a>,
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
}

fn styled_text_area() -> TextArea<'static> {
    let mut text_area = TextArea::default();
    text_area.set_block(Block::bordered().title("Input"));
    text_area.set_style(Style::default().fg(Color::Yellow));
    text_area
}

impl Default for App<'_> {
    fn default() -> Self {
        Self {
            text_area: styled_text_area(),
            input_mode: InputMode::Normal,
            current_message: None,
            messages: Vec::new(),
            user_messages: Vec::new(),
            assistant_messages: Vec::new(),
            vertical_scroll: 0,
            running: true,
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
        let text = self.text_area.lines().join("\n");
        self.current_message = Some(text.clone());
        self.messages.push(format!("USER:\n---\n{}\n", text));
        self.user_messages.push(text.clone());
        self.text_area = styled_text_area();
    }

    pub async fn receive_message(&mut self, message: String) {
        self.messages
            .push(format!("ASSISTANT:\n---\n{}\n", message));
        self.assistant_messages.push(message);
        self.current_message = None;
    }

    pub fn quit(&mut self) {
        self.running = false;
    }
}
