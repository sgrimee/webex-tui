// app/teams_store/room_content.rs

use color_eyre::{eyre::eyre, Result};
use log::*;
use webex::Message;

use super::{msg_thread::MsgThread, MessageId};

/// `RoomContent` is a list of `MsgThread`s (conversations)
/// ordered by creation time. Messages within a same thread
/// are kept together after the first message in the thread.

#[derive(Default, Debug)]
pub(crate) struct RoomContent {
    threads: Vec<MsgThread>,
}

impl RoomContent {
    /// Returns an iterator to all messages in the room in the order they should be displayed:
    /// ordered by creation time and with threads grouped together.
    pub(crate) fn messages(&self) -> impl Iterator<Item = &Message> {
        self.threads.iter().flat_map(|thread| thread.messages())
    }

    /// Returns the message with the given index in display order, if any.
    pub(crate) fn nth_message(&self, index: usize) -> Result<&Message> {
        self.messages()
            .nth(index)
            .ok_or(eyre!("Message {} not found in room", index))
    }

    /// Adds a message to the room content, respecting the thread order.
    pub(crate) fn add(&mut self, msg: &Message) -> Result<()> {
        if msg.id.is_none() {
            return Err(eyre!("The message does not have an id"));
        }
        if msg.created.is_none() {
            return Err(eyre!("The message does not have a created date"));
        }

        // If a message exists with that id in any of the threads, update it
        for thread in self.threads.iter_mut() {
            if thread.update_if_exists(msg)? {
                debug!("Updating existing message: {:?}", msg.id);
                return Ok(());
            }
        }

        // Try to find an existing thread with that id, or create a new one
        let thread_id = msg.parent_id.clone().or(msg.id.clone());
        let index = self
            .threads
            .iter()
            .position(|thread| thread.id() == thread_id.as_ref());
        match index {
            Some(i) => {
                self.threads[i].update_or_add(msg)?;
            }
            None => {
                // No thread with that id, create a new one and place it in chronological order
                // based on the creation time of the first message in the thread.
                let mut thread = MsgThread::default();
                thread.update_or_add(msg)?;
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

    pub(crate) fn len(&self) -> usize {
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
        assert_eq!(actual_ids, expected_ids);
    }

    #[test]
    fn update_msg_in_thread_wo_parent_id_should_not_create_new_thread() {
        let mut room_content = RoomContent::default();
        let parent = make_message("parent", None);
        let child = make_message("child", Some("parent"));
        room_content.add(&parent).unwrap();
        room_content.add(&child).unwrap();
        assert_eq!(room_content.threads.len(), 1);
        // update received for child does not have the parent_id set
        let child_update = make_message("child", None);
        room_content.add(&child_update).unwrap();
        assert_eq!(room_content.threads.len(), 1);
    }

    #[test]
    fn test_nth_message() {
        let mut room_content = RoomContent::default();
        let parent = make_message("parent", None);
        let child = make_message("child", Some("parent"));
        room_content.add(&parent).unwrap();
        room_content.add(&child).unwrap();
        assert_eq!(
            room_content.nth_message(0).unwrap().id,
            Some("parent".into())
        );
        assert_eq!(
            room_content.nth_message(1).unwrap().id,
            Some("child".into())
        );
    }

    #[test]
    fn test_delete_message() {
        let mut room_content = RoomContent::default();
        let parent = make_message("parent", None);
        let child = make_message("child", Some("parent"));
        room_content.add(&parent).unwrap();
        room_content.add(&child).unwrap();
        assert_eq!(room_content.len(), 2);
        room_content.delete_message(&"parent".into()).unwrap();
        assert_eq!(room_content.len(), 1);
        room_content.delete_message(&"child".into()).unwrap();
        assert_eq!(room_content.len(), 0);
    }
}
