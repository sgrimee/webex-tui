use log::*;
use webex::Event;

use super::Teams;

impl Teams<'_> {
    pub async fn handle_webex_event(&mut self, event: Event) {
        match event.activity_type() {
            webex::ActivityType::Message(webex::MessageActivity::Posted) => {
                // The event stream doesn't contain the message -- you have to go fetch it
                if let Ok(msg) = self
                    .client
                    .get::<webex::Message>(&event.get_global_id())
                    .await
                {
                    trace!("Message: {:?}", msg);
                    let mut app = self.app.lock().await;
                    app.message_received(msg).await;
                }
            }
            _ => {
                debug!("Unhandled webex event type: {:#?}", event.activity_type());
            }
        }
    }
}
