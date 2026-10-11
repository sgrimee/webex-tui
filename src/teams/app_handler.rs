// teams/app_handler.rs

//! Handles events received from the `App` main thread.
//!
//! Callbacks to the `App` are made via mutex.

use super::Teams;
use crate::app::cache::persons::PersonId;
use crate::app::cache::room::RoomId;
use crate::app::cache::teams::TeamId;
use crate::app::cache::MessageId;
use crate::app::state::ActivePane;
use crate::frontend::{data::ComposedBody, envelopes::FrontendIntent};
use color_eyre::eyre::{eyre, Result};
use log::*;
use webex::{
    GlobalId, GlobalIdType, Message, MessageEditParams, MessageListParams, MessageOut,
    RoomListParams, SortRoomsBy,
};

/// Commands the main `App` can send to the `Teams` thread.
#[derive(Clone)]
pub(crate) enum AppCmdEvent {
    DeleteMessage(FrontendIntent),
    /// Temporary neutral-to-legacy edit boundary; only this Teams edge builds edit params.
    EditMessage(FrontendIntent),
    Initialize(),
    ListAllRooms(),
    ListMessagesInRoom(RoomId, Option<MessageId>, u32),
    /// Temporary neutral-to-legacy write boundary; only this Teams edge builds MessageOut.
    SendMessage(FrontendIntent),
    UpdateRoom(RoomId),
    UpdateTeam(TeamId, Option<String>), // TeamId and optional room context
    UpdateMessage(MessageId),
    UpdateChildrenMessages(MessageId, RoomId),
    UpdatePerson(PersonId),
    LeaveRoom(FrontendIntent),
    WhoAmI(),
}

/// Explicit old-transport adapter. It has no logging and accepts only a send
/// intent, so read/edit/delete commands cannot accidentally become writes.
fn legacy_message_out(intent: &FrontendIntent) -> Option<MessageOut> {
    let FrontendIntent::SendMessage {
        room_id,
        parent_id,
        body,
    } = intent
    else {
        return None;
    };
    let mut message = MessageOut {
        room_id: Some(room_id.as_str().to_owned()),
        parent_id: parent_id.as_ref().map(|id| id.as_str().to_owned()),
        ..Default::default()
    };
    match body {
        ComposedBody::PlainText(text) => message.text = Some(text.clone()),
        ComposedBody::Markdown(text) => message.markdown = Some(text.clone()),
    }
    Some(message)
}

fn legacy_delete_id(intent: &FrontendIntent) -> Option<&str> {
    let FrontendIntent::DeleteMessage {
        message_id,
        room_id,
    } = intent
    else {
        return None;
    };
    (!message_id.as_str().is_empty() && !room_id.as_str().is_empty()).then_some(message_id.as_str())
}
fn legacy_leave_room_id(intent: &FrontendIntent) -> Option<&str> {
    let FrontendIntent::LeaveRoom(room_id) = intent else {
        return None;
    };
    (!room_id.as_str().is_empty()).then_some(room_id.as_str())
}
fn legacy_edit_fields(intent: &FrontendIntent) -> Option<(&str, &str, &ComposedBody)> {
    let FrontendIntent::EditMessage {
        message_id,
        room_id,
        body,
    } = intent
    else {
        return None;
    };
    Some((message_id.as_str(), room_id.as_str(), body))
}

impl AppCmdEvent {
    /// Returns true if this event should be retried on failure.
    /// Only critical read operations that don't modify state should be retried.
    fn should_retry_on_failure(&self) -> bool {
        matches!(self, AppCmdEvent::ListAllRooms())
    }
}

impl Teams<'_> {
    /// Handle an `AppCmdEvent` dispatched by the App.
    pub(crate) async fn handle_app_event(&mut self, app_cmd_event: AppCmdEvent) {
        {
            self.app.lock().await.state.is_loading = true;
        }
        let should_retry = app_cmd_event.should_retry_on_failure();
        if let Err(error) = match app_cmd_event {
            AppCmdEvent::DeleteMessage(intent) => match legacy_delete_id(&intent) {
                Some(message_id) => self.delete_message(message_id).await,
                None => Err(eyre!("invalid delete intent")),
            },
            AppCmdEvent::EditMessage(intent) => match legacy_edit_fields(&intent) {
                Some((message_id, room_id, body)) => {
                    self.do_edit_message(message_id, room_id, body).await
                }
                None => Err(eyre!("invalid edit intent")),
            },
            AppCmdEvent::Initialize() => self.do_initialize().await,
            AppCmdEvent::ListAllRooms() => self.do_list_all_rooms_with_retry().await,
            AppCmdEvent::ListMessagesInRoom(room_id, before_id, max) => {
                self.do_list_messages_in_room(&room_id, before_id, max)
                    .await
            }
            AppCmdEvent::SendMessage(intent) => match legacy_message_out(&intent) {
                Some(message) => self.do_send_message(&message).await,
                None => Err(eyre!("invalid send intent")),
            },
            AppCmdEvent::UpdateChildrenMessages(msg_id, room_id) => {
                self.do_update_children_messages(&msg_id, &room_id).await
            }
            AppCmdEvent::UpdateMessage(msg_id) => self.do_update_message(&msg_id).await,
            AppCmdEvent::UpdatePerson(person_id) => self.do_update_person(&person_id).await,
            AppCmdEvent::UpdateRoom(room_id) => self.do_refresh_room(&room_id).await,
            AppCmdEvent::UpdateTeam(team_id, room_context) => {
                self.do_update_team(&team_id, room_context.as_deref()).await
            }
            AppCmdEvent::LeaveRoom(intent) => match legacy_leave_room_id(&intent) {
                Some(room_id) => self.do_leave_room(room_id).await,
                None => Err(eyre!("invalid leave intent")),
            },
            AppCmdEvent::WhoAmI() => self.get_me_user().await,
        } {
            error!("Error handling app event: {error}");
            if should_retry {
                warn!("Critical operation failed, will be retried automatically");
            }
        }
        {
            self.app.lock().await.state.is_loading = false;
        }
    }

    /// Calls back `cb_teams_initialized` on `app`.
    /// This is useful to inform the main thread that the `teams` thread is ready.
    async fn do_initialize(&mut self) -> Result<()> {
        debug!("Initializing webex-tui");

        // Note: Scope verification happens automatically during event stream initialization
        // If there are scope issues, detailed error messages will appear in the logs

        let mut app = self.app.lock().await;
        app.cb_teams_initialized();
        debug!("👍 Webex initialization successful");
        Ok(())
    }

    /// Deletes the message with given id
    async fn delete_message(&self, id: &str) -> Result<()> {
        let global_id = GlobalId::new(GlobalIdType::Message, id.to_owned()).unwrap();
        match self.client.delete::<Message>(&global_id).await {
            Ok(_) => {
                debug!("Deleted message with id: {global_id:?}");
                Ok(())
            }
            Err(e) => Err(eyre!("Could not delete message: {}", e)),
        }
    }

    /// Sends `msg_to_send` and calls back `cb_message_sent` on app when done.
    async fn do_send_message(&mut self, msg_to_send: &MessageOut) -> Result<()> {
        match self.client.send_message(msg_to_send).await {
            Ok(msg) => {
                self.app.lock().await.cb_message_sent(&msg);
                debug!("Sent message: {msg_to_send:?}");
                Ok(())
            }
            Err(e) => Err(eyre!("Error sending message: {}", e)),
        }
    }

    /// Gets the message with given id.
    async fn do_update_message(&self, msg_id: &MessageId) -> Result<()> {
        debug!("Getting message with id: {msg_id:?}");
        let global_id = GlobalId::new(GlobalIdType::Message, msg_id.to_owned()).unwrap();
        match self.client.get::<Message>(&global_id).await {
            Ok(msg) => {
                self.app.lock().await.cb_message_received(&msg, false);
                debug!("Updated message: {msg_id:?}");
                Ok(())
            }
            Err(e) => Err(eyre!("Error retrieving message: {}", e)),
        }
    }

    /// Gets the children of the message with given id.
    async fn do_update_children_messages(
        &self,
        msg_id: &MessageId,
        room_id: &RoomId,
    ) -> Result<()> {
        debug!("Getting children of message with id: {msg_id:?}");
        let msg_gid = GlobalId::new(GlobalIdType::Message, msg_id.to_owned()).unwrap();
        let mut params = MessageListParams::new(room_id);
        params.parent_id = Some(msg_gid.id());
        match self.client.list_with_params::<Message>(params).await {
            Ok(messages) => {
                if !messages.is_empty() {
                    // add messages but do not change the room unread status
                    self.app.lock().await.cb_messages_received_in_room(
                        &messages[0].room_id.clone().unwrap(),
                        &messages,
                        false,
                    );
                    debug!(
                        "Updated {} children of message: {:?}",
                        messages.len(),
                        msg_id
                    );
                }
                Ok(())
            }
            Err(e) => Err(eyre!("Error retrieving children of message: {}", e)),
        }
    }

    /// Edits the message with given id and params
    async fn do_edit_message(
        &self,
        message_id: &str,
        room_id: &str,
        body: &ComposedBody,
    ) -> Result<()> {
        let message_id = GlobalId::new(GlobalIdType::Message, message_id.to_owned()).unwrap();
        let room_id = GlobalId::new(GlobalIdType::Room, room_id.to_owned()).unwrap();
        let params = MessageEditParams {
            room_id: room_id.id(),
            text: match body {
                ComposedBody::PlainText(text) => Some(text),
                ComposedBody::Markdown(_) => None,
            },
            markdown: match body {
                ComposedBody::PlainText(_) => None,
                ComposedBody::Markdown(text) => Some(text),
            },
            ..Default::default()
        };
        match self.client.edit_message(&message_id, &params).await {
            Ok(_) => {
                debug!("Edited message with id: {message_id:?}");
                Ok(())
            }
            Err(e) => Err(eyre!("Could not edit message: {}", e)),
        }
    }

    async fn do_refresh_room(&mut self, id: &RoomId) -> Result<()> {
        let global_id = GlobalId::new(GlobalIdType::Room, id.to_owned()).unwrap();
        debug!("Getting room with local id {id} and global id: {global_id:?}");
        match self.client.get::<webex::Room>(&global_id).await {
            Ok(webex_room) => {
                self.app.lock().await.cb_room_updated(webex_room);
                Ok(())
            }
            Err(e) => Err(eyre!("Error retrieving room: {}", e)),
        }
    }

    /// Gets the team with given id and updates the store.
    /// Many of these calls fail because the user does not have access to the
    /// team details (HTTP 404 when not a team member), so errors are silenced.
    /// Note: You can access rooms within a team without being a team member,
    /// but team details require team membership per Webex API design.
    async fn do_update_team(&self, team_id: &TeamId, room_context: Option<&str>) -> Result<()> {
        let global_id = GlobalId::new(GlobalIdType::Team, team_id.to_owned()).unwrap();
        debug!("Getting team with global id: {global_id:?}");
        match self.client.get::<webex::Team>(&global_id).await {
            Ok(webex_team) => {
                self.app.lock().await.cb_team_updated(webex_team);
            }
            Err(e) => {
                // Log the error but don't fail the operation
                let error_msg = e.to_string();
                let context_info = room_context
                    .map(|room| format!(" (requested from room: '{room}')"))
                    .unwrap_or_default();

                if error_msg.contains("missing required scopes")
                    || error_msg.contains("missing required roles")
                {
                    debug!("Skipping team update for {team_id}{context_info}: insufficient API permissions");
                } else if error_msg.contains("Could not find teams") || error_msg.contains("404") {
                    debug!("Skipping team update for {team_id}{context_info}: not a team member (404 is expected behavior)");
                } else {
                    debug!("Failed to get team details for {team_id}{context_info}: {error_msg}");
                }
            }
        }
        Ok(())
    }

    /// Gets as many rooms as the API allows (1000 as webex-rust does not yet implement paging) rooms.
    /// Updates the store and sets the active pane to Rooms list.
    async fn do_list_all_rooms(&mut self) -> Result<()> {
        debug!("Getting all rooms");
        let params = RoomListParams {
            sort_by: Some(SortRoomsBy::LastActivity),
            max: Some(1000),
            ..Default::default()
        };
        self.list_and_add_rooms(params).await?;
        self.app
            .lock()
            .await
            .state
            .set_active_pane(Some(ActivePane::Rooms));
        Ok(())
    }

    /// Gets all rooms with retry logic on failure.
    /// Retries up to 3 times with 5 second delays between attempts.
    async fn do_list_all_rooms_with_retry(&mut self) -> Result<()> {
        const MAX_RETRIES: u32 = 3;
        const RETRY_DELAY_SECS: u64 = 5;

        for attempt in 1..=MAX_RETRIES {
            match self.do_list_all_rooms().await {
                Ok(()) => {
                    if attempt > 1 {
                        info!("Successfully loaded rooms after {attempt} attempts");
                    }
                    return Ok(());
                }
                Err(e) => {
                    if attempt < MAX_RETRIES {
                        warn!("Failed to load rooms (attempt {attempt}/{MAX_RETRIES}): {e}");
                        warn!("Retrying in {RETRY_DELAY_SECS} seconds...");
                        tokio::time::sleep(std::time::Duration::from_secs(RETRY_DELAY_SECS)).await;
                    } else {
                        error!("Failed to load rooms after {MAX_RETRIES} attempts: {e}");
                        return Err(e);
                    }
                }
            }
        }
        unreachable!()
    }

    /// Gets the rooms as per `params`, updates the store.
    async fn list_and_add_rooms(&mut self, params: RoomListParams<'_>) -> Result<()> {
        match self.client.list_with_params::<webex::Room>(params).await {
            Ok(webex_rooms) => {
                debug!("Got {} rooms", webex_rooms.len());
                for webex_room in webex_rooms {
                    self.app.lock().await.cb_room_updated(webex_room);
                }
                Ok(())
            }
            Err(e) => Err(eyre!("Error retrieving rooms: {:#?}", e)),
        }
    }

    /// Gets all the messages in a room and update the store.
    async fn do_list_messages_in_room(
        &mut self,
        room_id: &RoomId,
        before_id: Option<MessageId>,
        max: u32,
    ) -> Result<()> {
        debug!("Getting messages in room {room_id}");
        let gid = GlobalId::new(GlobalIdType::Room, room_id.to_owned()).unwrap();
        let mut params = MessageListParams::new(gid.id());
        params.max = Some(max);
        let mut _msg_id = String::new();
        if let Some(before_id) = before_id {
            debug!("Only messages before {before_id}");
            _msg_id = before_id.to_string();
            params.before_message = Some(&_msg_id);
        }
        match self.client.list_with_params::<Message>(params).await {
            Ok(messages) => {
                // add messages but do not change the room unread status
                self.app
                    .lock()
                    .await
                    .cb_messages_received_in_room(room_id, &messages, false);
                Ok(())
            }
            Err(e) => Err(eyre!("Error retrieving messages in room: {:#?}", e)),
        }
    }

    async fn do_update_person(&self, person_id: &PersonId) -> Result<()> {
        debug!("Getting person with id: {person_id}");
        let global_id = GlobalId::new(GlobalIdType::Person, person_id.to_owned()).unwrap();
        match self.client.get::<webex::Person>(&global_id).await {
            Ok(webex_person) => {
                self.app.lock().await.cb_person_updated(webex_person);
                Ok(())
            }
            Err(e) => Err(eyre!("Error retrieving person: {}", e)),
        }
    }

    /// Retrieve the logged in user
    async fn get_me_user(&self) -> Result<()> {
        trace!("Getting logged in user");
        let global_id =
            GlobalId::new_with_cluster_unchecked(GlobalIdType::Person, "me".to_string(), None);
        match self.client.get::<webex::Person>(&global_id).await {
            Ok(me) => {
                debug!("Webex knows us as user: {}", me.display_name);
                self.app.lock().await.cb_set_me(&me);
                Ok(())
            }
            Err(e) => Err(eyre!("Error retrieving logged in user: {}", e)),
        }
    }

    /// Leave a room by deleting our membership
    async fn do_leave_room(&mut self, room_id: &str) -> Result<()> {
        let room_id = room_id.to_owned();
        let global_id = GlobalId::new(GlobalIdType::Room, room_id.clone()).unwrap();
        debug!("=== STARTING LEAVE ROOM PROCESS ===");
        debug!("Leaving room with local id {room_id} and global id: {global_id:?}");

        // Get room info for better logging and validation
        let (room_title, is_locked) = {
            let app = self.app.lock().await;
            let room = app.state.cache.rooms.room_with_id(&room_id);
            (
                room.and_then(|r| r.title.clone())
                    .unwrap_or_else(|| "Unknown".to_string()),
                room.map(|r| r.is_moderated()).unwrap_or(false),
            )
        };
        debug!("Room title: {room_title}, is_locked: {is_locked}");

        // Check if room is moderated/locked before attempting to leave
        if is_locked {
            warn!(
                "⚠️  Cannot leave moderated room '{room_title}' ({room_id}). Moderated rooms require administrator approval to leave."
            );
            return Err(eyre!(
                "Cannot leave moderated room '{}'. Moderated rooms are locked and require administrator approval to leave or remove members.",
                room_title
            ));
        }

        debug!("About to call client.leave_room...");
        match self.client.leave_room(&global_id).await {
            Ok(_) => {
                debug!("✅ Successfully left room: {room_title} ({room_id})");
                // Remove the room from the cache
                debug!("Calling cb_space_left callback...");
                self.app.lock().await.cb_space_left(&room_id);
                debug!("=== LEAVE ROOM PROCESS COMPLETED ===");
                Ok(())
            }
            Err(e) => {
                error!("❌ Error leaving room {room_title} ({room_id}): {e}");
                debug!("=== LEAVE ROOM PROCESS FAILED ===");
                Err(eyre!("Error leaving room: {}", e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn neutral_send_intent_maps_only_at_legacy_edge() {
        let intent = FrontendIntent::SendMessage {
            room_id: crate::frontend::data::RoomId::new("room"),
            parent_id: Some(crate::frontend::data::MessageId::new("parent")),
            body: ComposedBody::Markdown("**body**".into()),
        };
        let message = legacy_message_out(&intent).unwrap();
        assert_eq!(message.room_id.as_deref(), Some("room"));
        assert_eq!(message.parent_id.as_deref(), Some("parent"));
        assert_eq!(message.markdown.as_deref(), Some("**body**"));
        assert!(message.text.is_none());
    }
    #[test]
    fn non_send_intent_never_becomes_legacy_message() {
        assert!(legacy_message_out(&FrontendIntent::WhoAmI).is_none());
    }
    #[test]
    fn neutral_delete_and_leave_validate_matching_intents() {
        let delete = FrontendIntent::DeleteMessage {
            message_id: crate::frontend::data::MessageId::new("message"),
            room_id: crate::frontend::data::RoomId::new("room"),
        };
        assert_eq!(legacy_delete_id(&delete), Some("message"));
        assert!(legacy_delete_id(&FrontendIntent::LeaveRoom(
            crate::frontend::data::RoomId::new("room")
        ))
        .is_none());
        assert_eq!(
            legacy_leave_room_id(&FrontendIntent::LeaveRoom(
                crate::frontend::data::RoomId::new("room")
            )),
            Some("room")
        );
        assert!(legacy_leave_room_id(&delete).is_none());
    }
    #[test]
    fn neutral_edit_intent_maps_exact_fields_only_at_legacy_edge() {
        let intent = FrontendIntent::EditMessage {
            message_id: crate::frontend::data::MessageId::new("message"),
            room_id: crate::frontend::data::RoomId::new("room"),
            body: ComposedBody::PlainText("body".into()),
        };
        let (message, room, body) = legacy_edit_fields(&intent).unwrap();
        assert_eq!((message, room), ("message", "room"));
        assert!(matches!(body, ComposedBody::PlainText(text) if text == "body"));
        assert!(legacy_edit_fields(&FrontendIntent::WhoAmI).is_none());
    }
}
