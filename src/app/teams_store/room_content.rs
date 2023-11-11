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
    /// Returns all messages in the room, ordered by creation time and
    /// grouping threads together.
    pub fn messages(&self) -> Vec<Message> {
        self.threads
            .iter()
            .flat_map(|thread| thread.messages().to_vec())
            .collect::<Vec<Message>>()
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
        assert_eq!(messages.len(), 4);
        let expected_ids = vec![
            Some("message1".to_string()),
            Some("child_of_1".to_string()),
            Some("message2".to_string()),
            Some("message3".to_string()),
        ];
        let actual_ids: Vec<_> = messages.into_iter().map(|message| message.id).collect();
        assert_eq!(expected_ids, actual_ids);
    }
}
