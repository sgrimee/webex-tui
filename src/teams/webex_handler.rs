// teams/webex_handler.rs

//! Handles events received from the `Teams` events sub-thread.
//!
//! Callbacks to the `App` are made via mutex.
//!

use color_eyre::{eyre::eyre, Result};
use log::*;
use webex::ActivityType::{
    AdaptiveCardSubmit, Highlight, Janus, Locus, Message, Space, StartTyping, Unknown,
};
use webex::MessageActivity::{Acknowledged, Deleted, Posted, Shared};
use webex::SpaceActivity::{Changed, Created, Joined, Left};
use webex::{Event, MessageActivity};

use crate::app::cache::room::RoomId;

use super::Teams;

impl Teams<'_> {
    /// Handle a webex event.
    pub(crate) async fn handle_webex_event(&mut self, event: Event) -> Result<()> {
        match event.activity_type() {
            Message(activity) => self.handle_message_event(&activity, &event).await?,
            Space(activity) => self.handle_space_event(&activity, &event).await?,
            AdaptiveCardSubmit => {
                trace!("Received unhandled adaptive card submit event.");
            }
            Locus => { // Call from webex app to webex app
            }
            Janus => {}
            StartTyping => {
                trace!("Received unhandled start typing event.");
            }
            Highlight => {
                trace!("Received unhandled highlight event.");
            }
            Unknown(s) => {
                trace!("Received unhandled unknown event: {s:#?}");
            }
        }
        Ok(())
    }

    /// Handle a message event.
    async fn handle_message_event(
        &mut self,
        activity: &MessageActivity,
        event: &Event,
    ) -> Result<()> {
        match activity {
            Acknowledged => {
                trace!("Received unhandled message acknowledged event.");
                // trace!("Message acknowledged event: {:#?}", event);
            }
            Posted | Shared => {
                let global_id = event.try_global_id().map_err(|e| {
                    eyre!(
                        "Error getting room id from {:?} message posted/shared event: {}",
                        event,
                        e
                    )
                })?;
                trace!("Received message posted/shared event with global id: {global_id:#?}");
                // The event doesn't contain the message content, go fetch it
                if let Ok(msg) = self.client.get::<webex::Message>(&global_id).await {
                    trace!("Message: {msg:#?}");
                    // add message and mark room as unread
                    self.app.lock().await.cb_message_received(&msg, true);
                }
            }
            Deleted => {
                let global_id = event.try_global_id().map_err(|e| {
                    eyre!(
                        "Error getting room id from {:?} message deleted event: {}",
                        event,
                        e
                    )
                })?;
                trace!("Received message deleted event with global id: {global_id:#?}");
                self.app
                    .lock()
                    .await
                    .cb_message_deleted(&(global_id.id().to_string() as RoomId));
            }
        }
        Ok(())
    }

    // Handle a space event.
    async fn handle_space_event(
        &self,
        activity: &webex::SpaceActivity,
        event: &webex::Event,
    ) -> Result<()> {
        // get_global_id does not work for space events
        match activity {
            Changed | Joined | Created => {
                let room_id = event
                    .try_global_id()
                    .map_err(|e| {
                        eyre!("Error getting room id from {:?} space event: {}", event, e)
                    })?
                    .id()
                    .to_string() as RoomId;
                trace!("Received space event {activity:?} for room: {room_id}");
                trace!("Space event: {event:#?}");
                self.app.lock().await.cb_space_updated(&room_id);
            }
            Left => {
                let room_id = event
                    .try_global_id()
                    .map_err(|e| {
                        eyre!("Error getting room id from {:?} space event: {}", event, e)
                    })?
                    .id()
                    .to_string() as RoomId;
                trace!("Received left space event for room: {room_id}");
                self.app.lock().await.cb_space_left(&room_id);
            }
            _ => {
                trace!("Unhandled space event: {activity:?}");
            }
        }
        Ok(())
    }
}
