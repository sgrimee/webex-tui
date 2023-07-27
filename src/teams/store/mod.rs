use std::collections::HashMap;

use log::warn;
use webex::Message;

// A caching (in the future) store for Webex messages and context
#[derive(Default, Debug)]
pub struct Store {
    msg_per_room: HashMap<String, Vec<Message>>,
}

impl Store {
    pub fn add_message(&mut self, msg: Message) {
        let m = msg.clone();
        if let Some(room_id) = msg.room_id {
            self.msg_per_room
                .entry(room_id)
                .and_modify(|messages| messages.push(m.clone()))
                .or_insert(vec![m]);
        } else {
            warn!("Message with no room_id: {:#?}", msg);
        }
    }

    pub fn messages_in_room(&self, room_id: &str) -> Vec<Message> {
        let empty_vec: Vec<Message> = vec![];
        self.msg_per_room
            .get(room_id)
            .unwrap_or(&empty_vec)
            .to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_add_message_with_unknown_room() {
        let mut store = Store::default();
        let room_id = "some_new_room_id";
        let message = Message {
            room_id: Some(room_id.to_string()),
            ..Default::default()
        };
        store.add_message(message);
        assert_eq!(store.msg_per_room[room_id].len(), 1);
    }

    #[test]
    fn should_add_message_with_known_room() {
        let mut store = Store::default();
        let room_id = "some_new_room_id";
        let message = Message {
            room_id: Some(room_id.to_string()),
            ..Default::default()
        };
        // add the message once to the empty store
        store.add_message(message.clone());
        // add the message again, it should get added
        store.add_message(message);
        assert_eq!(store.msg_per_room[room_id].len(), 2);
    }
}
