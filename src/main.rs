// main.rs

mod app;
mod banner;
mod config;
mod inputs;
mod logger;
mod teams;
mod tui;
mod ui;

use app::{App, AppReturn};
use config::ClientConfig;
use inputs::handler::Event;
use inputs::key::Key;
use logger::setup_logger;
use teams::app_handler::AppCmdEvent;
use teams::ClientCredentials;
use teams::Teams;
use tui::Tui;

use color_eyre::eyre::Result;
use log::*;
use std::sync::Arc;

fn get_credentials() -> Result<ClientCredentials> {
    let mut client_config = ClientConfig::new();
    client_config.load_config()?;
    Ok(ClientCredentials {
        client_id: client_config.client_id,
        client_secret: client_config.client_secret,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup logging.
    color_eyre::install()?;
    setup_logger();

    // Read configuration
    let credentials = get_credentials()?;

    // Initialize the terminal user interface with events thread
    let mut tui = Tui::default()?;
    tui.init()?;

    // Setup App and Teams thread
    let (app_to_teams_tx, app_to_teams_rx) = tokio::sync::mpsc::channel::<AppCmdEvent>(100);
    let app = Arc::new(tokio::sync::Mutex::new(App::new(app_to_teams_tx.clone())));
    let app_ui = Arc::clone(&app);
    tokio::spawn(async move {
        let mut teams = Teams::new(credentials, app).await;
        teams.handle_events(app_to_teams_rx).await;
    });
    {
        let mut app = app_ui.lock().await;
        app.dispatch_to_teams(AppCmdEvent::Initialize()).await;
    }

    loop {
        let mut app = app_ui.lock().await;

        // Move logs to main buffer so they are written to file even if widget not shown
        tui_logger::move_events();

        // Render
        tui.draw(&app)?;

        // Handle terminal inputs
        let result = match tui.events.next().await {
            Event::Input(key_event) if app.is_editing() => {
                trace!("Keyevent: {:#?}", key_event);
                app.process_editing_key(key_event).await
            }
            Event::Input(key_event) => app.do_action(Key::from(key_event)).await,
            Event::Tick => app.update_on_tick().await,
        };
        if result == AppReturn::Exit {
            tui.events.close();
            break;
        }
    }

    // Exit the user interface.
    tui.exit()?;
    Ok(())
}
