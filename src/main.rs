// main.rs

mod app;
mod banner;
mod config;
mod inputs;
mod logger;
mod teams;
mod tui;
mod ui;

use crate::logger::{crate_modules, setup_logger};
use app::{App, AppReturn};
use banner::BANNER;
use clap::{arg, command, value_parser};
use config::ClientConfig;
use inputs::handler::Event;
use log::LevelFilter;
// use std::path::PathBuf;
use teams::app_handler::AppCmdEvent;
use teams::auth::get_integration_token;
use teams::ClientCredentials;
use teams::Teams;
use tui::Tui;

use color_eyre::eyre::Result;
use std::sync::Arc;

/// Retrieve credentials from config file, interactively guiding the user
/// to create a Webex integration if needed.
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
    // Parse command line arguments
    let matches = command!()
        .before_help(BANNER)
        .after_help(
          "Your webex Client ID and Client Secret are stored in $HOME/.config/webex-tui/client.yml",
        )
        // .arg(
        //     arg!(
        //     -c --config <FILE> "Specify configuration file path."
        //     )
        //     .required(false)
        //     .value_parser(value_parser!(PathBuf)),
        // )
        .arg(arg!(
            -d --debug ... "Show debug logging. Use twice for trace level."
        ))
        .get_matches();

    // Setup logging.
    color_eyre::install()?;
    let log_level = match matches
        .get_one::<u8>("debug")
        .expect("Count's are defaulted")
    {
        0 => LevelFilter::Info,
        1 => LevelFilter::Debug,
        2 => LevelFilter::Trace,
        _ => LevelFilter::Trace,
    };
    setup_logger(log_level, crate_modules(), LevelFilter::Info); // only for tui mode
    println!("{}", BANNER);
    println!("Starting webex-tui, version {}.", env!("CARGO_PKG_VERSION"));

    // Read configuration or prompt for integration details
    let credentials = get_credentials()?;

    // Start authentication via web browser
    println!("Opening a browser and waiting for authentication.");
    let token = get_integration_token(credentials)
        .await
        .expect("Need token to continue");

    // Initialize the terminal user interface with events thread
    let mut tui = Tui::default()?;
    tui.init()?;

    // Setup App and Teams thread
    let (app_to_teams_tx, app_to_teams_rx) = tokio::sync::mpsc::unbounded_channel::<AppCmdEvent>();
    let app = Arc::new(tokio::sync::Mutex::new(App::new(app_to_teams_tx.clone())));
    let app_ui = Arc::clone(&app);
    tokio::spawn(async move {
        let mut teams = Teams::new(token, app).await;
        teams.handle_events(app_to_teams_rx).await;
    });

    {
        let app = app_ui.lock().await;
        app.dispatch_to_teams(AppCmdEvent::Initialize());
    }

    loop {
        // Move logs to main buffer so they are written to file even if widget not shown
        tui_logger::move_events();

        // Render
        {
            let mut app = app_ui.lock().await;
            tui.draw(&mut app)?;
        }

        // Handle terminal inputs
        let event = tui.events.next().await;
        {
            let mut app = app_ui.lock().await;
            let result = match event {
                Event::Input(key_event) => app.process_key_event(key_event).await,
                Event::Tick => app.update_on_tick().await,
            };
            if result == AppReturn::Exit {
                tui.events.close();
                break;
            }
        }
    }

    tui.exit()?;
    Ok(())
}
