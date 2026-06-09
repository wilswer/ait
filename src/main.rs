use anyhow::Context;
use clap::Parser;
use crossterm::event::EnableMouseCapture;
use crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};
use futures::{FutureExt, StreamExt};
use genai::chat::{ChatStreamEvent, StreamChunk, StreamEnd};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;
use tokio::task;

use ait::ai::{assistant_response_streaming, get_models};
use ait::app::{App, AppMode, AppResult, Message, Notification};
use ait::cli::Cli;
use ait::event::{Event, EventHandler};
use ait::handler::{handle_key_events, handle_mouse_events};
use ait::storage::{create_db, migrate_db};
use ait::tui::Tui;

#[derive(Debug, Clone)]
enum Action {
    StreamStart,
    StreamPartial(String),
    StreamComplete(String),
    StreamCancelled(String),
    Error(String),
    ModelsLoaded(Vec<(String, String)>),
}

/// Handle a single terminal event (key/mouse/tick/resize).
fn handle_event(
    event: Event,
    app: &mut App,
    current_cancel_tx: &mut Option<mpsc::Sender<()>>,
) -> AppResult<()> {
    match event {
        Event::Tick => app.tick(),
        Event::Key(key_event) => {
            if key_event.code == crossterm::event::KeyCode::Char('u')
                && app.app_mode == AppMode::Normal
            {
                // If we have an active stream, send the cancel signal
                if let Some(tx) = current_cancel_tx.take() {
                    let _ = tx.try_send(());
                }
            }
            handle_key_events(key_event, app).context("Error handling key events")?;
        }
        Event::Mouse(mouse_event) => {
            handle_mouse_events(mouse_event, app)?;
        }
        Event::Resize(x, y) => {
            app.set_terminal_size(x, y);
            app.needs_recache = true;
        }
    }
    Ok(())
}

/// Handle a single async action coming back from a spawned task.
async fn handle_action(action: Action, app: &mut App<'_>) -> AppResult<()> {
    match action {
        Action::StreamStart => {
            app.receive_incomplete_message("").await?;
        }
        Action::StreamPartial(content) => {
            app.is_streaming = true;
            app.is_waiting_for_response = false;
            app.receive_incomplete_message(&content).await?;
        }
        Action::StreamComplete(content) => {
            app.is_streaming = false;
            app.receive_message(Message::Assistant(content)).await?;
        }
        Action::StreamCancelled(content) => {
            app.is_streaming = false;
            app.is_waiting_for_response = false;
            // Persist whatever portion of the message was generated before stopping
            app.receive_message(Message::Assistant(content)).await?;
        }
        Action::Error(err_msg) => {
            app.is_waiting_for_response = false;
            app.has_unprocessed_messages = false;
            app.is_streaming = false;
            app.set_app_mode(AppMode::Notify {
                notification: Notification::Error(err_msg),
            });
        }
        Action::ModelsLoaded(models) => {
            app.set_models(models);
            app.set_chat_list(None)?;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> AppResult<()> {
    let cli = Cli::parse();
    create_db().context("Failed to create database")?;
    migrate_db().context("Failed to migrate database")?;

    // Create an application.
    let maybe_context = cli.read().context("Could not read from file or stdin.")?;

    let system_prompt = if let Some(context) = maybe_context {
        if !context.is_empty() {
            format!(
                r#"
You are a helpful assistant.
Answer the user's query using the provided context.
Context:

{context}
    "#
            )
        } else {
            cli.system_prompt.clone()
        }
    } else {
        cli.system_prompt.clone()
    };
    let mut app = App::new(&system_prompt);

    // Initialize the terminal user interface.
    let backend = CrosstermBackend::new(std::io::stderr());
    let mut terminal = Terminal::new(backend).context("Failed to create terminal")?;

    // Extra initialization.
    enable_raw_mode()?;
    crossterm::execute!(std::io::stderr(), EnterAlternateScreen, EnableMouseCapture)?;
    terminal.hide_cursor()?;
    terminal.clear()?;

    // Find the terminal size.
    app.set_terminal_size(terminal.size()?.width, terminal.size()?.height);

    let events = EventHandler::new(100);
    let (action_tx, mut action_rx) = mpsc::channel(32);
    let mut current_cancel_tx: Option<mpsc::Sender<()>> = None;

    let mut tui = Tui::new(terminal, events);
    tui.init().context("Failed to initialize terminal")?;

    // Start the main loop.
    while app.running {
        // 1. DRAW ONCE PER ITERATION
        tui.draw(&mut app)
            .context("Failed to render user interface")?;

        // 2. WAIT for EITHER a terminal event OR an async action.
        tokio::select! {
            // --- Terminal events ---
            maybe_event = tui.events.next() => {
                let event = maybe_event.context("Unable to get next event")?;
                handle_event(event, &mut app, &mut current_cancel_tx)?;

                // Drain any terminal events that arrived immediately behind it.
                while let Some(Ok(next_event)) = tui.events.next().now_or_never() {
                    handle_event(next_event, &mut app, &mut current_cancel_tx)?;
                }
            }

            // --- Async actions from spawned tasks ---
            Some(action) = action_rx.recv() => {
                handle_action(action, &mut app).await?;

                // Drain any other actions already queued up.
                while let Ok(action) = action_rx.try_recv() {
                    handle_action(action, &mut app).await?;
                }
            }
        }

        // 3. POST-EVENT WORK (runs after either branch wakes us up)

        if app.is_loading_models {
            app.is_loading_models = false;

            let tx = action_tx.clone();
            let ollama_host_url = cli.ollama_host.clone();

            task::spawn(async move {
                match get_models(ollama_host_url.as_deref()).await {
                    Ok(models) => {
                        let _ = tx.send(Action::ModelsLoaded(models)).await;
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Action::Error(format!("Failed to find models: {}", e)))
                            .await;
                    }
                }
            });
        }

        if app.needs_recache {
            app.recache_lines(app.messages.clone());
            app.needs_recache = false;
        }

        if app.has_unprocessed_messages {
            app.has_unprocessed_messages = false;
            app.is_waiting_for_response = true;

            // Clone data needed for the task
            let messages = app.messages.clone();
            let selected_model = app.selected_model_name.clone();
            let thinking_effort = app.thinking_effort.clone();
            let ollama_host_url = cli.ollama_host.clone();
            let sys_prompt = if selected_model.starts_with("gpt") {
                None
            } else {
                Some(system_prompt.clone())
            };

            let tx = action_tx.clone();

            let (cancel_tx, mut cancel_rx) = mpsc::channel::<()>(1);
            current_cancel_tx = Some(cancel_tx);

            // Spawn ONE task that does everything
            task::spawn(async move {
                let response = assistant_response_streaming(
                    &messages,
                    &selected_model,
                    sys_prompt,
                    thinking_effort,
                    ollama_host_url,
                )
                .await;

                match response {
                    Ok(mut stream) => {
                        let mut full_content = String::new();
                        let mut full_thinking_content = String::new();
                        let _ = tx.send(Action::StreamStart).await;

                        loop {
                            tokio::select! {
                                _ = cancel_rx.recv() => {
                                    let all_content = if !full_thinking_content.is_empty() {
                                        format!("<think>\n{}\n</think>\n{}", full_thinking_content, full_content)
                                    } else {
                                        full_content
                                    };
                                    let _ = tx.send(Action::StreamCancelled(all_content)).await;
                                    break;
                                }
                                result_opt = stream.next() => {
                                    match result_opt {
                                        Some(Ok(event)) => {
                                            let mut partial_updated = false;

                                            match event {
                                                ChatStreamEvent::ReasoningChunk(StreamChunk { content }) if !content.is_empty() => {
                                                    full_thinking_content.push_str(&content);
                                                    partial_updated = true;
                                                }
                                                ChatStreamEvent::Chunk(StreamChunk { content }) if !content.is_empty() => {
                                                    full_content.push_str(&content);
                                                    partial_updated = true;
                                                }
                                                ChatStreamEvent::End(StreamEnd {captured_content: Some(content), captured_reasoning_content: reasoning_content, ..}) => {
                                                    if let Some(texts) = content.into_joined_texts() {
                                                        let full = if let Some(reasoning) = reasoning_content {
                                                            format!("<think>\n{}\n</think>\n{}", reasoning, texts)
                                                        } else {
                                                            texts
                                                        };
                                                        let _ = tx.send(Action::StreamComplete(full)).await;
                                                    }
                                                }
                                                _ => {}
                                            }

                                            if partial_updated {
                                                let all_content = if !full_thinking_content.is_empty() {
                                                    format!("<think>\n{}\n</think>\n{}", full_thinking_content, full_content)
                                                } else {
                                                    full_content.clone()
                                                };
                                                let _ = tx.send(Action::StreamPartial(all_content)).await;
                                            }
                                        }
                                        Some(Err(e)) => {
                                            let _ = tx.send(Action::Error(format!("Stream error: {}", e))).await;
                                            break;
                                        }
                                        None => {
                                            let all_content = if !full_thinking_content.is_empty() {
                                                format!("<think>\n{}\n</think>\n{}", full_thinking_content, full_content)
                                            } else {
                                                full_content
                                            };
                                            let _ = tx.send(Action::StreamComplete(all_content)).await;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Action::Error(format!("API Error: {}", e))).await;
                    }
                }
            });
        }
    }

    // Exit the user interface.
    tui.exit().context("Failed during application shutdown")?;
    Ok(())
}
