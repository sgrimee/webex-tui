// app/callbacks.rs

//! Callback functions called by the `Teams` thread (under locking)

use std::collections::HashSet;

use super::{
    cache::{room::RoomId, MessageId},
    App,
};
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
    pub(crate) fn cb_set_me(&mut self, person: &Person) {
        self.state.cache.set_me(person);
        self.cb_person_updated(person.to_owned());
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
        debug!(
            "Received {} messages in room {}",
            messages.len(),
            room_id.to_string()
        );
        // keep track of the selected message, if any
        let selected_message_id = self
            .state
            .selected_message()
            .ok()
            .and_then(|msg| msg.id.clone());

        self.add_messages_to_cache(messages, update_unread, room_id);
        self.request_missing_parent_messages(messages, room_id);
        self.state.update_room_selection_with_active_room();
        self.update_message_selection_in_active_room(room_id, selected_message_id);
        self.request_missing_room_info(room_id);
        self.request_missing_persons(messages);
    }

    /// Adds messages to the cache for a given room, and updates the unread status if requested and
    /// messages are not from self.
    fn add_messages_to_cache(&mut self, messages: &[Message], update_unread: bool, room_id: &String) {
        // messages came in with most recent first, reverse before adding them to cache
        for msg in messages.iter().rev() {
            if update_unread && !self.state.cache.is_me(&msg.person_id) {
                self.state.cache.rooms.mark_unread(room_id);
            }
            if let Err(err) = self.state.cache.add_message(msg) {
                error!("Error adding received message to store: {}", err);
            }
        }
    }

    /// Request info on parent messages referenced in the messages that are not in cache.
    fn request_missing_parent_messages(&mut self, messages: &[Message], room_id: &String) {
        // Identify referenced parent messages that are not in cache
        let new_parent_ids = HashSet::<&MessageId>::from_iter(
            messages
                .iter()
                .filter_map(|msg| msg.parent_id.as_ref())
                .filter(|parent_id| !self.state.cache.message_exists_in_room(parent_id, room_id)),
        );
        for parent_id in new_parent_ids {
            // get the parent message
            self.dispatch_to_teams(AppCmdEvent::UpdateMessage(parent_id.clone()));
            // we have at least one child, ensure we have all its children
            self.dispatch_to_teams(AppCmdEvent::UpdateChildrenMessages(
                parent_id.clone(),
                room_id.clone(),
            ));
        }
    }

    /// Updates the message selection in the active room.
    fn update_message_selection_in_active_room(&mut self, room_id: &String, selected_message_id: Option<String>) {
        // If the room is active, maintain the message selection as we are adding messages.
        if self.state.is_active_room(room_id) {
            if let Some(selected_message_id) = selected_message_id {
                if let Some(msg_index) = self
                    .state
                    .cache
                    .index_of_message_in_room(&selected_message_id, room_id)
                {
                    self.state.messages_list.select_index(msg_index);
                }
            }
        }
    }

    /// Request info on the room referenced in the messages if not in cache.
    fn request_missing_room_info(&mut self, room_id: &String) {
        // TODO: use events for room updates. He we just request it once.
        // If the room doesn't exist, request room info and add it to the list of requested rooms.
        if !self.state.cache.rooms.room_exists_or_requested(room_id) {
            self.state.cache.rooms.add_requested(room_id.clone());
            self.dispatch_to_teams(AppCmdEvent::UpdateRoom(room_id.to_string()));
        }
    }

    /// Request info on persons referenced in the messages that are not in cache.
    fn request_missing_persons(&mut self, messages: &[Message]) {
        // Identify referenced persons that are not in cache
        let new_person_ids = HashSet::<&String>::from_iter(
            messages
                .iter()
                .filter_map(|msg| msg.person_id.as_ref())
                .filter(|person_id| !self.state.cache.persons.exists_or_requested(person_id)),
        );
        // Request the person info and add it to the list of requested persons.
        for person_id in new_person_ids {
            self.state.cache.persons.add_requested(person_id);
            self.dispatch_to_teams(AppCmdEvent::UpdatePerson(person_id.to_owned()));
        }
    }

    /// Callback when room information is received.
    /// Saves the room info in the store and adjusts the position
    /// of the selector in the list.
    pub(crate) fn cb_room_updated(&mut self, webex_room: webex::Room) {
        let team_id = webex_room.team_id.clone();
        let room_title = webex_room.title.clone().unwrap_or_default();
        self.state.cache.rooms.update_with_room(&webex_room.into());
        self.state.update_room_selection_with_active_room();

        // If the webex_room has a team_id, and the team is not already requested, request it and add it to the list of requested teams.
        if let Some(team_id) = team_id {
            if !self.state.cache.teams.exists_or_requested(&team_id) {
                debug!(
                    "Requesting team {} identified by room: {}",
                    team_id, room_title
                );
                self.state.cache.teams.add_requested(team_id.clone());
                self.dispatch_to_teams(AppCmdEvent::UpdateTeam(team_id));
            }
        }
    }

    /// Callback when team information is received.
    /// Saves the team info in the store.
    pub(crate) fn cb_team_updated(&mut self, team: webex::Team) {
        self.state.cache.teams.add(team);
    }

    /// Callback when person information is received.
    /// Saves the person info in the store.
    pub(crate) fn cb_person_updated(&mut self, person: Person) {
        self.state.cache.persons.insert(person);
    }
}
