// app/teams_store/mod.rs

//! A caching store for Webex messages and context.

use chrono::{DateTime, Duration, Utc};
use color_eyre::{eyre::eyre, Result};
use log::*;
use std::collections::{HashMap, HashSet};
use webex::{Message, Room};

pub mod msg_thread;
pub mod room_content;

use self::room_content::RoomContent;

use super::rooms_list::RoomsListFilter;

pub type RoomId = String;
pub type MessageId = String;

/// `TeamsStore` maintains a local cache of room information,
/// messages in some of those rooms, and other state information
/// directly related to them.
///
/// Currently there is no garbage collection and the cache only grows.
/// This is usually acceptable for a daily session.
#[derive(Default, Debug)]
pub struct TeamsStore {
    rooms_by_id: HashMap<RoomId, Room>,
    room_content_by_room_id: HashMap<RoomId, RoomContent>,
    unread_rooms: HashSet<RoomId>,
}

impl TeamsStore {
    /// Adds a message to the store, respecting the thread order.
    pub fn add_message(&mut self, msg: &Message) -> Result<()> {
        let room_id = msg.room_id.clone().ok_or(eyre!("message has no room id"))?;
        let content = self.room_content_by_room_id.entry(room_id).or_default();
        content.add(msg)?;
        Ok(())
    }

    /// Returns the room for given id, if found.
    pub fn room_with_id(&self, id: &RoomId) -> Option<&Room> {
        self.rooms_by_id.get(id)
    }

    /// Adds or update a room to the store.
    pub fn update_room(&mut self, room: Room) {
        self.rooms_by_id.insert(room.id.to_owned(), room);
    }

    /// Mark a room as unread.
    pub fn mark_unread(&mut self, id: &RoomId) {
        debug!("Marking room {} unread", id);
        self.unread_rooms.insert(id.clone());
    }

    /// Mark a room as read.
    pub fn mark_read(&mut self, id: &RoomId) {
        debug!("Marking room {} read", id);
        self.unread_rooms.remove(id);
    }

    /// Returns whether the room has unread messages.
    pub fn room_has_unread(&self, id: &RoomId) -> bool {
        self.unread_rooms.contains(id)
    }

    /// Returns whether the room has seen any activity in the past specified period.
    /// Panics if room is not known.
    fn room_has_activity_since(&self, duration: Duration, id: &RoomId) -> bool {
        let room = self.rooms_by_id.get(id).unwrap();
        let last_activity = DateTime::parse_from_rfc3339(&room.last_activity).unwrap();
        let now = Utc::now();
        last_activity > (now - duration)
    }

    /// Returns whether a room is a 1-1 chat
    // panics if room is not known
    fn room_is_direct(&self, id: &RoomId) -> bool {
        let room = self.rooms_by_id.get(id).unwrap();
        room.room_type == "direct"
    }

    /// Returns whether a room is a space.
    /// panics if room is not known.
    fn room_is_space(&self, id: &RoomId) -> bool {
        let room = self.rooms_by_id.get(id).unwrap();
        room.room_type == "group"
    }

    /// Returns an iterator to rooms with the given filter.
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

    /// Returns an iterator with all pre-loaded messages in the room, in display order.
    pub fn messages_in_room<'a>(&'a self, id: &RoomId) -> Box<dyn Iterator<Item = &Message> + 'a> {
        match self.room_content_by_room_id.get(id) {
            Some(content) => Box::new(content.messages()),
            None => Box::new(::std::iter::empty()),
        }
    }

    /// Returns whether there are any messages in the room.
    pub fn room_is_empty(&self, id: &RoomId) -> bool {
        match self.room_content_by_room_id.get(id) {
            Some(content) => content.is_empty(),
            None => true,
        }
    }

    /// Returns the number of messages in the room.
    /// More efficient than `messages_in_room` if only the count is needed.
    pub fn nb_messages_in_room(&self, id: &RoomId) -> usize {
        match self.room_content_by_room_id.get(id) {
            Some(content) => content.len(),
            None => 0,
        }
    }

    /// Deletes message with `msg_id` in `room_id` if it exists.
    pub fn delete_message(&mut self, msg_id: &MessageId, room_id: &RoomId) -> Result<()> {
        if let Some(content) = self.room_content_by_room_id.get_mut(room_id) {
            content.delete_message(msg_id)?;
        }
        Ok(())
    }

    pub(crate) fn nth_message_in_room(&self, index: usize, room_id: &str) -> Result<&Message> {
        self.room_content_by_room_id
            .get(room_id)
            .ok_or(eyre!("Room {} not found", index))?
            .nth_message(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_message(id: &str, room_id: &str, parent_id: Option<&str>) -> Message {
        Message {
            id: Some(id.to_string()),
            parent_id: parent_id.map(|x| x.to_string()),
            room_id: Some(room_id.to_string()),
            created: Some(chrono::Utc::now().to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn should_add_message_with_unknown_room() {
        let mut store = TeamsStore::default();
        let room_id = "some_new_room_id";
        let message1 = make_message("message1", room_id, None);
        store.add_message(&message1).unwrap();
        assert_eq!(store.room_content_by_room_id[room_id].len(), 1);
    }

    #[test]
    fn should_add_message_with_known_room() {
        let mut store = TeamsStore::default();
        let room_id = "some_new_room_id";
        let message1 = make_message("message1", room_id, None);
        // add the message once to the empty store
        store.add_message(&message1).unwrap();
        // add the message again, it should get added
        store.add_message(&message1).unwrap();
        assert_eq!(store.room_content_by_room_id[room_id].len(), 2);
    }

    #[test]
    fn should_sort_messages_by_thread() {
        let mut store = TeamsStore::default();
        let room_id: RoomId = "some_new_room_id".into();
        store
            .add_message(&make_message("message1", &room_id, None))
            .unwrap();
        store
            .add_message(&make_message("message2", &room_id, None))
            .unwrap();
        store
            .add_message(&make_message("child_of_1", &room_id, Some("message1")))
            .unwrap();
        let expected = [
            "message1".to_string(),
            "child_of_1".to_string(),
            "message2".to_string(),
        ];
        for (i, msg) in store.messages_in_room(&room_id).enumerate() {
            assert_eq!(&expected[i], msg.id.as_ref().unwrap());
        }
    }
}
