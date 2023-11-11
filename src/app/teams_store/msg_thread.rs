// app/teams_store/sorted_messages.rs

use color_eyre::{eyre::eyre, Result};
use webex::Message;

use super::MessageId;

/// A `MsgThread` is a list of `Message`s in a room ordered by creation time,
/// where all messages starting from the second one have the first one as parent.
#[derive(Default, Debug)]
pub struct MsgThread {
    messages: Vec<Message>,
    id: Option<MessageId>,
}

impl MsgThread {
    /// Adds a message to self.messages while maintaining ordering by creation date.
    /// Returns an error if the message does not have a created date.
    pub fn add(&mut self, msg: &Message) -> Result<()> {
        if msg.created.is_none() {
            return Err(eyre!("The message does not have a created date"));
        }
        self.validate_and_update_thread_id(msg)?;
        // Insert the message in the right place
        let pos = self.messages.partition_point(|x| x.created < msg.created);
        self.messages.insert(pos, msg.clone());
        Ok(())
    }

    // If the thread has an id set already, the message parent_id must match it, if not then
    // the message id must match it (we can receive an update to the first message in the thread).
    // Otherwise, set the thread id to the parent_id of the message, or the message id if it does not have one.
    fn validate_and_update_thread_id(&mut self, msg: &Message) -> Result<()> {
        if let Some(thread_id) = self.id.clone() {
            match msg.parent_id.clone() {
                Some(parent_id) if parent_id != thread_id => {
                    return Err(eyre!(
                        "The thread is {} but the message parent is {}",
                        thread_id,
                        parent_id,
                    ));
                }
                None if msg.id != self.id => {
                    return Err(eyre!(
                        "The thread is {} but the message id is {:?}",
                        thread_id,
                        msg.id,
                    ));
                }
                _ => {}
            }
        } else {
            let thread_id = msg
                .parent_id
                .clone()
                .or(msg.id.clone())
                .ok_or(eyre!("The message does not have an id"))?;
            self.id = Some(thread_id);
        }
        Ok(())
    }

    /// Returns the creation time of the first message in the thread.
    pub fn creation_time_of_first_message(&self) -> Option<&str> {
        self.messages.first().and_then(|msg| msg.created.as_deref())
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn id(&self) -> Option<&String> {
        self.id.as_ref()
    }

    /// Deletes a message from the thread, returns true if the message was found and deleted,
    /// false otherwise.
    pub fn delete_message(&mut self, msg_id: &MessageId) -> bool {
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

    pub fn len(&self) -> usize {
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
    fn add_message_sorted_should_sort() -> Result<()> {
        let mut conversation = MsgThread::default();
        let message1 = make_message("1", None);
        let message2 = make_message("2", Some("1"));
        let message3 = make_message("3", Some("1"));
        conversation.add(&message2)?;
        conversation.add(&message1)?;
        conversation.add(&message3)?;
        let messages = conversation.messages();
        assert_eq!(messages[0].id, Some("1".to_string()));
        assert_eq!(messages[1].id, Some("2".to_string()));
        assert_eq!(messages[2].id, Some("3".to_string()));
        Ok(())
    }

    #[test]
    fn validate_and_update_thread_id_should_fail() {
        let mut conversation = MsgThread::default();
        // First message with no parent, it should set thread_id to the message id
        let msg1 = make_message("msg1", None);
        conversation.validate_and_update_thread_id(&msg1).unwrap();
        assert_eq!(Some("msg1".to_string()), conversation.id);

        // Second message with parent_id matching the thread_id, it should not change the thread_id
        let msg2 = make_message("msg2", Some("msg1"));
        conversation.validate_and_update_thread_id(&msg2).unwrap();
        assert_eq!(Some("msg1".to_string()), conversation.id);

        // Third message with parent_id not matching the thread_id, it should fail
        let msg3 = make_message("msg3", Some("other"));
        let _ = conversation
            .validate_and_update_thread_id(&msg3)
            .unwrap_err();

        // Fourth message with no parent_id, but id not matching the thread_id, it should fail
        let msg4 = make_message("msg4", None);
        let _ = conversation
            .validate_and_update_thread_id(&msg4)
            .unwrap_err();

        // Fifth message with no parent_id, but id matching the thread_id, it should not change the thread_id
        let msg5 = make_message("msg1", None);
        conversation.validate_and_update_thread_id(&msg5).unwrap();
        assert_eq!(Some("msg1".to_string()), conversation.id);

        // Reset the thread id and add a message with a parent_id, it should set the thread_id to the parent_id
        conversation.id = None;
        let msg6 = make_message("msg6", Some("msg1"));
        conversation.validate_and_update_thread_id(&msg6).unwrap();
        assert_eq!(Some("msg1".to_string()), conversation.id);
    }
}
