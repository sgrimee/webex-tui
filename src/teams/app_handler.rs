use crate::app::teams_store::RoomId;

use super::Teams;
use color_eyre::eyre::Result;
use log::{debug, error, info};
use webex::{GlobalId, GlobalIdType, MessageOut, Room};

#[derive(Debug)]
pub enum AppCmdEvent {
    Initialize(), // Launch to initiate login to Webex
    SendMessage(MessageOut),
    UpdateRoom(RoomId),
    GetAllRooms(),
}

impl Teams<'_> {
    /// Handle events dispatched by the App
    pub async fn handle_app_event(&mut self, app_cmd_event: AppCmdEvent) {
        let result = match app_cmd_event {
            AppCmdEvent::Initialize() => self.do_initialize().await,
            AppCmdEvent::SendMessage(msg_to_send) => self.do_send_message(msg_to_send).await,
            AppCmdEvent::UpdateRoom(room_id) => self.do_update_room(room_id).await,
            AppCmdEvent::GetAllRooms() => self.do_get_all_rooms().await,
        };

        if let Err(err) = result {
            error!("Oops, something wrong happened: {:?}", err);
        }

        let mut app = self.app.lock().await;
        app.loaded();
    }

    async fn do_initialize(&mut self) -> Result<()> {
        info!("🚀 Initializing to Webex");
        let mut app = self.app.lock().await;
        app.set_state_initialized().await;
        info!("👍 Webex initialization successful");
        Ok(())
    }

    async fn do_send_message(&mut self, msg_to_send: webex::types::MessageOut) -> Result<()> {
        debug!("I would like to send");
        self.client
            .send_message(&msg_to_send)
            .await
            .expect("do_send_message");
        let mut app = self.app.lock().await;
        app.message_sent();
        Ok(())
    }

    async fn do_update_room(&mut self, id: RoomId) -> Result<()> {
        debug!("Going to refresh room id: {}", id);
        let id = GlobalId::new(GlobalIdType::Room, id).unwrap();
        let room = self.client.get::<Room>(&id).await.expect("updating room");
        let mut app = self.app.lock().await;
        app.room_updated(room);
        Ok(())
    }

    async fn do_get_all_rooms(&mut self) -> Result<()> {
        debug!("Going to retrieve the list of all rooms");
        let rooms = self.client.get_all_rooms().await;
        let mut app = self.app.lock().await;
        for room in rooms.unwrap_or_default() {
            app.room_updated(room)
        }
        app.set_state_rooms_loaded().await;
        Ok(())
    }
}
