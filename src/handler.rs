use crate::app::{App, AppMode, AppResult};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Handles the key events and updates the state of [`App`].
pub fn handle_key_events(key_event: KeyEvent, app: &mut App) -> AppResult<()> {
    match app.app_mode {
        AppMode::Normal => match key_event.code {
            // Exit application on `ESC` or `q`
            KeyCode::Esc => app.quit(),
            KeyCode::Char('m') => app.set_app_mode(AppMode::ModelSelection),
            KeyCode::Char('i') => app.set_app_mode(AppMode::Editing),
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
        AppMode::Editing => match key_event.code {
            // Exit editing mode on `ESC`
            KeyCode::Esc => app.set_app_mode(AppMode::Normal),
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
        AppMode::ModelSelection => match key_event.code {
            KeyCode::Esc | KeyCode::Char('m') => app.set_app_mode(AppMode::Normal),
            KeyCode::Char('h') | KeyCode::Left => app.select_none(),
            KeyCode::Char('j') | KeyCode::Down => app.select_next(),
            KeyCode::Char('k') | KeyCode::Up => app.select_previous(),
            KeyCode::Char('g') | KeyCode::Home => app.select_first(),
            KeyCode::Char('G') | KeyCode::End => app.select_last(),
            KeyCode::Enter => {
                app.set_model();
                app.set_app_mode(AppMode::Normal);
            }
            // KeyCode::Up => {
            //     todo!()
            // }
            // KeyCode::Down => {
            //     todo!()
            // }
            _ => {}
        },
    }
    Ok(())
}
