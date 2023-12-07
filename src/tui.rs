// tui.rs

use color_eyre::eyre::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::*;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::time::Duration;
use std::{io, panic};

use crate::app::App;
use crate::inputs::handler::EventHandler;
use crate::ui::render;

pub(crate) type CrosstermTerminal =
    ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stderr>>;

const BACKEND_TICK_TIME_MS: u64 = 200;
/// Representation of a terminal user interface.
///
/// It is responsible for setting up the terminal,
/// initializing the interface and handling the draw events.
pub(crate) struct Tui {
    /// Interface to the Terminal.
    terminal: CrosstermTerminal,
    /// Terminal event handler.
    pub(crate) events: EventHandler,
}

impl Tui {
    /// Constructs a new instance of [`Tui`].
    pub(crate) fn new(terminal: CrosstermTerminal, events: EventHandler) -> Self {
        Self { terminal, events }
    }

    /// Constructs a new instance of [`Tui`] with Crossterm backend and
    /// BACKEND_TICK_TIME_MS tick time
    pub(crate) fn default() -> Result<Self> {
        let backend = CrosstermBackend::new(io::stderr());
        let terminal = Terminal::new(backend)?;
        let events = EventHandler::new(Duration::from_millis(BACKEND_TICK_TIME_MS));
        Ok(Tui::new(terminal, events))
    }

    /// Initializes the terminal interface.
    ///
    /// It enables the raw mode and sets terminal properties.
    pub(crate) fn init(&mut self) -> Result<()> {
        enable_raw_mode()?;
        execute!(io::stderr(), EnterAlternateScreen, EnableMouseCapture)?;

        // Define a custom panic hook to reset the terminal properties.
        // This way, you won't have your terminal messed up if an unexpected error happens.
        let panic_hook = panic::take_hook();
        panic::set_hook(Box::new(move |panic| {
            Self::reset().expect("failed to reset the terminal");
            panic_hook(panic);
        }));

        self.terminal.hide_cursor()?;
        self.terminal.clear()?;
        // A little marker to separate a new run from the previous one
        info!("=========================================================================");
        Ok(())
    }

    /// Draws the terminal interface by [`rendering`] the widgets.
    ///
    /// [`rendering`]: render
    pub(crate) fn draw(&mut self, app: &mut tokio::sync::MutexGuard<App>) -> Result<()> {
        self.terminal.draw(|rect| render(rect, &mut app.state))?;
        Ok(())
    }

    /// Resets the terminal interface.
    ///
    /// This function is also used for the panic hook to revert
    /// the terminal properties if unexpected errors occur.
    fn reset() -> Result<()> {
        disable_raw_mode()?;
        execute!(io::stderr(), LeaveAlternateScreen, DisableMouseCapture)?;
        Ok(())
    }

    /// Exits the terminal interface.
    ///
    /// It disables the raw mode and reverts back the terminal properties.
    pub(crate) fn exit(&mut self) -> Result<()> {
        Self::reset()?;
        self.terminal.show_cursor()?;
        Ok(())
    }
}
