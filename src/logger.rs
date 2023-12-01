// logger.rs

use log::LevelFilter;
use tui_logger::{init_logger, set_default_level};

/// Configures the logger with levels per module.
pub(crate) fn setup_logger(
    default_level: LevelFilter,
    modules: &[&str],
    modules_level: LevelFilter,
) {
    init_logger(LevelFilter::Trace).unwrap();
    set_default_level(default_level);

    for target in modules {
        tui_logger::set_level_for_target(target, modules_level);
    }

    const LOG_FILE: &str = concat!(env!("CARGO_PKG_NAME"), ".log");
    let _ = tui_logger::set_log_file(LOG_FILE);
}

/// Returns a list of the modules in this crate
/// in a format usable by set_level_for_target
pub(crate) fn crate_modules() -> &'static [&'static str] {
    &[
        "webex_tui::app::actions",
        "webex_tui::app::cache::msg_thread",
        "webex_tui::app::cache::room_and_team_title",
        "webex_tui::app::cache::room_content",
        "webex_tui::app::cache::room_list_filter",
        "webex_tui::app::cache::room",
        "webex_tui::app::cache::rooms",
        "webex_tui::app::cache::teams",
        "webex_tui::app::cache",
        "webex_tui::app::callbacks",
        "webex_tui::app::mesage_editor",
        "webex_tui::app::messages_list",
        "webex_tui::app::rooms_list",
        "webex_tui::app::state",
        "webex_tui::app",
        "webex_tui::config",
        "webex_tui::teams::app_handler",
        "webex_tui::teams::webex_handler",
        "webex_tui::teams:auth",
        "webex_tui::teams:client",
        "webex_tui::teams",
        "webex_tui::tui",
        "webex_tui",
        "webex::types",
        "webex",
    ]
}
