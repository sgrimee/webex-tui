use chrono::{DateTime, Duration, Utc};
use log::*;
use std::collections::{HashMap, HashSet};
use webex::{Message, Person, Room};

use super::rooms_list::RoomsListFilter;

pub(crate) type RoomId = String;

/// A caching store for Webex messages and context
#[derive(Default, Debug)]
pub struct TeamsStore {
    rooms_by_id: HashMap<RoomId, Room>,
    msg_by_room_id: HashMap<RoomId, Vec<Message>>,
    me: Option<Person>,
    unread_rooms: HashSet<RoomId>,
}

impl TeamsStore {
    pub fn add_message(&mut self, msg: &Message) {
        if let Some(room_id) = msg.room_id.clone() {
            let messages = self.msg_by_room_id.entry(room_id.clone()).or_default();
            messages.push(msg.clone());
        } else {
            warn!("Message with no room_id: {:#?}", msg);
        }
    }

    // Return the room for given id, if found
    pub fn room_with_id(&self, id: &RoomId) -> Option<&Room> {
        self.rooms_by_id.get(id)
    }

    // Add or update the room to the store
    pub fn update_room(&mut self, room: Room) {
        self.rooms_by_id.insert(room.id.to_owned(), room);
    }

    // Mark a room as unread
    pub fn mark_unread(&mut self, id: &RoomId) {
        trace!("Marking room {} unread", id);
        self.unread_rooms.insert(id.clone());
    }

    // Mark a room as read
    pub fn mark_read(&mut self, id: &RoomId) {
        trace!("Marking room {} read", id);
        self.unread_rooms.remove(id);
    }

    // Returns whether the room has unread messages
    pub fn room_has_unread(&self, id: &RoomId) -> bool {
        self.unread_rooms.contains(id)
    }

    // Returns whether the room has seen any activity in the past specified period
    // panics if room is not known
    pub fn room_has_activity_since(&self, duration: Duration, id: &RoomId) -> bool {
        let room = self.rooms_by_id.get(id).unwrap();
        let last_activity = DateTime::parse_from_rfc3339(&room.last_activity).unwrap();
        let now = Utc::now();
        last_activity > (now - duration)
    }

    // Returns whether a room is a 1-1 chat
    // panics if room is not known
    pub fn room_is_direct(&self, id: &RoomId) -> bool {
        let room = self.rooms_by_id.get(id).unwrap();
        room.room_type == "direct"
    }

    // Returns whether a room is a space
    // panics if room is not known
    pub fn room_is_space(&self, id: &RoomId) -> bool {
        let room = self.rooms_by_id.get(id).unwrap();
        room.room_type == "group"
    }

    // Return an iterator to rooms with the given filter
    #[allow(clippy::needless_lifetimes)]
    pub fn rooms_filtered_by<'a>(
        &'a self,
        filter: RoomsListFilter,
    ) -> impl Iterator<Item = &'a Room> {
        self.rooms_by_id.values().filter(move |room| match filter {
            RoomsListFilter::All => true,
            RoomsListFilter::Direct => self.room_is_direct(&room.id),
            RoomsListFilter::Recent => self.room_has_activity_since(Duration::hours(24), &room.id),
            RoomsListFilter::Spaces => self.room_is_space(&room.id),
            RoomsListFilter::Unread => self.room_has_unread(&room.id),
        })
    }

    // Return an iterator to all pre-loaded messages in the room
    // Currently by order of reception, not supporing conversations
    pub fn messages_in_room<'a>(&'a self, id: &RoomId) -> impl Iterator<Item = &'a Message> {
        self.msg_by_room_id.get(id).into_iter().flatten()
    }

    // Sets the user of the app, used to filter its own messages
    pub fn set_me_user(&mut self, me: Person) {
        self.me = Some(me);
    }

    /// Return true if me is not None, person_id is not None and person_id equals me
    /// Return false if they are different or either is None.
    pub fn is_me(&self, person_id: &Option<String>) -> bool {
        match (&self.me, person_id) {
            (Some(me), Some(id)) => me.id.eq(id),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_add_message_with_unknown_room() {
        let mut store = TeamsStore::default();
        let room_id = "some_new_room_id";
        let message = Message {
            room_id: Some(room_id.to_string()),
            ..Default::default()
        };
        store.add_message(&message);
        assert_eq!(store.msg_by_room_id[room_id].len(), 1);
    }

    #[test]
    fn should_add_message_with_known_room() {
        let mut store = TeamsStore::default();
        let room_id = "some_new_room_id";
        let message = Message {
            room_id: Some(room_id.to_string()),
            ..Default::default()
        };
        // add the message once to the empty store
        store.add_message(&message);
        // add the message again, it should get added
        store.add_message(&message);
        assert_eq!(store.msg_by_room_id[room_id].len(), 2);
    }
}
