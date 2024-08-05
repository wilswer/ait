use crate::app::{App, AppMode, AppResult};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Handles the key events and updates the state of [`App`].
pub fn handle_key_events(key_event: KeyEvent, app: &mut App) -> AppResult<()> {
    match app.app_mode {
        AppMode::Normal => match key_event.code {
            // Exit application on `ESC` or `q`
            KeyCode::Esc | KeyCode::Char('q') => app.quit(),
            KeyCode::Char('m') => app.set_app_mode(AppMode::ModelSelection),
            KeyCode::Char('s') => app.set_app_mode(AppMode::SnippetSelection),
            KeyCode::Char('i') => app.set_app_mode(AppMode::Editing),
            KeyCode::Char('?') => app.set_app_mode(AppMode::Help),
            #[cfg(not(target_os = "linux"))]
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
                    #[cfg(not(target_os = "linux"))]
                    app.paste_to_input_textarea();
                } else {
                    app.input_textarea.input(key_event);
                }
            }
            KeyCode::Enter => {
                if key_event.modifiers == KeyModifiers::NONE {
                    app.input_textarea.input(key_event);
                } else {
                    app.submit_message()?;
                }
            }
            _ => {
                app.input_textarea.input(key_event);
            }
        },
        AppMode::ModelSelection => match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('m') => {
                app.set_app_mode(AppMode::Normal)
            }
            KeyCode::Char('h') | KeyCode::Left => app.select_no_model(),
            KeyCode::Char('j') | KeyCode::Down => app.select_next_model(),
            KeyCode::Char('k') | KeyCode::Up => app.select_previous_model(),
            KeyCode::Char('g') | KeyCode::Home => app.select_first_model(),
            KeyCode::Char('G') | KeyCode::End => app.select_last_model(),
            KeyCode::Enter => {
                app.set_model();
                app.set_app_mode(AppMode::Normal);
            }
            _ => {}
        },
        AppMode::SnippetSelection => match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('s') => {
                app.set_app_mode(AppMode::Normal)
            }
            KeyCode::Char('h') | KeyCode::Left => app.select_no_snippet(),
            KeyCode::Char('j') | KeyCode::Down => app.select_next_snippet(),
            KeyCode::Char('k') | KeyCode::Up => app.select_previous_snippet(),
            KeyCode::Char('g') | KeyCode::Home => app.select_first_snippet(),
            KeyCode::Char('G') | KeyCode::End => app.select_last_snippet(),
            #[cfg(not(target_os = "linux"))]
            KeyCode::Enter | KeyCode::Char('y') => {
                app.copy_snippet()?;
                app.set_app_mode(AppMode::Normal);
            }
            _ => {}
        },
        AppMode::Help => match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                app.set_app_mode(AppMode::Normal)
            }
            _ => {}
        },
    }
    Ok(())
}
