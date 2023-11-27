// teams/app_handler.rs

//! Handles events received from the `App` main thread.
//!
//! Callbacks to the `App` are made via mutex.

use super::Teams;
use crate::app::state::ActivePane;
use crate::app::teams_store::room::RoomId;
use crate::app::teams_store::MessageId;
use color_eyre::eyre::{eyre, Result};
use log::*;
use webex::{
    GlobalId, GlobalIdType, Message, MessageEditParams, MessageListParams, MessageOut,
    RoomListParams, SortRoomsBy,
};

/// Commands the main `App` can send to the `Teams` thread.
#[derive(Debug)]
pub(crate) enum AppCmdEvent {
    DeleteMessage(MessageId),
    EditMessage(MessageId, RoomId, String),
    Initialize(),
    ListAllRooms(),
    ListMessagesInRoom(RoomId),
    SendMessage(MessageOut),
    UpdateRoom(RoomId),
    // Quit(),
}

impl Teams<'_> {
    /// Handle an `AppCmdEvent` dispatched by the App.
    pub(crate) async fn handle_app_event(&mut self, app_cmd_event: AppCmdEvent) {
        {
            self.app.lock().await.state.is_loading = true;
        }
        if let Err(error) = match app_cmd_event {
            AppCmdEvent::DeleteMessage(msg_id) => self.delete_message(&msg_id).await,
            AppCmdEvent::Initialize() => self.do_initialize().await,
            AppCmdEvent::ListAllRooms() => self.do_list_all_rooms().await,
            AppCmdEvent::ListMessagesInRoom(room_id) => {
                self.do_list_messages_in_room(&room_id).await
            }

            AppCmdEvent::SendMessage(msg_to_send) => self.do_send_message(&msg_to_send).await,

            AppCmdEvent::EditMessage(msg_id, room_id, text) => {
                self.do_edit_message(&msg_id, &room_id, &text).await
            }
            AppCmdEvent::UpdateRoom(room_id) => self.do_refresh_room(&room_id).await,
            // AppCmdEvent::Quit() => self.do_quit().await,
        } {
            error!("Error handling app event: {}", error);
        }
        {
            self.app.lock().await.state.is_loading = false;
        }
    }

    /// Calls back `cb_teams_initialized` on `app`.
    /// This is useful to inform the main thread that the `teams` thread is ready.
    async fn do_initialize(&mut self) -> Result<()> {
        let mut app = self.app.lock().await;
        app.cb_teams_initialized();
        debug!("👍 Webex initialization successful");
        Ok(())
    }

    /// Deletes the message with given id
    async fn delete_message(&self, id: &MessageId) -> Result<()> {
        let global_id = GlobalId::new(GlobalIdType::Message, id.to_owned()).unwrap();
        match self.client.delete::<Message>(&global_id).await {
            Ok(_) => {
                debug!("Deleted message with id: {:?}", global_id);
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
                debug!("Sent message: {:?}", msg_to_send);
                Ok(())
            }
            Err(e) => Err(eyre!("Error sending message: {}", e)),
        }
    }

    /// Edits the message with given id and params
    async fn do_edit_message(
        &self,
        message_id: &MessageId,
        room_id: &RoomId,
        text: &str,
    ) -> Result<()> {
        let message_id = GlobalId::new(GlobalIdType::Message, message_id.to_owned()).unwrap();
        let room_id = GlobalId::new(GlobalIdType::Room, room_id.to_owned()).unwrap();
        let params = MessageEditParams {
            room_id: room_id.id(),
            text: Some(text),
            ..Default::default()
        };
        match self.client.edit_message(&message_id, &params).await {
            Ok(_) => {
                debug!("Edited message with id: {:?}", message_id);
                Ok(())
            }
            Err(e) => Err(eyre!("Could not edit message: {}", e)),
        }
    }

    async fn do_refresh_room(&mut self, id: &RoomId) -> Result<()> {
        let global_id = GlobalId::new(GlobalIdType::Room, id.to_owned()).unwrap();
        debug!(
            "Getting room with local id {} and global id: {:?}",
            id, global_id
        );
        match self.client.get::<webex::Room>(&global_id).await {
            Ok(webex_room) => {
                self.app.lock().await.cb_room_updated(webex_room);
                Ok(())
            }
            Err(e) => Err(eyre!("Error retrieving room: {}", e)),
        }
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
    async fn do_list_messages_in_room(&mut self, room_id: &RoomId) -> Result<()> {
        debug!("Getting messages in room {}", room_id);
        let gid = GlobalId::new(GlobalIdType::Room, room_id.to_owned()).unwrap();
        let params = MessageListParams::new(gid.id());
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
}
