use anyhow::Context;
use clap::Parser;
use futures::{FutureExt, StreamExt};
use genai::chat::{ChatStreamEvent, StreamChunk, StreamEnd};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;
use tokio::task;

use ait::ai::{assistant_response_streaming, get_models};
use ait::app::{Action, App, AppMode, AppResult, Message, Notification, SpawnArgs};
use ait::cli::Cli;
use ait::config::{Config, ModelConfig};
use ait::event::{Event, EventHandler};
use ait::handler::{handle_key_events, handle_mouse_events};
use ait::storage::{create_db, migrate_db};
use ait::tui::Tui;

/// Handle a single terminal event (key/mouse/tick/resize).
fn handle_event(
    event: Event,
    app: &mut App,
    action_tx: &mpsc::Sender<Action>,
) -> AppResult<()> {
    match event {
        Event::Tick => app.tick(),
        Event::Key(key_event) => {
            if key_event.code == crossterm::event::KeyCode::Char('u')
                && app.app_mode == AppMode::Normal
            {
                // Cancel the in-flight stream for the currently viewed
                // conversation, if any.
                app.cancel_current_stream();
            }
            handle_key_events(key_event, app, action_tx).context("Error handling key events")?;
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
        Action::StreamStart { conversation_id } => {
            // Only seed the in-memory partial message for the view when the
            // user is still on the conversation that owns the stream.
            if app.conversation_id == Some(conversation_id) {
                app.receive_incomplete_message(conversation_id, "").await?;
            }
        }
        Action::StreamPartial {
            conversation_id,
            content,
        } => {
            app.receive_incomplete_message(conversation_id, &content).await?;
        }
        Action::StreamComplete {
            conversation_id,
            content,
        } => {
            app.receive_message(conversation_id, Message::Assistant(content))
                .await?;
        }
        Action::StreamCancelled {
            conversation_id,
            content,
        } => {
            // Persist whatever portion of the message was generated before
            // stopping.
            app.receive_message(conversation_id, Message::Assistant(content))
                .await?;
        }
        Action::Error {
            conversation_id,
            message,
        } => {
            // Drop the in-flight state for this conversation (if any).
            if let Some(id) = conversation_id {
                app.streams.remove(&id);
            }
            // Surface the error notification when it either isn't tied to a
            // specific conversation (e.g. model discovery) or the user is
            // looking at the conversation it belongs to. Background stream
            // failures don't interrupt the current view.
            let show_notification = match conversation_id {
                None => true,
                Some(id) => app.conversation_id == Some(id),
            };
            if show_notification {
                app.set_app_mode(AppMode::Notify {
                    notification: Notification::Error(message),
                });
            } else {
                tracing::warn!(
                    ?conversation_id,
                    "background stream errored while viewing a different chat"
                );
            }
        }
        Action::ModelsLoaded(models) => {
            app.set_models(models);
            app.set_chat_list(None)?;
        }
        Action::ContextFileAdded { file, est_tokens } => {
            app.add_to_context(file, est_tokens);
        }
        Action::ContextAddDone { notification } => {
            app.set_app_mode(AppMode::Notify { notification });
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> AppResult<()> {
    let cli = Cli::parse();

    // Load configuration (falls back to defaults if not found)
    let config = Config::load().unwrap_or_default();

    // Read context (file or stdin)
    let maybe_context = cli.read().context("Could not read from file or stdin.")?;

    // Resolve system prompt: CLI > Config > Default
    let system_prompt = if let Some(context) = maybe_context {
        if !context.is_empty() {
            format!(
                r#"
You are a helpful, friendly assistant.
Answer the user's query using the provided context.
Context:

{context}
    "#
            )
        } else {
            cli.system_prompt
                .or(config.system_prompt)
                .unwrap_or_else(|| "You are a helpful, friendly assistant.".to_string())
        }
    } else {
        cli.system_prompt
            .or(config.system_prompt)
            .unwrap_or_else(|| "You are a helpful, friendly assistant.".to_string())
    };

    // Resolve default model: Config > Default
    let default_model = config.default_model.unwrap_or_else(|| {
        ModelConfig::new("gemini-3.1-pro-preview".to_string(), "Gemini".to_string())
    });

    // Resolve Ollama host: CLI > Config > Default
    let resolved_ollama_host = cli
        .ollama_host
        .or(config.ollama_host)
        .or_else(|| Some("http://localhost:11434/".to_string()));

    create_db().context("Failed to create database")?;
    migrate_db().context("Failed to migrate database")?;

    // Initialize file-based logging. The guard must be held until shutdown so
    // the background writer is flushed; if initialization fails logging is
    // simply disabled and the app continues.
    let _log_guard = ait::logger::init_logging();

    let mut app = App::new(&system_prompt, default_model);

    // Initialize the terminal user interface.
    let backend = CrosstermBackend::new(std::io::stderr());
    let terminal = Terminal::new(backend).context("Failed to create terminal")?;

    // Find the terminal size.
    app.set_terminal_size(terminal.size()?.width, terminal.size()?.height);

    let events = EventHandler::new(16);
    let (action_tx, mut action_rx) = mpsc::channel(32);

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
                handle_event(event, &mut app, &action_tx)?;

                // Drain any terminal events that arrived immediately behind it.
                while let Some(Ok(next_event)) = tui.events.next().now_or_never() {
                    handle_event(next_event, &mut app, &action_tx)?;
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
            let ollama_host_url = resolved_ollama_host.clone();

            task::spawn(async move {
                match get_models(ollama_host_url.as_deref()).await {
                    Ok(models) => {
                        let _ = tx.send(Action::ModelsLoaded(models)).await;
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Action::Error {
                                conversation_id: None,
                                message: format!("Failed to find models: {}", e),
                            })
                            .await;
                    }
                }
            });
        }

        if app.needs_recache {
            app.recache_lines(app.messages.clone());
            app.needs_recache = false;
        }

        // Launch streaming tasks for any conversations that have been
        // submitted but not yet spawned. Scanning every iteration is cheap
        // (the map is small and bounded by `MAX_CONCURRENT_STREAMS`).
        for args in app.take_pending_spawns() {
            let SpawnArgs {
                conversation_id,
                messages,
                selected_model,
                thinking_effort,
                system_prompt: sys_prompt,
                mut cancel_rx,
            } = args;
            let ollama_host_url = resolved_ollama_host.clone();
            let tx = action_tx.clone();

            // Spawn ONE task per conversation that does everything.
            task::spawn(async move {
                let response = assistant_response_streaming(
                    &messages,
                    selected_model,
                    sys_prompt,
                    thinking_effort,
                    ollama_host_url,
                )
                .await;

                match response {
                    Ok(mut stream) => {
                        let mut full_content = String::new();
                        let mut full_thinking_content = String::new();
                        let _ = tx
                            .send(Action::StreamStart { conversation_id })
                            .await;

                        loop {
                            tokio::select! {
                                _ = cancel_rx.recv() => {
                                    let all_content = if !full_thinking_content.is_empty() {
                                        format!("<think>\n{}\n</think>\n{}", full_thinking_content, full_content)
                                    } else {
                                        full_content
                                    };
                                    let _ = tx.send(Action::StreamCancelled { conversation_id, content: all_content }).await;
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
                                                ChatStreamEvent::End(StreamEnd {captured_content: Some(content), captured_reasoning_content: reasoning_content, captured_usage: usage, ..}) => {
                                                    // Log token usage
                                                    if let Some(u) = &usage {
                                                        let prompt = u.prompt_tokens.unwrap_or(0);
                                                        let completion = u.completion_tokens.unwrap_or(0);
                                                        let total = u.total_tokens.unwrap_or(0);
                                                        let cached = u
                                                            .prompt_tokens_details
                                                            .as_ref()
                                                            .and_then(|d| d.cached_tokens)
                                                            .unwrap_or(0);
                                                        let cache_creation = u
                                                            .prompt_tokens_details
                                                            .as_ref()
                                                            .and_then(|d| d.cache_creation_tokens)
                                                            .unwrap_or(0);

                                                        tracing::info!(
                                                            prompt_tokens = prompt,
                                                            completion_tokens = completion,
                                                            total_tokens = total,
                                                            cached_tokens = cached,
                                                            cache_creation_tokens = cache_creation,
                                                            "stream completed - token usage"
                                                        );
                                                    } else {
                                                        tracing::info!("stream completed - no token usage returned");
                                                    }
                                                    if let Some(texts) = content.into_joined_texts() {
                                                        let full = if let Some(reasoning) = reasoning_content {
                                                            format!("<think>\n{}\n</think>\n{}", reasoning, texts)
                                                        } else {
                                                            texts
                                                        };
                                                        let _ = tx.send(Action::StreamComplete { conversation_id, content: full }).await;
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
                                                let _ = tx.send(Action::StreamPartial { conversation_id, content: all_content }).await;
                                            }
                                        }
                                        Some(Err(e)) => {
                                            let _ = tx.send(Action::Error { conversation_id: Some(conversation_id), message: format!("Stream error: {}", e) }).await;
                                            break;
                                        }
                                        None => {
                                            let all_content = if !full_thinking_content.is_empty() {
                                                format!("<think>\n{}\n</think>\n{}", full_thinking_content, full_content)
                                            } else {
                                                full_content
                                            };
                                            let _ = tx.send(Action::StreamComplete { conversation_id, content: all_content }).await;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Action::Error { conversation_id: Some(conversation_id), message: format!("API Error: {}", e) }).await;
                    }
                }
            });
        }
    }

    // Exit the user interface.
    tui.exit().context("Failed during application shutdown")?;
    Ok(())
}
