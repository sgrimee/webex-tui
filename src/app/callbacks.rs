// app/callbacks.rs

//! Callback functions called by the `Teams` thread (under locking)

use super::{teams_store::room::RoomId, App};
use crate::teams::app_handler::AppCmdEvent;

use log::*;
use webex::{Message, Person};

impl App<'_> {
    /// Deselects all active panes and initialise the retrieval of all rooms
    pub(crate) fn cb_teams_initialized(&mut self) {
        self.state.set_active_pane(None);
        // Some more heavy tasks that we put after init to ensure quick startup
        self.dispatch_to_teams(AppCmdEvent::ListAllRooms());
    }

    /// Saves `me` as the user of the client
    /// This is used to identify when a message was originated by that user.
    pub(crate) fn cb_set_me(&mut self, person: Person) {
        self.state.set_me(person);
    }

    /// Callback when a message was sent. Add the message to the room immediately.
    pub(crate) fn cb_message_sent(&mut self, message: &Message) {
        self.cb_message_received(message, false);
    }

    /// Stores a single received message
    /// If `update_unread` is true and the messages are not from self, the room is marked as unread.
    /// Otherwise, the unread status is unchanged.
    pub(crate) fn cb_message_received(&mut self, msg: &Message, update_unread: bool) {
        match msg.room_id.clone() {
            Some(room_id) => {
                let messages: [Message; 1] = [msg.clone()];
                self.cb_messages_received_in_room(&room_id, &messages, update_unread)
            }
            None => {
                error!("Received message without room id: {:#?}", msg);
            }
        }
    }

    /// Stores multiple received messages
    /// If `update_unread` is true and the messages are not from self, the room is marked as unread.
    /// Otherwise, the unread status is unchanged.
    pub(crate) fn cb_messages_received_in_room(
        &mut self,
        room_id: &RoomId,
        messages: &[Message],
        update_unread: bool,
    ) {
        // messages came in with most recent first, so reverse them
        for msg in messages.iter().rev() {
            if update_unread && !self.state.is_me(&msg.person_id) {
                self.state.teams_store.rooms_info.mark_unread(room_id);
            }
            if let Err(err) = self.state.teams_store.add_message(msg) {
                error!("Error adding received message to store: {}", err);
            }
        }
        // TODO: use events for room updates. He we just request it once.
        // If the room doesn't exist, request room info and add it to the list of requested rooms.
        if !self
            .state
            .teams_store
            .rooms_info
            .room_exists_or_requested(room_id)
        {
            self.state
                .teams_store
                .rooms_info
                .add_requested_room(room_id.clone());
            self.dispatch_to_teams(AppCmdEvent::UpdateRoom(room_id.to_string()));
        }
    }

    /// Callback when room information is received.
    /// Saves the room info in the store and adjusts the position
    /// of the selector in the list.
    pub(crate) fn cb_room_updated(&mut self, webex_room: webex::Room) {
        self.state
            .teams_store
            .rooms_info
            .update_with_webex_room(webex_room);
        self.state.update_selection_with_active_room();
    }
}
