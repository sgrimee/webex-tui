// app/callbacks.rs

//! Callback functions called by the `Teams` thread (under locking)

use super::App;
use crate::teams::app_handler::AppCmdEvent;

use log::*;
use std::collections::HashSet;
use webex::{Message, Person};

impl App<'_> {
    /// Deselects all active panes and initialise the retrieval of all rooms
    pub fn cb_teams_initialized(&mut self) {
        self.state.set_active_pane(None);
        // Some more heavy tasks that we put after init to ensure quick startup
        self.dispatch_to_teams(AppCmdEvent::ListAllRooms());
    }

    /// Saves `me` as the user of the client
    /// This is used to identify when a message was originated by that user.
    pub fn cb_set_me(&mut self, person: Person) {
        self.state.set_me(person);
    }

    /// Callback when a message was sent. Add the message to the room immediately.
    pub fn cb_message_sent(&mut self, message: &Message) {
        self.cb_message_received(message, false);
    }

    /// Stores a single received message
    pub fn cb_message_received(&mut self, msg: &Message, mark_unread: bool) {
        let messages: [Message; 1] = [msg.clone()];
        self.cb_messages_received(&messages, mark_unread)
    }

    /// Stores multiple received messages
    pub fn cb_messages_received(&mut self, messages: &[Message], mark_unread: bool) {
        // keep track of rooms we add messages to
        let mut room_ids = HashSet::new();
        // messages came in with most recent first, so reverse them
        for msg in messages.iter().rev() {
            if let Some(id) = &msg.room_id {
                room_ids.insert(id);
                if mark_unread && !self.state.is_me(&msg.person_id) {
                    self.state.teams_store.rooms_info.mark_unread(id);
                }
            }
            if let Err(err) = self.state.teams_store.add_message(msg) {
                error!("Error adding received message to store: {}", err);
            }
        }
        // update room details, including title, adding room if needed
        // TODO: use events for room updates, maintain last_activity locally
        for room_id in room_ids {
            self.dispatch_to_teams(AppCmdEvent::UpdateRoom(room_id.to_owned()));
        }
    }

    /// Callback when room information is received.
    /// Saves the room info in the store and adjusts the position
    /// of the selector in the list.
    pub fn cb_room_updated(&mut self, webex_room: webex::Room) {
        self.state
            .teams_store
            .rooms_info
            .update_with_webex_room(webex_room);
        self.state.update_selection_with_active_room();
    }
}
