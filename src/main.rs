use gait::ai::assistant_response;
use gait::app::{App, AppResult};
use gait::event::{Event, EventHandler};
use gait::handler::handle_key_events;
use gait::tui::Tui;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use tokio::sync::mpsc;
use tokio::task;

#[tokio::main]
async fn main() -> AppResult<()> {
    // Create an application.
    let mut app = App::new();

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
            task::spawn(async move {
                let assistant_response = assistant_response(messages, &selected_model_name).await;
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
