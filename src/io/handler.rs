use std::sync::Arc;

use eyre::Result;
use log::{debug, error, info};

use super::webex_client::get_webex_client;
use super::IoEvent;
use crate::app::state::AppState;
use crate::app::App;

/// In the IO thread, we handle IO event without blocking the UI thread
pub struct IoAsyncHandler<'a> {
    app: Arc<tokio::sync::Mutex<App<'a>>>,
}

impl<'a> IoAsyncHandler<'a> {
    pub fn new(app: Arc<tokio::sync::Mutex<App<'a>>>) -> Self {
        Self { app }
    }

    /// We could be async here
    pub async fn handle_io_event(&mut self, io_event: IoEvent) {
        let result = match io_event {
            IoEvent::Initialize => self.do_initialize().await,
            IoEvent::SendMessage(msg_to_send) => self.do_send_message(msg_to_send).await,
        };

        if let Err(err) = result {
            error!("Oops, something wrong happened: {:?}", err);
        }

        let mut app = self.app.lock().await;
        app.loaded();
    }

    async fn do_initialize(&mut self) -> Result<()> {
        info!("🚀 Login to Webex");
        let client = get_webex_client().await;
        debug!("We got a webex client");
        let mut app = self.app.lock().await;
        app.initialized(client); // we could update the app state
        info!("👍 Login successful");
        Ok(())
    }

    async fn do_send_message(&mut self, msg_to_send: webex::types::MessageOut) -> Result<()> {
        info!("Sending message");
        // TODO: do not block...
        let app = self.app.lock().await;
        if let AppState::Initialized { webex, .. } = app.state() {
            webex
                .send_message(&msg_to_send)
                .await
                .expect("do_send_message");
        }
        // let mut app = self.app.lock().await;
        // app.slept();
        Ok(())
    }
}
