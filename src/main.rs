mod banner;
mod config;

use color_eyre::eyre::Result;
use log::*;
use std::env;
use std::panic;
use std::process;
use std::sync::Arc;

use config::ClientConfig;
use webex_tui::app::App;
use webex_tui::start_ui;
use webex_tui::teams::app_handler::AppCmdEvent;
use webex_tui::teams::ClientCredentials;
use webex_tui::teams::Teams;

fn setup_logger() {
    tui_logger::init_logger(LevelFilter::Trace).unwrap();
    tui_logger::set_default_level(LevelFilter::Debug);
    for target in [
        "reqwest::connect",
        "rustls::client::hs",
        "rustls::common_state",
        "rustls::common_state",
        "tungstenite::handshake::client",
        "webex",
        "webex::types",
    ] {
        tui_logger::set_level_for_target(target, LevelFilter::Info);
    }

    const LOG_FILE: &str = concat!(env!("CARGO_PKG_NAME"), ".log");
    let _ = tui_logger::set_log_file(LOG_FILE);
}

fn get_credentials() -> Result<ClientCredentials> {
    let mut client_config = ClientConfig::new();
    client_config.load_config()?;
    Ok(ClientCredentials {
        client_id: client_config.client_id,
        client_secret: client_config.client_secret,
    })
}

fn set_panic_hook() {
    // Ensure the process terminates if one of the threads panics.
    let orig_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        // invoke the default handler and exit the process
        orig_hook(panic_info);
        process::exit(1);
    }));
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    setup_logger();
    let credentials = get_credentials()?;
    set_panic_hook();

    // Channel to send commands to the teams thread
    let (app_to_teams_tx, app_to_teams_rx) = tokio::sync::mpsc::channel::<AppCmdEvent>(100);

    // The teams thread communicates back to the app main thread by locking app
    let app = Arc::new(tokio::sync::Mutex::new(App::new(app_to_teams_tx.clone())));
    let app_ui = Arc::clone(&app);

    tokio::spawn(async move {
        let mut teams = Teams::new(credentials, app).await;
        teams.handle_events(app_to_teams_rx).await;
    });

    start_ui(&app_ui).await?;

    Ok(())
}
