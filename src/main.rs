// main.rs

mod app;
mod banner;
// Parts of the bridge boundary stay unused until the 5E legacy removal.
#[allow(dead_code)]
mod bridge;
mod config;
mod frontend;
mod host;
mod inputs;
mod logger;
mod theme;
mod tui;
mod ui;

use crate::logger::setup_logger;
use app::{App, AppReturn};
use banner::BANNER;
use clap::{arg, command, value_parser};
use clap::{Arg, ArgAction};
use config::UserConfig;
use inputs::handler::Event;
use log::LevelFilter;
use std::path::PathBuf;
use std::process::ExitCode;
use theme::load_theme;

use color_eyre::eyre::Result;

#[tokio::main]
async fn main() -> Result<ExitCode> {
    // Parse command line arguments
    let matches = command!()
        .before_help(BANNER)
        .after_help(
            "webex-tui is a client of the webex-bridge daemon, which owns the Webex connection.\n\
             Install it with `webex-bridge install-service`; webex-tui pairs with it once (approve the dialog).",
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
        return Ok(ExitCode::SUCCESS);
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

    let user_config = UserConfig::load();
    let theme = load_theme(&user_config.theme);
    // CLI args override config values
    let debug = matches.get_flag("debug") || user_config.debug;
    let messages_to_load = *matches
        .get_one("messages")
        .unwrap_or(&user_config.messages_to_load);

    run(debug, messages_to_load, theme).await
}

#[cfg(not(target_os = "macos"))]
async fn run(_debug: bool, _messages_to_load: u32, _theme: theme::Theme) -> Result<ExitCode> {
    eprintln!("webex-tui needs the webex-bridge daemon, which is only supported on macOS.");
    Ok(ExitCode::FAILURE)
}

/// Bridge-backed session (§7.3 S4): startup before raw mode, the runtime on
/// its own task, a drain every tick, a bounded quit join and a metadata-only
/// summary after the terminal is restored.
#[cfg(target_os = "macos")]
async fn run(debug: bool, messages_to_load: u32, theme: theme::Theme) -> Result<ExitCode> {
    use bridge::startup;

    // Pre-terminal startup: socket, status, session-first pairing.
    let deps = match startup::macos_deps(messages_to_load) {
        Ok(deps) => deps,
        Err(error) => return Ok(startup_failed(&error)),
    };
    println!("Connecting to webex-bridge (if a pairing dialog appears, approve it)...");
    let started = match startup::start(deps).await {
        Ok(started) => started,
        Err(error) => return Ok(startup_failed(&error)),
    };
    for notice in &started.notices {
        println!("{}", notice.message());
        log::info!("{}", notice.message());
    }

    let mut app = App::new(debug, theme);
    app.attach_bridge(started.glue);
    let mut runtime = tokio::spawn(started.runtime.run());

    let mut tui = tui::Tui::default()?;
    tui.init()?;

    // Render loop; ends on a quit key or when the runtime itself exits.
    let mut ended: Option<host::Joined> = None;
    let loop_result: Result<()> = async {
        loop {
            tui.draw(&mut app)?;
            tokio::select! {
                event = tui.events.next() => {
                    let result = match event {
                        Event::Input(key_event) => app.process_key_event(key_event).await,
                        Event::Tick => app.update_on_tick().await,
                    };
                    if result == AppReturn::Exit {
                        return Ok(());
                    }
                }
                joined = &mut runtime => {
                    ended = Some(host::Joined::from_join(joined));
                    return Ok(());
                }
            }
        }
    }
    .await;

    // Quit (§7.2 f): note in-flight writes, request shutdown, then join
    // (≤ 35 s) while still draining and redrawing ("finishing write…").
    let mark = app.begin_quit();
    let joined = match ended {
        Some(joined) => joined,
        None => {
            host::join_bounded(&mut runtime, host::QUIT_JOIN_BOUND, host::QUIT_TICK, || {
                app.quit_tick();
                if let Err(error) = tui.draw(&mut app) {
                    log::warn!("redraw while quitting failed: {error}");
                }
            })
            .await
        }
    };
    app.quit_tick();
    tui.events.close();
    tui.exit()?;
    loop_result?;

    if let Some(message) = joined.message() {
        eprintln!("{message}");
    }
    if let Some(mark) = mark {
        for line in app.quit_summary(&mark) {
            println!("{line}");
        }
    }
    Ok(if joined.is_clean() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

#[cfg(target_os = "macos")]
fn startup_failed(error: &bridge::startup::StartupError) -> ExitCode {
    eprintln!("{}", error.message());
    ExitCode::FAILURE
}
