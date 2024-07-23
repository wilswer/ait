use gait::ai::bot_response;
use gait::app::{App, AppResult};
use gait::event::{Event, EventHandler};
use gait::handler::handle_key_events;
use gait::tui::Tui;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;

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
        if let Some(query) = app.current_message.clone() {
            app.current_message = None;
            let bot_response = bot_response(query).await?;
            app.receive_message(bot_response).await;
        }
    }

    // Exit the user interface.
    tui.exit()?;
    Ok(())
}
