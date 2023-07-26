use eyre::Result;
use log::LevelFilter;
use std::panic;
use std::process;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use webex_tui::app::App;
use webex_tui::start_ui;
use webex_tui::teams::app_handler::{IoAsyncHandler, IoEvent};

#[tokio::main]
async fn main() -> Result<()> {
    // Ensure the process terminates if one of the threads panics.
    let orig_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        // invoke the default handler and exit the process
        orig_hook(panic_info);
        process::exit(1);
    }));

    // Channel to the io::handler thread
    let (sync_io_tx, mut sync_io_rx) = tokio::sync::mpsc::channel::<IoEvent>(100);

    // We need to share the App between threads
    let app = Arc::new(tokio::sync::Mutex::new(App::new(sync_io_tx.clone())));
    let app_ui = Arc::clone(&app);

    tui_logger::init_logger(LevelFilter::Debug).unwrap();
    tui_logger::set_default_level(log::LevelFilter::Debug);

    // Handle IO in a specifc thread
    tokio::spawn(async move {
        let mut handler = IoAsyncHandler::new(app);
        loop {
            if let Some(io_event) = sync_io_rx.try_recv().ok() {
                handler.handle_app_event(io_event).await;
            }
            // Process messages from Webex Events sub?-thread
            handler.process_webex_events().await;
            // TODO: fix this so we don't use CPU but use async better
            sleep(Duration::from_millis(100)).await;
        }
    });

    start_ui(&app_ui).await?;

    Ok(())
}
