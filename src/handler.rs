use crate::app::{App, AppResult, InputMode};
use crossterm::event::{KeyCode, KeyEvent};

/// Handles the key events and updates the state of [`App`].
pub fn handle_key_events(key_event: KeyEvent, app: &mut App) -> AppResult<()> {
    match app.input_mode {
        InputMode::Normal => match key_event.code {
            // Exit application on `ESC` or `q`
            KeyCode::Esc => app.quit(),
            KeyCode::Char('i') => app.set_input_mode(InputMode::Editing),
            KeyCode::Char('q') => app.quit(),
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
            KeyCode::Char(c) => app.enter_char(c),
            KeyCode::Backspace => app.delete_char(),
            KeyCode::Enter => app.submit_message(),
            KeyCode::Right => app.move_cursor_right(),
            KeyCode::Left => app.move_cursor_left(),
            _ => {}
        },
    }
    Ok(())
}
