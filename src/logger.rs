// logger.rs

use log::LevelFilter;
use std::{collections::HashMap, path::PathBuf};
use tui_logger::{init_logger, set_default_level, TuiLoggerFile};

/// Configures the logger with levels per module.
/// `default_level` is the default log level for all modules. Using Trace
/// here is NOT recommended.
/// `modules` is a list of modules to set the log level to TRACE for.
pub(crate) fn setup_logger(
    default_level: LevelFilter,
    tracing_modules: Vec<&String>,
    logfile: Option<&PathBuf>,
) {
    init_logger(LevelFilter::Trace).unwrap();

    set_default_level(default_level);

    // also apply to to all crate modules so that they show up in the list
    let mut targets = HashMap::new();
    for module in crate_modules().iter() {
        targets.insert(*module, default_level);
    }

    // force some noisy modules to be quiet
    targets.insert("html5ever::tree_builder", LevelFilter::Info);
    targets.insert("html5ever::tokenizer", LevelFilter::Info);
    targets.insert("html5ever::tokenizer::char_ref", LevelFilter::Info);

    // if any modules are specified, set them to trace
    for module in tracing_modules {
        targets.insert(module, LevelFilter::Trace);
    }

    // configure the logger
    for (target, level) in targets {
        tui_logger::set_level_for_target(target, level);
    }

    // set the log file
    if let Some(logfile) = logfile {
        let log_file = TuiLoggerFile::new(&logfile.to_string_lossy());
        tui_logger::set_log_file(log_file);
    }
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
