// teams/webex_handler.rs

//! Handles events received from the `Teams` events sub-thread.
//!
//! Callbacks to the `App` are made via mutex.
//!

use log::*;
use webex::ActivityType::{
    AdaptiveCardSubmit, Highlight, Janus, Locus, Message, Space, StartTyping, Unknown,
};
use webex::Event;
use webex::MessageActivity::{Acknowledged, Deleted, Posted, Shared};
use webex::SpaceActivity::{
    Changed, Created, Joined, Left, Locked, MeetingScheduled, ModeratorAssigned,
    ModeratorUnassigned, Unlocked,
};

use super::Teams;

impl Teams<'_> {
    /// Handle a webex event.
    /// Currently only supports `Message` events.
    // TODO: add support for Room updated (e.g. rename) events
    pub(crate) async fn handle_webex_event(&mut self, event: Event) {
        match event.activity_type() {
            Message(Acknowledged) => {
                trace!("Received unhandled message acknowledged event.");
            }
            Message(Posted) | Message(Shared) => {
                trace!("Event: {:#?}", event);
                trace!("Event global id: {:#?}", event.get_global_id());
                // The event doesn't contain the message content, go fetch it
                if let Ok(msg) = self
                    .client
                    .get::<webex::Message>(&event.get_global_id())
                    .await
                {
                    trace!("Message: {:#?}", msg);
                    let mut app = self.app.lock().await;
                    // add message and mark room as unread
                    app.cb_message_received(&msg, true);
                }
            }
            Message(Deleted) => {
                match event
                    .data
                    .activity
                    .clone()
                    .and_then(|a| a.target)
                    .and_then(|t| t.global_id)
                {
                    Some(room_id) => {
                        trace!("Received message deleted event in room {}", room_id);
                        self.app.lock().await.cb_message_deleted(&room_id);
                    }
                    _ => {
                        error!(
                            "Received message deleted event without room id: {:#?}",
                            event
                        );
                    }
                }
            }
            Space(Created) => {
                trace!("Received unhandled space created event.");
            }
            Space(Joined) => {
                trace!("Received unhandled joined space event.");
            }
            Space(Left) => {
                trace!("Received unhandled left space event.");
            }
            Space(Changed) => {
                trace!("Received unhandled space changed event.");
            }
            Space(MeetingScheduled) => {
                trace!("Received unhandled meeting scheduled event.");
            }
            Space(Locked) => {
                info!("Received unhandled space locked event.");
            }
            Space(Unlocked) => {
                info!("Received unhandled space unlocked event.");
            }
            Space(ModeratorAssigned) => {
                info!("Received unhandled moderator assigned event.");
            }
            Space(ModeratorUnassigned) => {
                info!("Received unhandled moderator unassigned event.");
            }
            AdaptiveCardSubmit => {
                trace!("Received unhandled adaptive card submit event.");
            }
            Locus => {
                trace!("Received unhandled locus event.");
            }
            Janus => {
                trace!("Received unhandled janus event.");
            }
            StartTyping => {
                trace!("Received unhandled start typing event.");
            }
            Highlight => {
                trace!("Received unhandled highlight event.");
            }
            Unknown(s) => {
                warn!("Received unhandled unknown event: {:#?}", s);
            }
        }
    }
}
