use anyhow::Context;
use clap::Parser;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use tokio::sync::mpsc;
use tokio::task;

use ait::ai::{assistant_response, get_models};
use ait::app::{App, AppResult};
use ait::cli::Cli;
use ait::event::{Event, EventHandler};
use ait::handler::handle_key_events;
use ait::storage::create_db;
use ait::tui::Tui;

#[tokio::main]
async fn main() -> AppResult<()> {
    let cli = Cli::parse();
    let temperature = cli.temperature;

    create_db().context("Failed to create database")?;

    // Create an application.
    let mut app = App::new(&cli.system_prompt);
    let models = get_models()
        .await
        .context("Failed to find models from providers")?;
    app.set_models(models);
    app.set_chat_list()?;

    // Initialize the terminal user interface.
    let backend = CrosstermBackend::new(io::stderr());
    let terminal = Terminal::new(backend).context("Failed to create terminal")?;
    let events = EventHandler::new(250);
    let mut tui = Tui::new(terminal, events);
    tui.init().context("Failed to initialize terminal")?;

    // Create a channel to receive the assistant responses
    let (assistant_response_tx, mut assistant_response_rx) = mpsc::channel(32);

    // Start the main loop.
    while app.running {
        // Render the user interface.
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
                handle_key_events(key_event, &mut app).context("Error handling key events")?
            }
            Event::Mouse(_) | Event::Resize(_, _) => {}
        }

        // Check for a new query and spawn a task to handle it
        if app.has_unprocessed_messages {
            app.has_unprocessed_messages = false;
            let assistant_response_tx = assistant_response_tx.clone();
            let messages = app.messages.clone(); // This clone is necessary for the async task
            let selected_model_name = app.selected_model_name.clone(); // This clone is necessary for the async task
            let system_prompt = cli.system_prompt.clone(); // This clone is necessary for the async task
            task::spawn(async move {
                let assistant_response = assistant_response(
                    &messages,
                    &selected_model_name,
                    &system_prompt,
                    &temperature,
                )
                .await;
                let _ = assistant_response_tx.send(assistant_response).await;
            });
        }

        // Check for a response from the assistant and process it
        if let Ok(assistant_response) = assistant_response_rx.try_recv() {
            match assistant_response {
                Ok(response) => app
                    .receive_message(response)
                    .await
                    .context("Error while receiving message")?,
                Err(e) => eprintln!("Error receiving assistant response: {}", e),
            }
        }
    }

    // Exit the user interface.
    tui.exit().context("Failed during application shutdown")?;
    Ok(())
}
