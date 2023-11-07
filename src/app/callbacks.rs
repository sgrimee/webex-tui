// app/callbacks.rs

//! Callback functions called by the `Teams` thread (under locking)

use super::App;
use crate::teams::app_handler::AppCmdEvent;

use log::*;
use std::collections::HashSet;
use webex::{Message, Person, Room};

impl App<'_> {
    /// Deselects all active panes and initialise the retrieval of all rooms
    pub fn cb_teams_initialized(&mut self) {
        self.state.set_active_pane(None);
        // Some more heavy tasks that we put after init to ensure quick startup
        self.dispatch_to_teams(AppCmdEvent::ListAllRooms());
    }

    /// Saves `me` as the user of the client
    /// This is used to identify when a message was originated by that user.
    pub fn cb_set_me_user(&mut self, me: Person) {
        self.state.teams_store.set_me_user(me);
    }

    /// Callback when a message was sent. Does nothing.
    pub fn cb_message_sent(&mut self) {
        debug!("Message was sent.");
    }

    /// Stores a single received message
    pub async fn cb_message_received(&mut self, msg: &Message, mark_unread: bool) {
        let messages: [Message; 1] = [msg.clone()];
        self.cb_messages_received(&messages, mark_unread).await
    }

    /// Stores multiple received messages
    pub async fn cb_messages_received(&mut self, messages: &[Message], mark_unread: bool) {
        // keep track of rooms we add messages to
        let mut room_ids = HashSet::new();
        // messages came in with most recent first, so reverse them
        for msg in messages.iter().rev() {
            if let Some(id) = &msg.room_id {
                room_ids.insert(id);
                if mark_unread && !self.state.teams_store.is_me(&msg.person_id) {
                    self.state.teams_store.mark_unread(id);
                }
            }
            self.state.teams_store.add_message(msg);
        }
        // update room details, including title, adding room if needed
        for room_id in room_ids {
            self.dispatch_to_teams(AppCmdEvent::UpdateRoom(room_id.to_owned()));
        }
    }

    /// Callback when room information is received.
    /// Saves the room info in the store and adjusts the position
    /// of the selector in the list.
    pub fn cb_room_updated(&mut self, room: Room) {
        self.state.teams_store.update_room(room);
        self.state.update_selection_with_active_room();
    }
}
