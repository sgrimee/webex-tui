use crate::app::teams_store::RoomId;

use super::Teams;
use log::{debug, error, info};
use webex::{GlobalId, GlobalIdType, MessageOut};

#[derive(Debug)]
pub enum AppCmdEvent {
    Initialize(), // Launch to initiate login to Webex
    ListAllRooms(),
    ListMessagesInRoom(RoomId),
    SendMessage(MessageOut),
    UpdateRoom(RoomId),
    // Quit(),
}

impl Teams<'_> {
    /// Handle events dispatched by the App
    pub async fn handle_app_event(&mut self, app_cmd_event: AppCmdEvent) {
        match app_cmd_event {
            AppCmdEvent::Initialize() => self.do_initialize().await,
            AppCmdEvent::ListAllRooms() => self.do_list_all_rooms().await,
            AppCmdEvent::ListMessagesInRoom(room_id) => {
                self.do_list_messages_in_room(&room_id).await
            }
            AppCmdEvent::SendMessage(msg_to_send) => self.do_send_message(&msg_to_send).await,
            AppCmdEvent::UpdateRoom(room_id) => self.do_update_room(&room_id).await,
            // AppCmdEvent::Quit() => self.do_quit().await,
        };

        // TODO: do we need this?
        let mut app = self.app.lock().await;
        app.loaded();
    }

    async fn do_initialize(&mut self) {
        info!("🚀 Initializing to Webex");
        let mut app = self.app.lock().await;
        app.set_state_initialized().await;
        info!("👍 Webex initialization successful");
    }

    async fn do_send_message(&mut self, msg_to_send: &MessageOut) {
        debug!("Going to send message");
        self.client
            .send_message(msg_to_send)
            .await
            .expect("Error sending message");
        let mut app = self.app.lock().await;
        app.message_sent();
    }

    async fn do_update_room(&mut self, id: &RoomId) {
        self.refresh_room_roomid(id).await;
    }

    async fn do_list_all_rooms(&mut self) {
        debug!("Going to retrieve the list of all rooms");
        let rooms = self.client.get_all_rooms().await;
        let mut app = self.app.lock().await;
        for room in rooms.unwrap_or_default() {
            app.room_updated(room)
        }
        app.set_state_room_selection();
    }

    pub async fn do_list_messages_in_room(&mut self, id: &RoomId) {
        debug!("Getting messages in room {}", id);
        let gid = GlobalId::new(GlobalIdType::Room, id.to_owned()).unwrap();
        match self.client.list_messages_in_room(&gid).await {
            Ok(messages) => {
                let mut app = self.app.lock().await;
                app.messages_received(&messages).await;
            }
            Err(error) => error!("Error retrieving messages in room: {}", error),
        }
    }

    // async fn do_quit(&mut self) {
    //     debug!("Going to close webex event loop");

    // }
}
