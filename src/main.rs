use anyhow::Context;
use clap::Parser;
use futures::StreamExt;
use genai::chat::ChatStreamEvent;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
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
pub enum Action {
    StreamStart,
    StreamPartial(String),
    StreamComplete(String),
    StreamCancelled(String),
    Error(String),
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
    let models = get_models()
        .await
        .context("Failed to find models from providers")?;
    app.set_models(models);
    app.set_chat_list()?;

    // Initialize the terminal user interface.
    let backend = CrosstermBackend::new(std::io::stderr());
    let terminal = Terminal::new(backend).context("Failed to create terminal")?;
    app.set_terminal_size(terminal.size()?.width, terminal.size()?.height);
    let events = EventHandler::new(50);
    let mut tui = Tui::new(terminal, events);
    tui.init().context("Failed to initialize terminal")?;

    let (action_tx, mut action_rx) = mpsc::channel(32);

    let mut current_cancel_tx: Option<mpsc::Sender<()>> = None;

    // Start the main loop.
    while app.running {
        tui.draw(&mut app)
            .context("Failed to render user interface")?;

        // Handle events.
        match tui
            .events
            .next()
            .await
            .context("Unable to get next event")?
        {
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

                handle_key_events(key_event, &mut app).context("Error handling key events")?;
            }
            Event::Mouse(mouse_event) => {
                handle_mouse_events(mouse_event, &mut app)?;
            }
            Event::Resize(x, y) => {
                app.set_terminal_size(x, y);
                app.recache_lines(app.messages.clone());
            }
        }

        if app.has_unprocessed_messages {
            app.has_unprocessed_messages = false;
            app.is_waiting_for_response = true;

            // Clone data needed for the task
            let messages = app.messages.clone();
            let selected_model = app.selected_model_name.clone();
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
                let response =
                    assistant_response_streaming(&messages, &selected_model, sys_prompt).await;

                match response {
                    Ok(mut stream) => {
                        let mut full_content = String::new();
                        let _ = tx.send(Action::StreamStart).await;

                        // Use tokio::select! to listen for chunks OR a cancellation signal
                        loop {
                            tokio::select! {
                                // Listens for our cancel signal
                                _ = cancel_rx.recv() => {
                                    let _ = tx.send(Action::StreamCancelled(full_content)).await;
                                    break;
                                }
                                // Listens for the next chunk from the AI
                                result_opt = stream.next() => {
                                    match result_opt {
                                        Some(Ok(event)) => match event {
                                            ChatStreamEvent::Start => {}
                                            ChatStreamEvent::Chunk(chunk) | ChatStreamEvent::ReasoningChunk(chunk) => {
                                                if !chunk.content.is_empty() {
                                                    full_content.push_str(&chunk.content);
                                                    let _ = tx.send(Action::StreamPartial(full_content.clone())).await;
                                                }
                                            }
                                            ChatStreamEvent::End(_) => {}
                                            _ => {}
                                        },
                                        Some(Err(e)) => {
                                            let _ = tx.send(Action::Error(format!("Stream error: {}", e))).await;
                                            break;
                                        }
                                        None => {
                                            // Stream is finished naturally
                                            let _ = tx.send(Action::StreamComplete(full_content)).await;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        // API Connection failed
                        let _ = tx.send(Action::Error(format!("API Error: {}", e))).await;
                    }
                }
            });
        }

        // --- 2. HANDLING THE RESULTS ---
        while let Ok(action) = action_rx.try_recv() {
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
                // 6. Handle the StreamCancelled action
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
            }
        }
    }
    // Exit the user interface.
    tui.exit().context("Failed during application shutdown")?;
    Ok(())
}
