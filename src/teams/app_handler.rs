use super::Teams;
use eyre::Result;
use log::{debug, error, info};
use webex::MessageOut;

pub enum AppCmdEvent {
    Initialize(), // Launch to initiate login to Webex
    SendMessage(MessageOut),
}

impl Teams<'_> {
    /// Handle events dispatched by the App
    pub async fn handle_app_event(&mut self, io_event: AppCmdEvent) {
        let result = match io_event {
            AppCmdEvent::Initialize() => self.do_initialize().await,
            AppCmdEvent::SendMessage(msg_to_send) => self.do_send_message(msg_to_send).await,
        };

        if let Err(err) = result {
            error!("Oops, something wrong happened: {:?}", err);
        }

        let mut app = self.app.lock().await;
        app.loaded();
    }

    async fn do_initialize(&mut self) -> Result<()> {
        info!("🚀 Initializing to Webex");

        if let Ok(me) = self.client.me().await {
            info!("We are: {}", me.display_name);
        }

        let mut app = self.app.lock().await;
        app.initialized();
        info!("👍 Webex initialization successful");
        Ok(())
    }

    async fn do_send_message(&mut self, msg_to_send: webex::types::MessageOut) -> Result<()> {
        debug!("I would like to send");
        self.client
            .send_message(&msg_to_send)
            .await
            .expect("do_send_message");
        let mut app = self.app.lock().await;
        app.message_sent();
        Ok(())
    }
}
