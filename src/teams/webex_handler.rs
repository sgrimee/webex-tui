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
use webex::SpaceActivity::{
    Changed, Created, Favorite, Joined, Left, Locked, MeetingScheduled, ModeratorAssigned,
    ModeratorUnassigned, Unfavorite, Unlocked,
};
use webex::{Event, GlobalId, GlobalIdType, MessageActivity};

use crate::app::cache::room::RoomId;

use super::Teams;

impl Teams<'_> {
    /// Handle a webex event.
    /// Currently only supports `Message` events.
    // TODO: add support for Room updated (e.g. rename) events
    pub(crate) async fn handle_webex_event(&mut self, event: Event) {
        match event.activity_type() {
            Message(activity) => self.handle_message_event(&activity, &event).await,
            Space(activity) => self.handle_space_event(&activity, &event).await,
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
                warn!("Received unhandled unknown event: {:#?}", s);
            }
        }
    }

    /// Handle a message event.
    async fn handle_message_event(&mut self, activity: &MessageActivity, event: &Event) {
        match activity {
            Acknowledged => {
                trace!("Received unhandled message acknowledged event.");
            }
            Posted | Shared => {
                trace!("Event: {:#?}", event);
                trace!("Event global id: {:#?}", event.get_global_id());
                // The event doesn't contain the message content, go fetch it
                if let Ok(msg) = self
                    .client
                    .get::<webex::Message>(&event.get_global_id())
                    .await
                {
                    trace!("Message: {:#?}", msg);
                    // add message and mark room as unread
                    self.app.lock().await.cb_message_received(&msg, true);
                }
            }
            Deleted => {
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
        }
    }

    // Handle a space event.
    async fn handle_space_event(&self, activity: &webex::SpaceActivity, event: &webex::Event) {
        // get_global_id does not work for space events
        match activity {
            Changed => {
                // get_global_id does not work for this event. TODO: fix upstream
                match target_room_id(event) {
                    Ok(room_id) => {
                        debug!("Space changed event for room id {}", room_id,);
                        self.app.lock().await.cb_space_changed(&room_id);
                    }
                    Err(e) => {
                        error!("Error getting room id from space event: {}", e);
                    }
                }
            }
            Created => {
                if let Some(room_id) = room_id_from_space_created_event(event) {
                    debug!("Space created event for room id {}", room_id,);
                    self.app.lock().await.cb_space_created(&room_id);
                };
            }
            Favorite => {
                trace!("Unhandled space favorite event.");
            }
            Joined => match target_room_id(event) {
                Ok(room_id) => {
                    debug!("Joined space event for room id {}", room_id);
                    self.app.lock().await.cb_space_joined(&room_id);
                }
                Err(e) => {
                    error!("Error getting room id from space event: {}", e);
                }
            },
            Left => {
                // get_global_id does not work for this event. TODO: fix upstream
                match target_room_id(event) {
                    Ok(room_id) => {
                        debug!("Left space event for room id {}", room_id,);
                        self.app.lock().await.cb_space_left(&room_id);
                    }
                    Err(e) => {
                        error!("Error getting room id from space event: {}", e);
                    }
                }
            }
            Locked => {
                trace!("Unhandled space locked event.");
            }
            MeetingScheduled => {
                trace!("Unhandled meeting scheduled event.");
            }
            ModeratorAssigned => {
                trace!("Unhandled moderator assigned event.");
            }
            ModeratorUnassigned => {
                trace!("Unhandled moderator unassigned event.");
            }
            Unfavorite => {
                trace!("Unhandled space unfavorite event.");
            }
            Unlocked => {
                trace!("Unhandled space unlocked event.");
            }
        }
    }
}

fn room_id_from_space_created_event(event: &Event) -> Option<String> {
    // API weirdness... the event contains an id that is close to the room id,
    // but it is not the same. It differs from the room id by one character,
    // always by a value of 2. So we cannot use Event::get_global_id() here.
    let activity = match event.data.activity {
        Some(ref activity) => activity,
        None => {
            error!("No activity in space created event");
            return None;
        }
    };
    let mut uuid = activity.clone().id;
    if uuid.as_bytes()[7] == b'2' {
        uuid.replace_range(7..8, "0");
    } else {
        error!("Space created uuid {uuid} could not be not patched");
        return None;
    }
    let room_id = GlobalId::new_with_cluster_unchecked(GlobalIdType::Room, uuid, None)
        .id()
        .to_string() as RoomId;
    Some(room_id)
}

fn target_room_id(event: &Event) -> Result<RoomId> {
    event
        .data
        .activity
        .clone()
        .and_then(|a| a.target)
        .and_then(|t| t.global_id)
        .map(|id| id as RoomId)
        .ok_or(eyre!("No target room id in event"))
}
