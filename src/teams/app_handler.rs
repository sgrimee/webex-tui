// teams/app_handler.rs

//! Handles events received from the `App` main thread.
//!
//! Callbacks to the `App` are made via mutex.

use crate::app::state::ActivePane;
use crate::app::teams_store::RoomId;

use super::Teams;
use log::{debug, error, info};
use webex::{GlobalId, GlobalIdType, MessageOut, Room};

/// Commands the main `App` can send to the `Teams` thread.
#[derive(Debug)]
pub enum AppCmdEvent {
    Initialize(),
    ListAllRooms(),
    ListMessagesInRoom(RoomId),
    SendMessage(MessageOut),
    UpdateRoom(RoomId),
    // Quit(),
}

impl Teams<'_> {
    /// Handle an `AppCmdEvent` dispatched by the App.
    pub async fn handle_app_event(&mut self, app_cmd_event: AppCmdEvent) {
        {
            self.app.lock().await.state.is_loading = true;
        }
        match app_cmd_event {
            AppCmdEvent::Initialize() => self.do_initialize().await,
            AppCmdEvent::ListAllRooms() => self.do_list_all_rooms().await,
            AppCmdEvent::ListMessagesInRoom(room_id) => {
                self.do_list_messages_in_room(&room_id).await
            }
            AppCmdEvent::SendMessage(msg_to_send) => self.do_send_message(&msg_to_send).await,
            AppCmdEvent::UpdateRoom(room_id) => self.do_refresh_room(&room_id).await,
            // AppCmdEvent::Quit() => self.do_quit().await,
        };
        {
            self.app.lock().await.state.is_loading = false;
        }
    }

    /// Calls back `cb_teams_initialized` on `app`.
    /// This is useful to inform the main thread that the `teams` thread is ready.
    async fn do_initialize(&mut self) {
        info!("🚀 Initializing to Webex");
        let mut app = self.app.lock().await;
        app.cb_teams_initialized();
        info!("👍 Webex initialization successful");
    }

    /// Sends `msg_to_send` and calls back `cb_message_sent` on app when done.
    async fn do_send_message(&mut self, msg_to_send: &MessageOut) {
        debug!("Going to send message");
        self.client
            .send_message(msg_to_send)
            .await
            .expect("Error sending message");
        let mut app = self.app.lock().await;
        app.cb_message_sent();
    }

    async fn do_refresh_room(&mut self, id: &RoomId) {
        let global_id = GlobalId::new(GlobalIdType::Room, id.to_owned()).unwrap();
        debug!(
            "Getting room with local id {} and global id: {:?}",
            id, global_id
        );
        match self.client.get::<Room>(&global_id).await {
            Ok(room) => {
                let mut app = self.app.lock().await;
                app.cb_room_updated(room);
            }
            Err(error) => error!("Error retrieving room: {}", error),
        }
    }

    /// Gets all the rooms, update the store and set the active pane to Rooms list.
    async fn do_list_all_rooms(&mut self) {
        debug!("Going to retrieve the list of all rooms");
        let rooms = self.client.get_all_rooms().await;
        let mut app = self.app.lock().await;
        for room in rooms.unwrap_or_default() {
            app.cb_room_updated(room)
        }
        app.state
            .set_active_pane_and_actions(Some(ActivePane::Rooms));
    }

    async fn do_list_messages_in_room(&mut self, id: &RoomId) {
        debug!("Getting messages in room {}", id);
        let gid = GlobalId::new(GlobalIdType::Room, id.to_owned()).unwrap();
        match self.client.list_messages_in_room(&gid).await {
            Ok(messages) => {
                let mut app = self.app.lock().await;
                // add messages but do not mark the room as unread
                app.cb_messages_received(&messages, false).await;
            }
            Err(error) => error!("Error retrieving messages in room: {}", error),
        }
    }
}
