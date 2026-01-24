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
use ait::storage::create_db;
use ait::tui::Tui;

#[derive(Debug, Clone)]
pub enum Action {
    StreamStart,
    StreamPartial(String),
    StreamComplete(String),
    Error(String),
}

#[tokio::main]
async fn main() -> AppResult<()> {
    let cli = Cli::parse();
    let temperature = cli.temperature;
    create_db().context("Failed to create database")?;

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

            // Clone data needed for the task
            let messages = app.messages.clone();
            let selected_model = app.selected_model_name.clone();
            let (sys_prompt, temp) =
                if selected_model.starts_with("o1") || selected_model.starts_with("o3") {
                    (None, None)
                } else {
                    (Some(system_prompt.clone()), Some(temperature))
                };

            // Clone the single action channel
            let tx = action_tx.clone();

            // Spawn ONE task that does everything (Call API + Process Stream)
            task::spawn(async move {
                // A. Call the API
                let response =
                    assistant_response_streaming(&messages, &selected_model, sys_prompt, temp)
                        .await;

                match response {
                    Ok(mut stream) => {
                        // B. Process the stream immediately here
                        let mut full_content = String::new();

                        // Notify main thread we started
                        let _ = tx.send(Action::StreamStart).await;

                        while let Some(result) = stream.next().await {
                            match result {
                                Ok(event) => match event {
                                    ChatStreamEvent::Start => {} // Already handled
                                    ChatStreamEvent::Chunk(chunk)
                                    | ChatStreamEvent::ReasoningChunk(chunk) => {
                                        if !chunk.content.is_empty() {
                                            full_content.push_str(&chunk.content);
                                            // Send partial update
                                            let _ = tx
                                                .send(Action::StreamPartial(full_content.clone()))
                                                .await;
                                        }
                                    }
                                    ChatStreamEvent::End(_) => {}
                                    _ => {} // Ignore others
                                },
                                Err(e) => {
                                    let _ = tx
                                        .send(Action::Error(format!("Stream error: {}", e)))
                                        .await;
                                    return; // Stop processing
                                }
                            }
                        }
                        // Send final complete message
                        let _ = tx.send(Action::StreamComplete(full_content)).await;
                    }
                    Err(e) => {
                        // API Connection failed
                        let _ = tx.send(Action::Error(format!("API Error: {}", e))).await;
                    }
                }
            });
        }

        // --- 2. HANDLING THE RESULTS ---
        // We drain the channel so we process all pending updates in one tick
        while let Ok(action) = action_rx.try_recv() {
            match action {
                Action::StreamStart => {
                    app.is_streaming = true;
                    // Optional: clear previous incomplete buffer if needed
                    app.receive_incomplete_message("").await?;
                }
                Action::StreamPartial(content) => {
                    app.receive_incomplete_message(&content).await?;
                }
                Action::StreamComplete(content) => {
                    app.is_streaming = false;
                    app.receive_message(Message::Assistant(content)).await?;
                }
                Action::Error(err_msg) => {
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
