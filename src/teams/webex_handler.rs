// teams/webex_handler.rs

//! Handles events received from the `Teams` events sub-thread.
//!
//! Callbacks to the `App` are made via mutex.
//!

use color_eyre::{eyre::eyre, Result};
use log::*;
use webex::ActivityType::{
    AdaptiveCardSubmit, Highlight, Janus, Locus, Message, Reaction, Space, StartTyping, Unknown,
};
use webex::MessageActivity::{Acknowledged, Deleted, Posted, Shared};
use webex::SpaceActivity::{Changed, Created, Joined, Left};
use webex::{Event, MessageActivity, ReactionActivity};

use crate::app::cache::room::RoomId;

use super::Teams;

impl Teams<'_> {
    /// Handle a webex event.
    pub(crate) async fn handle_webex_event(&mut self, event: Event) -> Result<()> {
        match event.activity_type() {
            Message(activity) => self.handle_message_event(&activity, &event).await?,
            Space(activity) => self.handle_space_event(&activity, &event).await?,
            Reaction(activity) => self.handle_reaction_event(&activity, &event).await?,
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
                info!("===== UNKNOWN EVENT RECEIVED =====");
                info!("Event type string: {s}");
                info!("Full event: {event:#?}");
                info!("===== END UNKNOWN EVENT =====");
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

                // REACTION INVESTIGATION: Log full event details for Joined events
                if matches!(activity, Joined) {
                    info!("===== REACTION INVESTIGATION: Joined Event =====");
                    info!("Full event data: {event:#?}");
                    if let Some(ref activity_data) = event.data.activity {
                        info!("Activity verb: {}", activity_data.verb);
                        info!("Activity object type: {}", activity_data.object.object_type);
                        info!("Activity object content: {:?}", activity_data.object.content);
                        info!("Activity object display_name: {:?}", activity_data.object.display_name);
                        info!("Activity target: {:?}", activity_data.target);
                        info!("Full activity object: {:#?}", activity_data.object);
                        info!("Full activity: {:#?}", activity_data);
                    }
                    info!("===== END REACTION INVESTIGATION =====");
                }

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

    /// Handle a reaction event.
    async fn handle_reaction_event(
        &mut self,
        activity: &ReactionActivity,
        event: &webex::Event,
    ) -> Result<()> {
        info!("===== REACTION EVENT =====");
        info!("Reaction activity: {activity:?}");
        info!("Full event: {event:#?}");

        // Extract the message ID from the parent
        let message_id = event
            .data
            .activity
            .as_ref()
            .and_then(|a| a.parent.as_ref())
            .map(|p| p.id.clone())
            .ok_or_else(|| eyre!("No parent message ID found in reaction event"))?;

        info!("Reaction target message ID: {message_id}");

        // Extract the encrypted reaction data
        if let Some(activity_data) = &event.data.activity {
            if let Some(ref encrypted_emoji) = activity_data.object.display_name {
                info!("Encrypted reaction emoji (JWE): {encrypted_emoji}");

                // Extract encryption key URL
                if let Some(ref encryption_key_url) = activity_data.encryption_key_url {
                    info!("Encryption key URL: {encryption_key_url}");

                    // TODO: Decrypt the reaction emoji
                    // For now, we'll just log the encrypted data
                    // In the future, we'll use webex::encryption::DecryptionService to decrypt

                    match activity {
                        ReactionActivity::Added => {
                            info!("Reaction ADDED to message {message_id}");
                            // TODO: Update message cache with new reaction
                        }
                        ReactionActivity::Removed => {
                            info!("Reaction REMOVED from message {message_id}");
                            // TODO: Update message cache to remove reaction
                        }
                    }
                } else {
                    warn!("No encryption key URL found in reaction event");
                }
            } else {
                warn!("No display_name (encrypted emoji) found in reaction event");
            }
        }

        info!("===== END REACTION EVENT =====");
        Ok(())
    }
}
