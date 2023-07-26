use std::sync::Arc;

use eyre::Result;
use log::{debug, error, info};
use webex::MessageOut;

use super::{webex_handler, Teams};
use crate::app::App;

#[derive(Debug, Clone)]
pub enum IoEvent {
    Initialize, // Launch to initiate login to Webex
    SendMessage(MessageOut),
}

/// In the IO/Teams thread, we handle Webex activity without blocking the UI thread
pub struct IoAsyncHandler<'a> {
    app: Arc<tokio::sync::Mutex<App<'a>>>,
    teams: Option<Teams>,
}

impl<'a> IoAsyncHandler<'a> {
    pub fn new(app: Arc<tokio::sync::Mutex<App<'a>>>) -> Self {
        Self { app, teams: None }
    }

    pub async fn process_webex_events(&mut self) {
        if let Some(teams) = &mut self.teams {
            webex_handler::process_webex_events(teams).await;
        }
    }

    /// Handle events dispatched by the App
    pub async fn handle_app_event(&mut self, io_event: IoEvent) {
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
        info!("🚀 Initializing to Webex");
        self.teams = Some(Teams::new().await);
        let mut app = self.app.lock().await;
        app.initialized();
        info!("👍 Webex initialization successful");
        Ok(())
    }

    async fn do_send_message(&mut self, msg_to_send: webex::types::MessageOut) -> Result<()> {
        debug!("I would like to send");
        if let Some(teams) = &self.teams {
            teams
                .client
                .send_message(&msg_to_send)
                .await
                .expect("do_send_message");
        }
        let mut app = self.app.lock().await;
        app.message_sent();
        Ok(())
    }
}
