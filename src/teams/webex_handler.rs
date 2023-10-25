// teams/webex_handler.rs

//! Handles events received from the `Teams` events sub-thread.
//!
//! Callbacks to the `App` are made via mutex.
//!
use log::*;
use webex::Event;

use super::Teams;

impl Teams<'_> {
    /// Handle a webex event.
    /// Currently only supports `Message` events.
    // TODO: add support for Room updated (e.g. rename) events
    pub async fn handle_webex_event(&mut self, event: Event) {
        match event.activity_type() {
            webex::ActivityType::Message(webex::MessageActivity::Posted) => {
                // The event doesn't contain the message content, go fetch it
                if let Ok(msg) = self
                    .client
                    .get::<webex::Message>(&event.get_global_id())
                    .await
                {
                    trace!("Message: {:?}", msg);
                    let mut app = self.app.lock().await;
                    // add message and mark room as unread
                    app.cb_message_received(&msg, true).await;
                }
            }
            _ => {
                debug!("Unhandled webex event type: {:#?}", event.activity_type());
            }
        }
    }
}
