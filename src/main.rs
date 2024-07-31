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
use ait::tui::Tui;

#[tokio::main]
async fn main() -> AppResult<()> {
    let cli = Cli::parse();
    let system_prompt = if let Some(system_prompt) = cli.system_prompt {
        system_prompt
    } else {
        "You are a helpful and friendly assistant.".to_string()
    };

    // Create an application.
    let mut app = App::new();
    let models = get_models().await?;
    app.set_models(models);

    // Initialize the terminal user interface.
    let backend = CrosstermBackend::new(io::stderr());
    let terminal = Terminal::new(backend)?;
    let events = EventHandler::new(250);
    let mut tui = Tui::new(terminal, events);
    tui.init()?;

    // Create a channel to receive the assistant responses
    let (assistant_response_tx, mut assistant_response_rx) = mpsc::channel(32);

    // Start the main loop.
    while app.running {
        // Render the user interface.
        tui.draw(&mut app)?;
        // Handle events.
        match tui.events.next().await? {
            Event::Tick => app.tick(),
            Event::Key(key_event) => handle_key_events(key_event, &mut app)?,
            Event::Mouse(_) => {}
            Event::Resize(_, _) => {}
        }

        // Check for a new query and spawn a task to handle it
        if app.current_message.is_some() {
            app.current_message = None;
            let assistant_response_tx = assistant_response_tx.clone();
            let messages = app.messages.clone();
            let selected_model_name = app.selected_model_name.clone();
            let system_prompt = system_prompt.clone();
            task::spawn(async move {
                let assistant_response =
                    assistant_response(&messages, &selected_model_name, &system_prompt).await;
                let _ = assistant_response_tx.send(assistant_response).await;
            });
        }

        // Check for a response from the assistant and process it
        if let Ok(assistant_response) = assistant_response_rx.try_recv() {
            match assistant_response {
                Ok(response) => app.receive_message(response).await?,
                Err(e) => eprintln!("Error receiving assistant response: {}", e),
            }
        }
    }

    // Exit the user interface.
    tui.exit()?;
    Ok(())
}
