use crate::app::{App, AppMode, AppResult};

use anyhow::Context;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::event::{MouseEvent, MouseEventKind};

/// Handles the key events and updates the state of [`App`].
pub fn handle_key_events(key_event: KeyEvent, app: &mut App) -> AppResult<()> {
    let KeyEvent {
        code, modifiers, ..
    } = key_event;
    match app.app_mode {
        AppMode::Normal => match code {
            // Exit application on `ESC` or `q`
            KeyCode::Esc | KeyCode::Char('q') => app.quit(),
            KeyCode::Char('m') => app.set_app_mode(AppMode::ModelSelection),
            KeyCode::Char('s') => {
                app.snippet_list.state.select_first();
                app.set_app_mode(AppMode::SnippetSelection);
            }
            KeyCode::Char('i') => app.set_app_mode(AppMode::Editing),
            KeyCode::Char('h') => {
                app.set_chat_list()?;
                app.set_app_mode(AppMode::ShowHistory)
            }
            KeyCode::Char('?') => app.set_app_mode(AppMode::Help),
            #[cfg(not(target_os = "linux"))]
            KeyCode::Char('y') => app.yank_latest_assistant_message(),
            KeyCode::Up | KeyCode::Char('k') => {
                app.decrement_vertical_scroll()?;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.increment_vertical_scroll()?;
            }
            KeyCode::Char('g') => {
                app.scroll_to_top();
            }
            KeyCode::Char('G') => {
                let _ = app.scroll_to_bottom();
            }
            KeyCode::Char('r') => {
                app.redo_last_message()?;
                app.set_app_mode(AppMode::Editing);
            }
            KeyCode::Char('n') => app.new_chat(),
            _ => {}
        },
        AppMode::Editing => match code {
            // Exit editing mode on `ESC`
            KeyCode::Esc => app.set_app_mode(AppMode::Normal),
            KeyCode::Char('V') | KeyCode::Char('v') => {
                if modifiers.contains(KeyModifiers::CONTROL) {
                    #[cfg(not(target_os = "linux"))]
                    app.paste_to_input_textarea();
                } else {
                    app.input_textarea.input(key_event);
                }
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if modifiers.contains(KeyModifiers::CONTROL) {
                    app.submit_message()
                        .context("Handler failed to submit message")?;
                } else {
                    app.input_textarea.input(key_event);
                }
            }
            _ => {
                app.input_textarea.input(key_event);
            }
        },
        AppMode::ShowHistory => match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') => app.set_app_mode(AppMode::Normal),
            KeyCode::Char('h') | KeyCode::Left => app.select_no_chat(),
            KeyCode::Char('j') | KeyCode::Down => app.select_next_chat(),
            KeyCode::Char('k') | KeyCode::Up => app.select_previous_chat(),
            KeyCode::Char('g') | KeyCode::Home => app.select_first_chat(),
            KeyCode::Char('G') | KeyCode::End => app.select_last_chat(),
            KeyCode::Enter => {
                app.set_chat()?;
                app.set_app_mode(AppMode::Normal);
            }
            KeyCode::Char('d') => {
                app.delete_selected_chat()?;
                app.set_chat_list()?;
            }
            _ => {}
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
                app.set_app_mode(AppMode::Editing);
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
                app.copy_snippet()
                    .context("Error when copying snippet to clipboard")?;
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

pub fn handle_mouse_events(event: MouseEvent, app: &mut App) -> AppResult<()> {
    match event.kind {
        MouseEventKind::Down(_) => {
            // Start selection
            app.selection.start = Some((event.column, event.row));
            app.selection.end = Some((event.column, event.row));
        }
        MouseEventKind::Drag(_) => {
            // Update selection end point while dragging
            if app.selection.start.is_some() {
                app.selection.end = Some((event.column, event.row));
            }
        }
        MouseEventKind::Up(_) => {
            app.selection.start = None;
            app.selection.end = None;
        }
        MouseEventKind::ScrollDown => {
            app.increment_vertical_scroll()?;
        }
        MouseEventKind::ScrollUp => {
            app.decrement_vertical_scroll()?;
        }
        _ => {}
    }
    Ok(())
}
