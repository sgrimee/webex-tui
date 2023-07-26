use std::io::stdout;
use std::sync::Arc;
use std::time::Duration;

use app::{App, AppReturn};
use eyre::Result;
use inputs::events::Events;
use inputs::key::Key;
use inputs::InputEvent;
use crate::teams::app_handler::IoEvent;
// use log::debug;
use tui::backend::CrosstermBackend;
use tui::Terminal;

use crate::app::ui;

pub mod app;
pub mod inputs;
pub mod teams;

pub async fn start_ui(app: &Arc<tokio::sync::Mutex<App<'_>>>) -> Result<()> {
    // Configure Crossterm backend for tui
    let stdout = stdout();
    crossterm::terminal::enable_raw_mode()?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    terminal.hide_cursor()?;

    // User event handler
    let tick_rate = Duration::from_millis(200);
    let mut events = Events::new(tick_rate);

    {
        let mut app = app.lock().await;
        app.dispatch(IoEvent::Initialize).await;
    }

    loop {
        let mut app = app.lock().await;

        // Render
        terminal.draw(|rect| ui::draw(rect, &app))?;

        // Handle terminal inputs
        let result = match events.next().await {
            InputEvent::Input(key_event) if app.state().is_editing() => {
                // debug!("Keyevent: {:#?}", key_event);
                app.process_editing_key(key_event).await
            }
            InputEvent::Input(key_event) => app.do_action(Key::from(key_event)).await,
            InputEvent::Tick => app.update_on_tick().await,
        };
        if result == AppReturn::Exit {
            events.close();
            break;
        }
    }

    terminal.clear()?;
    terminal.show_cursor()?;
    crossterm::terminal::disable_raw_mode()?;

    Ok(())
}
