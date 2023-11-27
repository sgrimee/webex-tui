// app/teams_store/sorted_messages.rs

use color_eyre::{eyre::eyre, Result};
use webex::Message;

use super::MessageId;

/// A `MsgThread` is a list of `Message`s in a room ordered by creation time,
/// where all messages starting from the second one have the first one as parent.
#[derive(Default, Debug)]
pub(crate) struct MsgThread {
    messages: Vec<Message>,
    id: Option<MessageId>,
}

impl MsgThread {
    /// Adds a message to self.messages while maintaining ordering by creation date.
    pub(crate) fn update_or_add(&mut self, msg: &Message) -> Result<()> {
        if !self.update_if_exists(msg)? {
            // Update the thread id if needed
            self.check_and_update_thread_id(msg)?;
            // It is a new message, insert it at the right place
            let pos = self.messages.partition_point(|x| x.created < msg.created);
            self.messages.insert(pos, msg.clone());
        }
        Ok(())
    }

    /// Updates the message only if it already exists in the thread.
    /// Returns true if the message was found and updated, false otherwise.
    /// Returns an error if the message does not have a created date.
    pub(crate) fn update_if_exists(&mut self, msg: &Message) -> Result<bool> {
        if msg.created.is_none() {
            return Err(eyre!("The message does not have a created date"));
        }

        // If a message already exists with same id in the list, update it and set its parent_it to that of the existing message.
        // This is because incoming message updates do not have the parent_id set correctly.
        if let Some(index) = self.messages.iter().position(|x| x.id == msg.id) {
            let mut msg = msg.clone();
            msg.parent_id = self.messages[index].parent_id.clone();
            self.messages[index] = msg;
            return Ok(true);
        }
        Ok(false)
    }

    // If the thread has an id set already, and the message parent_id is set, they must match.
    // Otherwise, if the thead is is not set, set it to the parent_id of the message,
    // or the message id if it does not have one.
    fn check_and_update_thread_id(&mut self, msg: &Message) -> Result<()> {
        let msg_id = msg
            .id
            .clone()
            .ok_or(eyre!("The message does not have an id"))?;
        match (self.id(), msg.parent_id.clone()) {
            (Some(thread_id), Some(parent_id)) if *thread_id != parent_id => Err(eyre!(
                "The thread is {} but the message parent is {}",
                thread_id,
                parent_id,
            )),
            (Some(_), Some(_)) => Ok(()),
            (Some(thread_id), None) => {
                // if the msg id is different from the thread id, return an error
                if *thread_id != msg_id {
                    Err(eyre!(
                        "The thread is {} but the message id is {}",
                        thread_id,
                        msg.id.clone().unwrap_or_default(),
                    ))
                } else {
                    Ok(())
                }
            }
            (None, Some(parent_id)) => {
                self.id = Some(parent_id);
                Ok(())
            }
            (None, _) => {
                self.id = Some(msg_id);
                Ok(())
            }
        }
    }

    /// Returns the creation time of the first message in the thread.
    pub(crate) fn creation_time_of_first_message(&self) -> Option<&str> {
        self.messages.first().and_then(|msg| msg.created.as_deref())
    }

    pub(crate) fn messages(&self) -> impl Iterator<Item = &Message> {
        self.messages.iter()
    }

    pub(crate) fn id(&self) -> Option<&String> {
        self.id.as_ref()
    }

    /// Deletes a message from the thread, returns true if the message was found and deleted,
    /// false otherwise.
    pub(crate) fn delete_message(&mut self, msg_id: &MessageId) -> bool {
        if let Some(index) = self
            .messages
            .iter()
            .position(|msg| msg.id == Some(msg_id.clone()))
        {
            self.messages.remove(index);
            return true;
        }
        false
    }

    pub(crate) fn len(&self) -> usize {
        self.messages.len()
    }
}

// implement tests for Conversation
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
    fn test_update_or_add_should_not_duplicate_messages() {
        let mut conversation = MsgThread::default();
        let msg1 = make_message("msg1", None);
        let child_of_1 = make_message("child_of_1", Some("msg1"));
        conversation.update_or_add(&msg1).unwrap();
        conversation.update_or_add(&child_of_1).unwrap();
        assert_eq!(2, conversation.len());
        conversation.update_or_add(&msg1).unwrap();
        conversation.update_or_add(&child_of_1).unwrap();
        assert_eq!(2, conversation.len());
    }

    #[test]
    fn test_update_or_add_mismatch_id() {
        let mut conversation = MsgThread::default();
        let msg1 = make_message("msg1", None);
        conversation.update_or_add(&msg1).unwrap();
        // adding a message without parent_id and with id different
        // from the thread id should fail
        let msg2 = make_message("msg2", None);
        let _ = conversation.update_or_add(&msg2).unwrap_err();
        assert_eq!(1, conversation.len());
    }

    #[test]
    fn test_update_or_add_should_sort() -> Result<()> {
        let mut conversation = MsgThread::default();
        let message1 = make_message("1", None);
        let message2 = make_message("2", Some("1"));
        let message3 = make_message("3", Some("1"));
        conversation.update_or_add(&message2)?;
        conversation.update_or_add(&message1)?;
        conversation.update_or_add(&message3)?;
        let mut messages = conversation.messages();
        assert_eq!(messages.next().unwrap().id, Some("1".to_string()));
        assert_eq!(messages.next().unwrap().id, Some("2".to_string()));
        assert_eq!(messages.next().unwrap().id, Some("3".to_string()));
        Ok(())
    }

    #[test]
    fn test_update_if_exists() {
        let mut conversation = MsgThread::default();
        let msg1 = make_message("msg1", None);
        let msg2 = make_message("msg2", Some("msg1"));
        conversation.update_or_add(&msg1).unwrap();
        conversation.update_or_add(&msg2).unwrap();
        assert_eq!(conversation.len(), 2);
        // Add a message with the same id as the first message, it should update it
        conversation.update_if_exists(&msg1).unwrap();
        assert_eq!(conversation.len(), 2);
        // Try to add a message with unknown id, it should not add it
        let msg3 = make_message("msg3", Some("msg1"));
        conversation.update_if_exists(&msg3).unwrap();
        assert_eq!(conversation.len(), 2);
    }

    #[test]
    fn test_check_and_update_thread_id() {
        let mut conversation = MsgThread::default();
        // First message with no parent, it should set thread_id to the message id
        let msg1 = make_message("msg1", None);
        conversation.check_and_update_thread_id(&msg1).unwrap();
        assert_eq!(Some("msg1".to_string()), conversation.id);

        // Second message with parent_id matching the thread_id, it should not change the thread_id
        let msg2 = make_message("msg2", Some("msg1"));
        conversation.check_and_update_thread_id(&msg2).unwrap();
        assert_eq!(Some("msg1".to_string()), conversation.id);

        // Third message with parent_id not matching the thread_id, it should fail
        let msg3 = make_message("msg3", Some("other"));
        let _ = conversation.check_and_update_thread_id(&msg3).unwrap_err();

        // Fourth message with no parent_id, but id not matching the thread_id, it not change the thread_id
        let msg4 = make_message("msg4", None);
        let _ = conversation.check_and_update_thread_id(&msg4).unwrap_err();
        assert_eq!(Some("msg1".to_string()), conversation.id);

        // Fifth message with no parent_id, but id matching the thread_id, it should not change the thread_id
        let msg5 = make_message("msg1", None);
        conversation.check_and_update_thread_id(&msg5).unwrap();
        assert_eq!(Some("msg1".to_string()), conversation.id);

        // Reset the thread id and add a message with a parent_id, it should set the thread_id to the parent_id
        conversation.id = None;
        let msg6 = make_message("msg6", Some("msg1"));
        conversation.check_and_update_thread_id(&msg6).unwrap();
        assert_eq!(Some("msg1".to_string()), conversation.id);
    }

    #[test]
    fn test_creation_time_of_first_message() {
        let mut conversation = MsgThread::default();
        assert_eq!(None, conversation.creation_time_of_first_message());
        let msg1 = make_message("msg1", None);
        conversation.update_or_add(&msg1).unwrap();
        assert_eq!(
            msg1.clone().created.unwrap(),
            conversation.creation_time_of_first_message().unwrap()
        );
        let msg2 = make_message("msg2", Some("msg1"));
        conversation.update_or_add(&msg2).unwrap();
        assert_eq!(
            msg1.created.unwrap(),
            conversation.creation_time_of_first_message().unwrap()
        );
    }

    #[test]
    fn test_delete_message() {
        let mut conversation = MsgThread::default();
        let msg1 = make_message("msg1", None);
        let msg2 = make_message("msg2", Some("msg1"));
        let msg3 = make_message("msg3", Some("msg1"));
        conversation.update_or_add(&msg1).unwrap();
        conversation.update_or_add(&msg2).unwrap();
        conversation.update_or_add(&msg3).unwrap();
        assert_eq!(3, conversation.len());
        assert!(conversation.delete_message(&"msg2".to_string()));
        assert_eq!(2, conversation.len());
        assert!(!conversation.delete_message(&"msg2".to_string()));
        assert_eq!(2, conversation.len());
    }
}
