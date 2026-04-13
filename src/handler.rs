use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::event::{MouseEvent, MouseEventKind};
use ratatui_explorer::Input;

use crate::app::{get_file_content, App, AppMode, AppResult, Notification, RECACHE_COOLDOWN};

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
                let query_filter = app.search_bar.lines().first();
                app.set_chat_list(query_filter.map(|x| x.to_string()))?;
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
                if modifiers.contains(KeyModifiers::CONTROL)
                    && !app.has_unprocessed_messages
                    && !app.is_waiting_for_response
                {
                    app.redo_last_message()?;
                    app.set_app_mode(AppMode::Editing);
                } else if app.last_recache.elapsed() >= Duration::from_millis(RECACHE_COOLDOWN)
                    && !app.is_streaming
                {
                    app.toggle_highlighting();
                    app.last_recache = Instant::now();
                }
            }
            KeyCode::Char('n') => app.new_chat(),
            KeyCode::Char('f') => {
                app.set_app_mode(AppMode::ExploreFiles);
            }
            KeyCode::Char('c') => {
                app.set_app_mode(AppMode::ShowContext);
            }
            KeyCode::Char('e') => {
                app.thinking_effort_state
                    .select(Some(app.thinking_effort.to_index()));
                app.set_app_mode(AppMode::ThinkingEffortSelection);
            }
            KeyCode::Char('t') => {
                if app.last_recache.elapsed() >= Duration::from_millis(RECACHE_COOLDOWN) {
                    app.next_theme();
                    app.needs_recache = true;
                    app.last_recache = Instant::now();
                }
            }
            KeyCode::Char('T') => {
                if app.last_recache.elapsed() >= Duration::from_millis(RECACHE_COOLDOWN) {
                    app.previous_theme();
                    app.needs_recache = true;
                    app.last_recache = Instant::now();
                }
            }
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
            KeyCode::Esc | KeyCode::Char('q') => {
                app.reset_searchbar();
                app.set_app_mode(AppMode::Normal);
            }
            KeyCode::Char('h') | KeyCode::Left => app.select_no_chat(),
            KeyCode::Char('j') | KeyCode::Down => app.select_next_chat(),
            KeyCode::Char('k') | KeyCode::Up => app.select_previous_chat(),
            KeyCode::Char('g') | KeyCode::Home => app.select_first_chat(),
            KeyCode::Char('G') | KeyCode::End => app.select_last_chat(),
            KeyCode::Enter => {
                app.set_chat()?;
                app.set_app_mode(AppMode::Normal);
            }
            KeyCode::Char('r') => {
                if modifiers.contains(KeyModifiers::CONTROL) {
                    app.delete_selected_chat()?;
                    let query_filter = app.search_bar.lines().first();
                    app.set_chat_list(query_filter.map(|x| x.to_string()))?;
                }
            }
            KeyCode::Char('/') => {
                app.set_app_mode(AppMode::FilterHistory);
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
        AppMode::ThinkingEffortSelection => match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('e') => {
                app.set_app_mode(AppMode::Normal)
            }
            KeyCode::Char('j') | KeyCode::Down => app.select_next_thinking_effort(),
            KeyCode::Char('k') | KeyCode::Up => app.select_previous_thinking_effort(),
            KeyCode::Char('g') | KeyCode::Home => app.select_first_thinking_effort(),
            KeyCode::Char('G') | KeyCode::End => app.select_last_thinking_effort(),
            KeyCode::Enter => {
                app.set_thinking_effort();
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
                app.copy_snippet()
                    .context("Error when copying snippet to clipboard")?;
                app.set_app_mode(AppMode::Normal);
            }
            _ => {}
        },
        AppMode::ExploreFiles => match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') => app.set_app_mode(AppMode::Normal),
            KeyCode::Char('h') | KeyCode::Left => app.file_explorer.handle(Input::Left)?,
            KeyCode::Char('l') | KeyCode::Right => app.file_explorer.handle(Input::Right)?,
            KeyCode::Char('j') | KeyCode::Down => app.file_explorer.handle(Input::Down)?,
            KeyCode::Char('k') | KeyCode::Up => app.file_explorer.handle(Input::Up)?,
            KeyCode::Enter => {
                let current_file = app.file_explorer.current();
                if current_file.is_file() {
                    let current_name = current_file.name.to_string();
                    let is_valid_file = get_file_content(current_file).is_ok()
                        || [".png", ".jpg", ".pdf"]
                            .iter()
                            .any(|ext| current_name.ends_with(ext));

                    let notification = if is_valid_file {
                        app.add_to_context(current_file.clone());
                        Notification::Info(format!("File {} added to context!", current_name))
                    } else {
                        Notification::Error(format!(
                            "Could not add file {} to context.",
                            current_name
                        ))
                    };
                    app.set_app_mode(AppMode::Notify { notification });
                }
            }
            KeyCode::Char('d') => {
                if app.file_explorer.current().is_file() {
                    app.remove_from_context(&app.file_explorer.current().clone());
                    app.set_app_mode(AppMode::Notify {
                        notification: Notification::Info(format!(
                            "File {} removed from context!",
                            &app.file_explorer.current().name
                        )),
                    })
                }
            }
            _ => {}
        },
        AppMode::ShowContext => match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => app.set_app_mode(AppMode::Normal),
            _ => {}
        },
        AppMode::Notify { notification: _ } => match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => app.set_app_mode(AppMode::Normal),
            _ => {}
        },
        AppMode::Help => match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                app.reset_help_scroll();
                app.set_app_mode(AppMode::Normal)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.increment_help_scroll(30);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.decrement_help_scroll();
            }
            KeyCode::Char('g') => {
                app.reset_help_scroll();
            }
            KeyCode::Char('G') => {
                app.help_scroll = 30;
            }
            _ => {}
        },
        AppMode::FilterHistory => match code {
            KeyCode::Enter => {
                app.set_chat()?;
                app.set_app_mode(AppMode::Normal);
            }
            KeyCode::Up => {
                app.set_app_mode(AppMode::ShowHistory);
                app.select_previous_chat();
            }
            KeyCode::Down => {
                app.set_app_mode(AppMode::ShowHistory);
                app.select_next_chat();
            }
            KeyCode::Esc => {
                app.reset_searchbar();
                app.set_app_mode(AppMode::ShowHistory);
                app.set_chat_list(None)?;
            }
            _ => {
                app.search_bar.input(key_event);
                let query_filter = app.search_bar.lines().first().cloned();
                app.set_chat_list(query_filter)?;
            }
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
