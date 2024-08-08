use crate::app::{App, AppResult};
use crate::event::EventHandler;
use crate::ui;
use anyhow::Context;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
#[cfg(not(target_os = "windows"))]
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::Backend;
use ratatui::Terminal;
use std::io;
use std::panic;

/// Representation of a terminal user interface.
///
/// It is responsible for setting up the terminal,
/// initializing the interface and handling the draw events.
#[derive(Debug)]
pub struct Tui<B: Backend> {
    /// Interface to the Terminal.
    terminal: Terminal<B>,
    /// Terminal event handler.
    pub events: EventHandler,
}

impl<B: Backend> Tui<B> {
    /// Constructs a new instance of [`Tui`].
    pub fn new(terminal: Terminal<B>, events: EventHandler) -> Self {
        Self { terminal, events }
    }

    /// Initializes the terminal interface.
    ///
    /// It enables the raw mode and sets terminal properties.
    pub fn init(&mut self) -> AppResult<()> {
        terminal::enable_raw_mode().context("Could not enable raw mode")?;
        #[cfg(not(target_os = "windows"))]
        crossterm::execute!(
            io::stderr(),
            EnterAlternateScreen,
            EnableMouseCapture,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )
        .context("Could not initialize terminal, error in `crossterm::execute!`")?;

        #[cfg(target_os = "windows")]
        crossterm::execute!(io::stderr(), EnterAlternateScreen, EnableMouseCapture)
            .context("Could not initialize terminal, error in `crossterm::execute!`")?;

        // Define a custom panic hook to reset the terminal properties.
        // This way, you won't have your terminal messed up if an unexpected error happens.
        let panic_hook = panic::take_hook();
        panic::set_hook(Box::new(move |panic| {
            Self::reset().expect("failed to reset the terminal");
            panic_hook(panic);
        }));

        self.terminal
            .hide_cursor()
            .context("Error when hiding terminal cursor")?;
        self.terminal.clear().context("Could not clear terminal")?;
        Ok(())
    }

    /// [`Draw`] the terminal interface by [`rendering`] the widgets.
    ///
    /// [`Draw`]: ratatui::Terminal::draw
    /// [`rendering`]: crate::ui::render
    pub fn draw(&mut self, app: &mut App) -> AppResult<()> {
        self.terminal
            .draw(|frame| ui::render(frame, app))
            .context("Failed to render the user interface")?;
        Ok(())
    }

    /// Resets the terminal interface.
    ///
    /// This function is also used for the panic hook to revert
    /// the terminal properties if unexpected errors occur.
    fn reset() -> AppResult<()> {
        terminal::disable_raw_mode().context("Failed to disable raw mode")?;
        #[cfg(not(target_os = "windows"))]
        crossterm::execute!(
            io::stderr(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            PopKeyboardEnhancementFlags
        )
        .context("Failed resetting terminal, error during `crossterm::execute!`")?;
        #[cfg(target_os = "windows")]
        crossterm::execute!(io::stderr(), LeaveAlternateScreen, DisableMouseCapture)
            .context("Failed resetting terminal, error during `crossterm::execute!`")?;
        Ok(())
    }

    /// Exits the terminal interface.
    ///
    /// It disables the raw mode and reverts back the terminal properties.
    pub fn exit(&mut self) -> AppResult<()> {
        Self::reset().context("Failed to reset terminal")?;
        self.terminal
            .show_cursor()
            .context("Failed to show cursor")?;
        Ok(())
    }
}
