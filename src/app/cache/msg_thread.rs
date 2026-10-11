use color_eyre::{eyre::eyre, Result};

use crate::frontend::data::MessageSnapshot;

use super::MessageId;

/// Thread-ordered frontend-neutral message snapshots.
#[derive(Default)]
pub(crate) struct MsgThread {
    messages: Vec<MessageSnapshot>,
    id: Option<MessageId>,
}

impl MsgThread {
    pub(crate) fn update_or_add(&mut self, message: &MessageSnapshot) -> Result<()> {
        if !self.update_if_exists(message)? {
            self.check_and_update_thread_id(message)?;
            let position = self
                .messages
                .partition_point(|current| current.created_at < message.created_at);
            self.messages.insert(position, message.clone());
        }
        Ok(())
    }

    pub(crate) fn update_if_exists(&mut self, message: &MessageSnapshot) -> Result<bool> {
        if message.created_at.is_none() {
            return Err(eyre!("The message does not have a created date"));
        }
        if let Some(index) = self
            .messages
            .iter()
            .position(|current| current.id == message.id)
        {
            let mut replacement = message.clone();
            replacement
                .parent_id
                .clone_from(&self.messages[index].parent_id);
            self.messages[index] = replacement;
            return Ok(true);
        }
        Ok(false)
    }

    fn check_and_update_thread_id(&mut self, message: &MessageSnapshot) -> Result<()> {
        let id = message.id.as_str();
        match (self.id(), message.parent_id.as_ref()) {
            (Some(thread), Some(parent)) if thread != parent.as_str() => {
                Err(eyre!("Thread parent mismatch"))
            }
            (Some(_), Some(_)) => Ok(()),
            (Some(thread), None) if thread == id => Ok(()),
            (Some(_), None) => Err(eyre!("Thread root mismatch")),
            (None, Some(parent)) => {
                self.id = Some(parent.as_str().into());
                Ok(())
            }
            (None, None) => {
                self.id = Some(id.into());
                Ok(())
            }
        }
    }

    pub(crate) fn creation_time_of_first_message(&self) -> Option<&str> {
        self.messages
            .first()
            .and_then(|message| message.created_at.as_deref())
    }
    pub(crate) fn messages(&self) -> impl Iterator<Item = &MessageSnapshot> {
        self.messages.iter()
    }
    pub(crate) fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }
    #[allow(dead_code)]
    pub(crate) fn delete_message(&mut self, id: &MessageId) -> bool {
        if let Some(index) = self
            .messages
            .iter()
            .position(|message| message.id.as_str() == id)
        {
            self.messages.remove(index);
            true
        } else {
            false
        }
    }
    pub(crate) fn len(&self) -> usize {
        self.messages.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::data::{MessageId as Id, MessageStatus, RoomId};
    fn message(id: &str, parent: Option<&str>, created: &str) -> MessageSnapshot {
        MessageSnapshot {
            id: Id::new(id),
            room_id: RoomId::new("room"),
            revision: None,
            parent_id: parent.map(Id::new),
            sender_id: None,
            sender_email: None,
            created_at: Some(created.into()),
            updated_at: None,
            status: MessageStatus::Active,
            body: None,
            attachment_count: None,
        }
    }
    #[test]
    fn preserves_parent_on_revision_and_orders_deterministically() {
        let mut thread = MsgThread::default();
        thread
            .update_or_add(&message("reply", Some("root"), "2026-01-01T00:02:00Z"))
            .unwrap();
        thread
            .update_or_add(&message("root", None, "2026-01-01T00:01:00Z"))
            .unwrap();
        thread
            .update_if_exists(&message("reply", None, "2026-01-01T00:03:00Z"))
            .unwrap();
        assert_eq!(
            thread.messages().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["root", "reply"]
        );
        assert_eq!(
            thread
                .messages()
                .nth(1)
                .unwrap()
                .parent_id
                .as_ref()
                .unwrap()
                .as_str(),
            "root"
        );
    }
    #[test]
    fn duplicate_does_not_grow() {
        let mut t = MsgThread::default();
        let m = message("a", None, "2026-01-01T00:00:00Z");
        t.update_or_add(&m).unwrap();
        t.update_or_add(&m).unwrap();
        assert_eq!(t.len(), 1);
    }
    #[test]
    fn different_root_is_rejected() {
        let mut t = MsgThread::default();
        t.update_or_add(&message("a", None, "2026-01-01T00:00:00Z"))
            .unwrap();
        assert!(t
            .update_or_add(&message("b", None, "2026-01-01T00:01:00Z"))
            .is_err());
    }
    #[test]
    fn mismatched_parent_is_rejected() {
        let mut t = MsgThread::default();
        t.update_or_add(&message("a", None, "2026-01-01T00:00:00Z"))
            .unwrap();
        assert!(t
            .update_or_add(&message("b", Some("other"), "2026-01-01T00:01:00Z"))
            .is_err());
    }
    #[test]
    fn missing_created_is_rejected() {
        let mut t = MsgThread::default();
        let mut m = message("a", None, "2026-01-01T00:00:00Z");
        m.created_at = None;
        assert!(t.update_if_exists(&m).is_err());
    }
    #[test]
    fn creation_time_uses_first() {
        let mut t = MsgThread::default();
        t.update_or_add(&message("a", None, "2026-01-01T00:00:00Z"))
            .unwrap();
        assert_eq!(
            t.creation_time_of_first_message(),
            Some("2026-01-01T00:00:00Z")
        );
    }
    #[test]
    fn delete_returns_boolean() {
        let mut t = MsgThread::default();
        t.update_or_add(&message("a", None, "2026-01-01T00:00:00Z"))
            .unwrap();
        assert!(t.delete_message(&"a".into()));
        assert!(!t.delete_message(&"a".into()));
    }
    #[test]
    fn child_first_sets_parent_thread() {
        let mut t = MsgThread::default();
        t.update_or_add(&message("child", Some("root"), "2026-01-01T00:00:00Z"))
            .unwrap();
        assert_eq!(t.id(), Some("root"));
    }
}
