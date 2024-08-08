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
    snippets::{find_fenced_code_snippets, SnippetItem},
};
use crate::{models::ModelList, snippets::SnippetList};

/// Application result type.
pub type AppResult<T> = Result<T>;

#[derive(Clone)]
pub enum AppMode {
    Normal,
    Editing,
    ModelSelection,
    SnippetSelection,
    Help,
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
    /// System clipboard.
    /// Not enabled on Linux because of an issue with the `arboard` crate,
    /// see https://github.com/1Password/arboard/issues/153
    #[cfg(not(target_os = "linux"))]
    pub clipboard: Clipboard,
    /// List of models
    pub model_list: ModelList,
    /// Selected model name
    pub selected_model_name: String,
    /// Discovered snippets
    pub snippet_list: SnippetList,
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

    fn write_chat_log(&self) -> AppResult<()> {
        let mut chat_log = String::new();
        for (i, message) in self.messages.iter().enumerate() {
            if i % 2 == 0 {
                chat_log.push_str(&format!("User: {}\n", message));
            } else {
                chat_log.push_str(&format!("Assistant: {}\n", message));
            }
        }
        let mut path = home_dir().context("Cannot find home directory")?;
        path.push(".cache/ait");
        fs::create_dir_all(&path).context("Could not create cache directory")?;
        path.push("latest-chat.log");
        fs::write(&path, chat_log).context("Unable to write chat log")?;
        Ok(())
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

    pub fn submit_message(&mut self) -> AppResult<()> {
        let text = self.input_textarea.lines().join("\n");
        if text.is_empty() {
            return Ok(());
        }
        if self.user_messages.len() != self.assistant_messages.len() {
            return Ok(());
        }

        self.current_message = Some(text.clone());
        self.messages.push(text.clone());
        self.user_messages.push(text);
        self.input_textarea = styled_input_textarea();
        self.set_app_mode(AppMode::Normal);
        self.write_chat_log()
            .context("Unable to write submitted message to chat log")?;
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

    pub async fn receive_message(&mut self, message: String) -> AppResult<()> {
        self.messages.push(message.clone());
        let discovered_snippets =
            find_fenced_code_snippets(message.split('\n').map(|s| s.to_string()).collect());
        let snippet_items: Vec<SnippetItem> = discovered_snippets
            .iter()
            .map(|snippet| snippet.to_string().into())
            .collect();
        self.snippet_list.items.extend(snippet_items);
        self.assistant_messages.push(message);
        self.current_message = None;
        self.write_chat_log()
            .context("Unable to write received message to chat log")?;
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
        if let Some(message) = self.assistant_messages.last() {
            self.clipboard.set_text(message.clone()).unwrap();
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

    pub fn get_snippet_text(&self) -> Option<String> {
        self.snippet_list
            .state
            .selected()
            .map(|i| self.snippet_list.items[i].text.clone())
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
                .set_text(self.snippet_list.items[i].text.clone())
                .context("Unable to copy snippet to clipboard")?;
        }
        Ok(())
    }
}
