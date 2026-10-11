use color_eyre::{eyre::eyre, Result};

use crate::frontend::data::MessageSnapshot;

use super::{msg_thread::MsgThread, MessageId};

/// Thread-grouped frontend-neutral message cache.
#[derive(Default)]
pub(crate) struct RoomContent {
    threads: Vec<MsgThread>,
}
impl RoomContent {
    pub(crate) fn messages(&self) -> impl Iterator<Item = &MessageSnapshot> {
        self.threads.iter().flat_map(MsgThread::messages)
    }
    pub(crate) fn nth_message(&self, index: usize) -> Result<&MessageSnapshot> {
        self.messages()
            .nth(index)
            .ok_or(eyre!("Message {} not found in room", index))
    }
    pub(crate) fn index_of_message(&self, id: &MessageId) -> Option<usize> {
        self.messages()
            .position(|message| message.id.as_str() == id)
    }
    pub(crate) fn add(&mut self, message: &MessageSnapshot) -> Result<()> {
        if message.created_at.is_none() {
            return Err(eyre!("The message does not have a created date"));
        }
        for thread in &mut self.threads {
            if thread.update_if_exists(message)? {
                return Ok(());
            }
        }
        let thread_id = message
            .parent_id
            .as_ref()
            .map(|id| id.as_str())
            .unwrap_or_else(|| message.id.as_str());
        if let Some(index) = self
            .threads
            .iter()
            .position(|thread| thread.id() == Some(thread_id))
        {
            self.threads[index].update_or_add(message)?;
        } else {
            let mut thread = MsgThread::default();
            thread.update_or_add(message)?;
            let position = self.threads.partition_point(|current| {
                current.creation_time_of_first_message() < thread.creation_time_of_first_message()
            });
            self.threads.insert(position, thread);
        }
        Ok(())
    }
    pub(crate) fn delete_message(&mut self, id: &MessageId) -> Result<()> {
        for thread in &mut self.threads {
            if thread.delete_message(id) {
                return Ok(());
            }
        }
        Err(eyre!("Message not found"))
    }
    pub(crate) fn len(&self) -> usize {
        self.threads.iter().map(MsgThread::len).sum()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.threads.is_empty()
    }
    pub(crate) fn wipe(&mut self) -> usize {
        let count = self.len();
        self.threads.clear();
        count
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::{
        data::{MessageId, MessageStatus, OwnedMessageBody, RoomId},
        models::DisplayBody,
    };
    fn message(id: &str, parent: Option<&str>, created: &str) -> MessageSnapshot {
        MessageSnapshot {
            id: MessageId::new(id),
            room_id: RoomId::new("room"),
            revision: None,
            parent_id: parent.map(MessageId::new),
            sender_id: None,
            sender_email: None,
            created_at: Some(created.into()),
            updated_at: None,
            status: MessageStatus::Active,
            body: Some(OwnedMessageBody::LegacyUnknown(id.into())),
            attachment_count: None,
        }
    }
    #[test]
    fn preserves_order_and_tombstone_redaction() {
        let mut content = RoomContent::default();
        content
            .add(&message("second", None, "2026-01-01T00:02:00Z"))
            .unwrap();
        content
            .add(&message("root", None, "2026-01-01T00:01:00Z"))
            .unwrap();
        content
            .add(&message("reply", Some("root"), "2026-01-01T00:03:00Z"))
            .unwrap();
        let mut deleted = message("deleted", None, "2026-01-01T00:04:00Z");
        deleted.status = MessageStatus::Deleted;
        content.add(&deleted).unwrap();
        assert_eq!(
            content
                .messages()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["root", "reply", "second", "deleted"]
        );
        assert_eq!(
            content
                .nth_message(3)
                .unwrap()
                .presentation()
                .body
                .display(),
            DisplayBody::Missing
        );
    }
}
