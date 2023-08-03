use log::*;
use std::collections::HashMap;
use webex::{Message, Person, Room};

pub(crate) type RoomId = String;

const TARGET: &str = module_path!();

/// A caching store for Webex messages and context
#[derive(Default, Debug)]
pub struct TeamsStore {
    rooms_by_id: HashMap<RoomId, Room>,
    msg_by_room_id: HashMap<RoomId, Vec<Message>>,
    me: Option<Person>,
}

impl TeamsStore {
    pub fn add_message(&mut self, msg: Message) {
        let m = msg.clone();
        if let Some(room_id) = msg.room_id {
            self.msg_by_room_id
                .entry(room_id)
                .and_modify(|messages| messages.push(m.clone()))
                .or_insert(vec![m]);
        } else {
            warn!(target: TARGET, "Message with no room_id: {:#?}", msg);
        }
    }

    pub fn update_room(&mut self, room: Room) {
        self.rooms_by_id.insert(room.id.to_owned(), room);
    }

    pub fn rooms(&self) -> impl Iterator<Item = &Room> {
        self.rooms_by_id.values()
    }

    pub fn number_of_rooms(&self) -> usize {
        self.rooms_by_id.len()
    }

    pub fn messages_in_room(&self, id: &RoomId) -> Vec<Message> {
        let empty_vec: Vec<Message> = vec![];
        self.msg_by_room_id.get(id).unwrap_or(&empty_vec).to_vec()
    }

    pub fn set_me_user(&mut self, me: Person) {
        self.me = Some(me);
    }

    /// Return true if me is not None, p is not None and p equals me
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
        store.add_message(message);
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
        store.add_message(message.clone());
        // add the message again, it should get added
        store.add_message(message);
        assert_eq!(store.msg_by_room_id[room_id].len(), 2);
    }
}
