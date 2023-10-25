// logger.rs

use log::LevelFilter;
use tui_logger::{init_logger, set_default_level};

/// Configures the logger with levels per module.
pub fn setup_logger() {
    init_logger(LevelFilter::Trace).unwrap();
    set_default_level(LevelFilter::Info);
    for target in [
        "reqwest::connect",
        "rustls::client::hs",
        "rustls::common_state",
        "rustls::common_state",
        "tungstenite::handshake::client",
        // "webex",
        // "webex::types",
    ] {
        tui_logger::set_level_for_target(target, LevelFilter::Info);
    }
    for target in [
        "webex",
        "webex::types",
        "webex_tui::app",
        "webex_tui::app::rooms_list",
        "webex_tui::app::teams_store",
        "webex_tui::teams",
        "webex_tui::teams::app_handler",
        "webex_tui::teams:auth",
        "webex_tui::teams::webex_handler",
        "webex_tui::tui",
    ] {
        tui_logger::set_level_for_target(target, LevelFilter::Trace);
    }

    const LOG_FILE: &str = concat!(env!("CARGO_PKG_NAME"), ".log");
    let _ = tui_logger::set_log_file(LOG_FILE);
}
