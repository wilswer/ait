use anyhow::Context;
use clap::Parser;
use futures::FutureExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;
use tokio::task;

use ait::ai::{get_models, run_assistant_stream};
use ait::app::{Action, App, AppMode, AppResult, Message, Notification, SpawnArgs, ThinkingEffort};
use ait::cli::Cli;
use ait::config::{Config, ModelConfig};
use ait::event::{Event, EventHandler};
use ait::handler::{handle_key_events, handle_mouse_events};
use ait::python_tools::{PythonToolSource, validate_source_path};
use ait::storage::{create_db, migrate_db};
use ait::tools::ToolRegistry;
use ait::tui::Tui;

/// Build an immutable registry snapshot for one assistant request.
///
/// MCP schemas are fetched from live connections when a request begins. Python
/// schemas were validated at startup and are cached in `App`, so this does not
/// re-import user code. The returned snapshot is moved into the stream task and
/// is stable for every tool-calling round of that request.
async fn build_tool_registry(app: &App<'_>) -> ToolRegistry {
    let mut builder = ToolRegistry::builder();

    for connection in app.mcp_connections.values() {
        match connection.bridge.tools().await {
            Ok(tools) => {
                if let Err(error) = builder.add_mcp_tools(connection.bridge.clone(), tools) {
                    tracing::warn!(
                        mcp.server = %connection.id,
                        error = %error,
                        "skipping conflicting MCP tools while building request registry"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    mcp.server = %connection.id,
                    error = %error,
                    "failed to list MCP tools while building request registry"
                );
            }
        }
    }

    for source in &app.python_tool_sources {
        let Some(definitions) = app.python_tool_definitions.get(&source.id) else {
            continue;
        };
        if let Err(error) = builder.add_python_tools(source.clone(), definitions.clone()) {
            tracing::warn!(
                python.source = %source.id,
                error = %error,
                "skipping conflicting Python tools while building request registry"
            );
        }
    }

    let registry = builder.build();
    tracing::info!(tools = ?registry.names(), "built tool registry for request");
    registry
}

fn handle_event(event: Event, app: &mut App, action_tx: &mpsc::Sender<Action>) -> AppResult<()> {
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
            app.receive_incomplete_message(conversation_id, &content)
                .await?;
        }
        Action::StreamComplete {
            conversation_id,
            content,
            raw_messages,
        } => {
            app.receive_message(
                conversation_id,
                Message::Assistant(content, None, None, raw_messages),
            )
            .await?;
        }
        Action::StreamCancelled {
            conversation_id,
            content,
            raw_messages,
        } => {
            // Persist whatever portion of the message was generated before
            // stopping.
            app.receive_message(
                conversation_id,
                Message::Assistant(content, None, None, raw_messages),
            )
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
        // channel). `McpEnableRequested` is handled in the main loop where it
        // has access to the config. These arms should never fire.
        Action::McpServerReady { .. }
        | Action::McpServerFailed { .. }
        | Action::McpEnableRequested { .. } => {}
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

    let default_thinking_level = config
        .default_thinking_level
        .unwrap_or(ThinkingEffort::Medium);

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

    let mut app = App::new(&system_prompt, default_model, default_thinking_level);

    // Phase 1 Python tools are explicitly opt-in through repeated
    // `--python-tools <file>` arguments. Validate and discover them before
    // entering the alternate-screen TUI so failures remain terminal-safe.
    for (index, script) in cli.python_tools.iter().enumerate() {
        validate_source_path(script)
            .with_context(|| format!("Invalid Python tool source `{}`", script.display()))?;

        let mut source = PythonToolSource::new(format!("python-{}", index + 1), script.clone());
        source.display_name = script
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| script.display().to_string());

        tracing::info!(path = %source.script.display(), "discovering Python tools");
        let definitions = source.discover().await.map_err(|error| {
            anyhow::anyhow!(
                "Failed to load Python tools from `{}`: {error}",
                source.script.display()
            )
        })?;
        tracing::info!(
            path = %source.script.display(),
            tools = definitions.len(),
            "loaded Python tools"
        );
        app.python_tool_definitions
            .insert(source.id.clone(), definitions);
        app.python_tool_sources.push(source);
    }

    // Seed MCP status (one `Connecting` entry per enabled server) so the
    // footer shows "connecting" immediately, before any server resolves.
    app.mcp_statuses = ait::mcp::initial_statuses(&config.mcp);
    // Seed user intent from config: every `enabled = true` server is on.
    for (id, cfg) in &config.mcp.servers {
        if cfg.enabled {
            app.mcp_enabled.insert(id.clone());
        }
    }

    // Connect to MCP servers declared (and enabled) in the config. This runs
    // in the background so a slow/hanging server never blocks the TUI; each
    // server's outcome (ready/failed) is delivered to the main loop, updating
    // the footer and the tool bridge list. A clone of the config is kept for
    // on-demand reconnects when the user re-enables a server.
    let mcp_config = config.mcp.clone();
    let (mcp_tx, mut mcp_rx) = mpsc::channel::<ait::mcp::McpServerOutcome>(16);
    let mcp_enable_tx = mcp_tx.clone();
    let mcp_config_for_connect = mcp_config.clone();
    task::spawn(async move {
        ait::mcp::connect_all_streaming(&mcp_config_for_connect, mcp_tx).await;
    });

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
                // `McpEnableRequested` needs the config + mcp_tx, which live in
                // this scope, so handle it here before `handle_action`.
                if let Action::McpEnableRequested { ref id } = action {
                    let Some(server_cfg) = mcp_config.servers.get(id).cloned() else {
                        tracing::warn!(mcp.server = %id, "McpEnableRequested for unknown server id");
                        continue;
                    };
                    let mcp_tx = mcp_enable_tx.clone();
                    let id = id.clone();
                    task::spawn(async move {
                        let outcome = match ait::mcp::connect_one(id.clone(), server_cfg).await {
                            Ok(conn) => ait::mcp::McpServerOutcome::Ready(Box::new(conn)),
                            Err(e) => {
                                tracing::warn!(mcp.server = %id, error = %e, "failed to connect MCP server");
                                ait::mcp::McpServerOutcome::Failed { id, error: e.to_string() }
                            }
                        };
                        let _ = mcp_tx.send(outcome).await;
                    });
                    continue;
                }
                handle_action(action, &mut app).await?;

                // Drain any other actions already queued up.
                while let Ok(action) = action_rx.try_recv() {
                    if let Action::McpEnableRequested { ref id } = action {
                        let Some(server_cfg) = mcp_config.servers.get(id).cloned() else {
                            continue;
                        };
                        let mcp_tx = mcp_enable_tx.clone();
                        let id = id.clone();
                        task::spawn(async move {
                            let outcome = match ait::mcp::connect_one(id.clone(), server_cfg).await {
                                Ok(conn) => ait::mcp::McpServerOutcome::Ready(Box::new(conn)),
                                Err(e) => {
                                    tracing::warn!(mcp.server = %id, error = %e, "failed to connect MCP server");
                                    ait::mcp::McpServerOutcome::Failed { id, error: e.to_string() }
                                }
                            };
                            let _ = mcp_tx.send(outcome).await;
                        });
                        continue;
                    }
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
                        // Only accept the connection if the user still wants
                        // this server on (they may have disabled it while it
                        // was connecting).
                        if !app.mcp_is_enabled(&id) {
                            tracing::info!(
                                mcp.server = %id,
                                "dropping connection for disabled server"
                            );
                            // Dropping `conn` cancels the service.
                            continue;
                        }
                        let tool_count = conn.tool_count().await;
                        app.mcp_server_ready(conn, tool_count);
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

        // Refresh the streaming format cache for the viewed conversation (if
        // it is actively streaming). Throttled internally to avoid
        // re-parsing on every single token.
        // If the cache was updated and the user is following the stream,
        // re-scroll to the bottom so the view tracks the new content.
        if app.refresh_streaming_format() && app.needs_stream_scroll {
            app.scroll_to_bottom()
                .context("Could not scroll to bottom after streaming format refresh")?;
            app.needs_stream_scroll = false;
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
            // Build a fresh immutable snapshot from active MCP connections
            // plus the cached, validated Python definitions. This lookup is
            // performed before spawning so the stream cannot observe a later
            // source change mid-tool-loop.
            let tool_registry = build_tool_registry(&app).await;

            // Spawn ONE task per conversation that drives the full streaming
            // + MCP tool-calling loop, reporting progress via `tx`.
            task::spawn(async move {
                if let Err(e) = run_assistant_stream(
                    &messages,
                    selected_model,
                    sys_prompt,
                    thinking_effort,
                    ollama_host_url,
                    tool_registry,
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
