use crate::app::{App, AppResult, InputMode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Handles the key events and updates the state of [`App`].
pub fn handle_key_events(key_event: KeyEvent, app: &mut App) -> AppResult<()> {
    match app.input_mode {
        InputMode::Normal => match key_event.code {
            // Exit application on `ESC` or `q`
            KeyCode::Esc => app.quit(),
            KeyCode::Char('i') => app.set_input_mode(InputMode::Editing),
            KeyCode::Char('q') => app.quit(),
            KeyCode::Char('y') => app.yank_latest_assistant_message(),
            KeyCode::Up | KeyCode::Char('k') => {
                app.decrement_vertical_scroll();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.increment_vertical_scroll();
            }
            _ => {}
        },
        InputMode::Editing => match key_event.code {
            // Exit editing mode on `ESC`
            KeyCode::Esc => app.set_input_mode(InputMode::Normal),
            KeyCode::Char('V') | KeyCode::Char('v') => {
                if key_event.modifiers == KeyModifiers::CONTROL {
                    app.paste_to_input_textarea();
                } else {
                    app.input_textarea.input(key_event);
                }
            }
            KeyCode::Enter => {
                if key_event.modifiers == KeyModifiers::NONE {
                    app.input_textarea.input(key_event);
                } else {
                    app.submit_message();
                }
            }
            _ => {
                app.input_textarea.input(key_event);
            }
        },
    }
    Ok(())
}
