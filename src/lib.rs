use std::sync::Arc;
use std::time::Duration;

use crate::app::ui;
use crate::teams::app_handler::AppCmdEvent;
use app::{App, AppReturn};
use color_eyre::eyre::Result;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use inputs::events::Events;
use inputs::key::Key;
use inputs::InputEvent;
use log::*;
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

pub mod app;
// pub mod banner;
pub mod inputs;
pub mod teams;

fn startup() -> Result<()> {
    enable_raw_mode()?;
    execute!(std::io::stderr(), EnterAlternateScreen, EnableMouseCapture)?;
    Ok(())
}

fn shutdown() -> Result<()> {
    execute!(std::io::stderr(), LeaveAlternateScreen, DisableMouseCapture)?;
    disable_raw_mode()?;
    Ok(())
}

async fn run(app: &Arc<tokio::sync::Mutex<App<'_>>>) -> Result<()> {
    let mut t = Terminal::new(CrosstermBackend::new(std::io::stderr()))?;
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
        t.draw(|rect| ui::draw(rect, &app))?;

        // Handle terminal inputs
        let result = match events.next().await {
            InputEvent::Input(key_event) if app.is_editing() => {
                trace!("Keyevent: {:#?}", key_event);
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
    Ok(())
}

pub async fn start_ui(app: &Arc<tokio::sync::Mutex<App<'_>>>) -> Result<()> {
    // setup terminal
    startup()?;

    let status = run(app).await;

    // teardown terminal before unwrapping Result of app run
    shutdown()?;

    status?;

    Ok(())
}
