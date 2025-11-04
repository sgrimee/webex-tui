// main.rs

mod app;
mod banner;
mod config;
mod inputs;
mod logger;
mod teams;
mod theme;
mod tui;
mod ui;

use crate::app::Priority;
use crate::logger::setup_logger;
use app::{App, AppReturn};
use banner::BANNER;
use clap::{arg, command, value_parser};
use clap::{Arg, ArgAction};
use config::{ClientConfig, UserConfig};

const DEFAULT_PORT: u16 = 8080;
use inputs::handler::Event;
use log::LevelFilter;
use std::path::PathBuf;
use teams::app_handler::AppCmdEvent;
use teams::auth::get_integration_token;
use teams::token_cache::load_token_cache;
use teams::ClientCredentials;
use teams::Teams;
use theme::load_theme;
use tui::Tui;

use color_eyre::eyre::Result;
use std::sync::Arc;

/// Retrieve config from config file, interactively guiding the user
/// to create a Webex integration if needed.
fn get_config() -> Result<ClientConfig> {
    let mut client_config = ClientConfig::new();
    client_config.load_config()?;
    Ok(client_config)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let matches = command!()
        .before_help(BANNER)
        .after_help(
            "Your webex Client ID and Client Secret are stored in $HOME/.config/webex-tui/client.yml",
        )
        .arg(
            arg!(-d --debug ... "Set default log level to debug")
            .action(ArgAction::SetTrue)
        )
        .arg(
            arg!(-t --trace <MODULES> "Set trace logging for comma separated module names (see list-modules)")
            .required(false)
            .value_delimiter(',')
            .action(clap::ArgAction::Append)
        )
        .arg(
            arg!(-m --messages <NUMBER> "Set the number of messages to retrieve per room")
            .required(false)
            .value_parser(value_parser!(u32))
            .default_value("10")
        )
        .arg(
            Arg::new("list-modules")
            .long("list-modules")
            .help("List modules that can be traced")
            .required(false)
            .action(ArgAction::SetTrue)
        )
        .arg(
            arg!(-l --log <FILE> "Log to file")
            .required(false)
            .value_parser(value_parser!(PathBuf))
        )
        .get_matches();

    // Display list of modules that can be traced and
    if matches.get_flag("list-modules") {
        println!("Modules that can be traced:");
        for module in logger::crate_modules().iter() {
            println!("  {module}");
        }
        return Ok(());
    }

    // Setup logging.
    color_eyre::install()?;
    let default_log_level = if matches.get_flag("debug") {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    let trace_modules = matches
        .get_many::<String>("trace")
        .unwrap_or_default()
        .collect::<Vec<_>>();
    let log_file_opt = matches.get_one::<PathBuf>("log");
    setup_logger(default_log_level, trace_modules, log_file_opt); // only for tui mode

    // Welcome message
    println!("{BANNER}");
    println!("Starting webex-tui, version {}.", env!("CARGO_PKG_VERSION"));

    // Read configurations
    let client_config = get_config()?;
    let user_config = UserConfig::load();

    // Get port from client config (for OAuth redirect)
    let port = client_config.port.unwrap_or(DEFAULT_PORT);

    let credentials = ClientCredentials {
        client_id: client_config.client_id,
        client_secret: client_config.client_secret,
    };

    // Load theme from user config
    let theme = load_theme(&user_config.theme);

    // Start authentication (cached or browser-based)
    println!("Authenticating to Webex...");
    let token = match get_integration_token(credentials, port).await {
        Ok(token) => token,
        Err(e) => {
            eprintln!("Authentication failed: {e}");
            eprintln!("If you see scope errors, your integration may be configured with different scopes.");
            eprintln!("Trying to continue anyway - some features may not work.");
            // Try to load cached token as fallback
            match load_token_cache() {
                Ok(cache) => {
                    eprintln!("Using cached token...");
                    cache.to_access_token()
                }
                Err(_) => {
                    eprintln!("No cached token available. Cannot continue.");
                    return Err(e);
                }
            }
        }
    };

    // Initialize the terminal user interface with events thread
    let mut tui = Tui::default()?;
    tui.init()?;

    // Setup App and Teams thread
    let (app_to_teams_tx_lowpri, app_to_teams_rx_lowpri) =
        tokio::sync::mpsc::unbounded_channel::<AppCmdEvent>();
    let (app_to_teams_tx_highpri, app_to_teams_rx_highpri) =
        tokio::sync::mpsc::unbounded_channel::<AppCmdEvent>();
    // CLI args override config values
    let debug = matches.get_flag("debug") || user_config.debug;
    let messages_to_load = *matches
        .get_one("messages")
        .unwrap_or(&user_config.messages_to_load);

    let app = Arc::new(tokio::sync::Mutex::new(App::new(
        app_to_teams_tx_lowpri.clone(),
        app_to_teams_tx_highpri.clone(),
        debug,
        messages_to_load,
        theme,
    )));
    let app_ui = Arc::clone(&app);
    tokio::spawn(async move {
        let mut teams = Teams::new(token, app).await;
        teams
            .handle_events(app_to_teams_rx_lowpri, app_to_teams_rx_highpri)
            .await;
    });

    {
        let app = app_ui.lock().await;
        app.dispatch_to_teams(AppCmdEvent::Initialize(), &Priority::High);
    }

    loop {
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
