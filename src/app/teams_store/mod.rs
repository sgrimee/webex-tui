// app/teams_store/mod.rs

//! A caching store for Webex messages and context.

use std::collections::HashMap;

use color_eyre::{eyre::eyre, Result};
use webex::Message;

pub mod msg_thread;
pub mod room;
pub mod room_content;
pub mod room_list_filter;
pub mod rooms;

use self::room_content::RoomContent;
use room::RoomId;
use rooms::Rooms;

pub type MessageId = String;

/// `TeamsStore` maintains a local cache of room information,
/// messages in some of those rooms, and other state information
/// directly related to them.
///
/// Currently there is no garbage collection and the cache only grows.
/// This is usually acceptable for a daily session.
#[derive(Default, Debug)]
pub struct TeamsStore {
    pub rooms_info: Rooms,
    rooms_content: HashMap<RoomId, RoomContent>,
}

impl TeamsStore {
    /// Adds a message to the store, respecting the thread order.
    pub fn add_message(&mut self, msg: &Message) -> Result<()> {
        let room_id = msg.room_id.clone().ok_or(eyre!("message has no room id"))?;
        let content = self.rooms_content.entry(room_id).or_default();
        content.add(msg)?;
        Ok(())
    }

    /// Returns an iterator with all pre-loaded messages in the room, in display order.
    pub fn messages_in_room<'a>(&'a self, id: &RoomId) -> Box<dyn Iterator<Item = &Message> + 'a> {
        match self.rooms_content.get(id) {
            Some(content) => Box::new(content.messages()),
            None => Box::new(::std::iter::empty()),
        }
    }

    /// Returns whether there are any messages in the room.
    pub fn room_is_empty(&self, id: &RoomId) -> bool {
        match self.rooms_content.get(id) {
            Some(content) => content.is_empty(),
            None => true,
        }
    }

    /// Returns the number of messages in the room.
    /// More efficient than `messages_in_room` if only the count is needed.
    pub fn nb_messages_in_room(&self, id: &RoomId) -> usize {
        match self.rooms_content.get(id) {
            Some(content) => content.len(),
            None => 0,
        }
    }

    /// Deletes message with `msg_id` in `room_id` if it exists.
    pub fn delete_message(&mut self, msg_id: &MessageId, room_id: &RoomId) -> Result<()> {
        if let Some(content) = self.rooms_content.get_mut(room_id) {
            content.delete_message(msg_id)?;
        }
        Ok(())
    }

    pub(crate) fn nth_message_in_room(&self, index: usize, room_id: &str) -> Result<&Message> {
        self.rooms_content
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
        assert_eq!(store.rooms_content[room_id].len(), 1);
    }

    #[test]
    fn should_add_message_with_known_room() {
        let mut store = TeamsStore::default();
        let room_id = "some_new_room_id";
        let message1 = make_message("message1", room_id, None);
        // add the message once to the empty store
        store.add_message(&message1).unwrap();
        // add another message to the same room
        let message2 = make_message("message2", room_id, None);
        store.add_message(&message2).unwrap();
        assert_eq!(store.rooms_content[room_id].len(), 2);
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
