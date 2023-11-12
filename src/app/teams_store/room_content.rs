// app/teams_store/room_content.rs

use color_eyre::{eyre::eyre, Result};
use webex::Message;

use super::{msg_thread::MsgThread, MessageId};

/// `RoomContent` is a list of `MsgThread`s (conversations)
/// ordered by creation time. Messages within a same thread
/// are kept together after the first message in the thread.

#[derive(Default, Debug)]
pub struct RoomContent {
    threads: Vec<MsgThread>,
}

impl RoomContent {
    /// Returns an iterator to all messages in the room in the order they should be displayed:
    /// ordered by creation time and with threads grouped together.
    pub fn messages(&self) -> impl Iterator<Item = &Message> {
        self.threads.iter().flat_map(|thread| thread.messages())
    }

    pub fn nth_message(&self, index: usize) -> Result<&Message> {
        self.messages()
            .nth(index)
            .ok_or(eyre!("Message {} not found in room", index))
    }

    /// Adds a message to the room content, respecting the thread order.
    pub fn add(&mut self, msg: &Message) -> Result<()> {
        if msg.id.is_none() {
            return Err(eyre!("The message does not have an id"));
        }
        if msg.created.is_none() {
            return Err(eyre!("The message does not have a created date"));
        }
        let thread_id = msg.parent_id.clone().or(msg.id.clone());
        // Try to find an existing thread with that id
        let index = self
            .threads
            .iter()
            .position(|thread| thread.id() == thread_id.as_ref());
        match index {
            Some(i) => {
                self.threads[i].add(msg)?;
            }
            None => {
                // No thread with that id, create a new one and place it in chronological order
                // based on the creation time of the first message in the thread.
                let mut thread = MsgThread::default();
                thread.add(msg)?;
                let pos = self.threads.partition_point(|t| {
                    t.creation_time_of_first_message() < thread.creation_time_of_first_message()
                });
                self.threads.insert(pos, thread);
            }
        }
        Ok(())
    }

    pub(crate) fn delete_message(&mut self, msg_id: &MessageId) -> Result<()> {
        for thread in self.threads.iter_mut() {
            if thread.delete_message(msg_id) {
                return Ok(());
            }
        }
        Err(eyre!("Message not found"))
    }

    pub fn len(&self) -> usize {
        self.threads.iter().map(|thread| thread.len()).sum()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use webex::Message;

    fn make_message(id: &str, parent_id: Option<&str>) -> Message {
        Message {
            id: Some(id.to_string()),
            parent_id: parent_id.map(|x| x.to_string()),
            created: Some(chrono::Utc::now().to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn out_of_order_messages_should_be_sorted() {
        let mut room_content = RoomContent::default();
        // Create messages stamped with the current time
        let message1 = make_message("message1", None);
        let message2 = make_message("message2", None);
        let child_of_1 = make_message("child_of_1", Some("message1"));
        let message3 = make_message("message3", None);

        // Add them in a different order
        room_content.add(&message3).unwrap();
        room_content.add(&message1).unwrap();
        room_content.add(&message2).unwrap();
        room_content.add(&child_of_1).unwrap();

        // Check that they are sorted
        let messages = room_content.messages();
        let expected_ids = vec!["message1", "child_of_1", "message2", "message3"];
        let actual_ids: Vec<_> = messages
            .map(|message| message.id.as_ref().unwrap())
            .collect();
        assert_eq!(expected_ids, actual_ids);
    }
}
