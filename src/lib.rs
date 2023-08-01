use std::io::stdout;
use std::sync::Arc;
use std::time::Duration;

use crate::app::ui;
use crate::teams::app_handler::AppCmdEvent;
use app::{App, AppReturn};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use eyre::Result;
use inputs::events::Events;
use inputs::key::Key;
use inputs::InputEvent;
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

pub mod app;
// pub mod banner;
pub mod inputs;
pub mod teams;

pub async fn start_ui(app: &Arc<tokio::sync::Mutex<App<'_>>>) -> Result<()> {
    // Configure Crossterm backend for tui
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // User event handler
    let tick_rate = Duration::from_millis(200);
    let mut events = Events::new(tick_rate);

    {
        let mut app = app.lock().await;
        app.dispatch_to_teams(AppCmdEvent::Initialize()).await;
    }

    loop {
        let mut app = app.lock().await;

        // Render
        terminal.draw(|rect| ui::draw(rect, &app))?;

        // Handle terminal inputs
        let result = match events.next().await {
            InputEvent::Input(key_event) if app.is_editing() => {
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

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
