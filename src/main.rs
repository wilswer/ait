use anyhow::Context;
use clap::Parser;
use futures::FutureExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;
use tokio::task;

use ait::ai::{get_models, run_assistant_stream};
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
            app.receive_message(conversation_id, Message::Assistant(content, None, None))
                .await?;
        }
        Action::StreamCancelled {
            conversation_id,
            content,
        } => {
            // Persist whatever portion of the message was generated before
            // stopping.
            app.receive_message(conversation_id, Message::Assistant(content, None, None))
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
        // MCP outcomes are consumed directly in the main `tokio::select!`
        // (they own a `McpConnection` that can't go through the action
        // channel). These arms should never fire; if they do, ignore them.
        Action::McpServerReady { .. } | Action::McpServerFailed { .. } => {}
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

    // Seed MCP status (one `Connecting` entry per enabled server) so the
    // footer shows "connecting" immediately, before any server resolves.
    app.mcp_statuses = ait::mcp::initial_statuses(&config.mcp);

    // Connect to MCP servers declared (and enabled) in the config. This runs
    // in the background so a slow/hanging server never blocks the TUI; each
    // server's outcome (ready/failed) is delivered to the main loop as a
    // dedicated action, updating the footer and the tool bridge list.
    let mcp_config = config.mcp.clone();
    let (mcp_tx, mut mcp_rx) =
        mpsc::channel::<ait::mcp::McpServerOutcome>(16);
    task::spawn(async move {
        ait::mcp::connect_all_streaming(&mcp_config, mcp_tx).await;
    });
    // Connections live here so their `RunningService`s stay alive for the
    // session; only the cheap `McpToolBridge` clones flow into `App`.
    let mut mcp_keepalive: Vec<ait::mcp::McpConnection> = Vec::new();

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

            // --- MCP server outcomes coming online ---
            Some(outcome) = mcp_rx.recv() => {
                use ait::mcp::McpServerOutcome;
                match outcome {
                    McpServerOutcome::Ready(conn) => {
                        let conn = *conn;
                        let id = conn.id.clone();
                        let display_name = conn.display_name.clone();
                        let bridge = conn.bridge.clone();
                        let tool_count = conn.tool_count().await;
                        app.mcp_server_ready(id, display_name, tool_count, bridge);
                        // Keep the connection (and its `RunningService`) alive
                        // for the whole session.
                        mcp_keepalive.push(conn);
                    }
                    McpServerOutcome::Failed { id, error } => {
                        // Look up the display name we seeded earlier.
                        let display_name = app
                            .mcp_statuses
                            .iter()
                            .find(|s| s.id() == id)
                            .map(|s| s.display_name().to_string())
                            .unwrap_or_else(|| id.clone());
                        app.mcp_server_failed(id, display_name, error);
                    }
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
                cancel_rx,
            } = args;
            let ollama_host_url = resolved_ollama_host.clone();
            let tx = action_tx.clone();
            // Snapshot the currently-connected MCP bridges so the spawned
            // task can resolve and execute tools. `McpToolBridge` is cheap to
            // clone (it holds a channel handle to the running service).
            let mcp_bridges: Vec<mcp_genai_glue::McpToolBridge> =
                app.mcp_bridges.clone();

            // Spawn ONE task per conversation that drives the full streaming
            // + MCP tool-calling loop, reporting progress via `tx`.
            task::spawn(async move {
                if let Err(e) = run_assistant_stream(
                    &messages,
                    selected_model,
                    sys_prompt,
                    thinking_effort,
                    ollama_host_url,
                    mcp_bridges,
                    conversation_id,
                    tx,
                    cancel_rx,
                )
                .await
                {
                    tracing::error!(error = %e, "assistant stream task failed");
                }
            });
        }
    }

    // Exit the user interface.
    tui.exit().context("Failed during application shutdown")?;
    Ok(())
}
