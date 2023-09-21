use log::LevelFilter;
use tui_logger::{init_logger, set_default_level};

pub fn setup_logger() {
    init_logger(LevelFilter::Trace).unwrap();
    set_default_level(LevelFilter::Debug);
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
