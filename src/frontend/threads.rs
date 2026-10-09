//! Pure thread-page presentation seam for the bridge's explicit parent states.
//!
//! This does not group messages, hydrate parents or repair canonical data. It
//! validates a disposable page for its requested room/root and retains ordering,
//! cursors and orphan replies exactly as supplied by the eventual adapter.

use super::{
    data::{MessageId, MessagePage, MessageSnapshot, MessageStatus, RoomId},
    models::MessagePresentation,
};

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ThreadParent {
    Active(MessageSnapshot),
    Deleted(MessageSnapshot),
    Missing(MessageId),
    Unavailable(MessageId),
}

impl ThreadParent {
    pub(crate) fn id(&self) -> &MessageId {
        match self {
            Self::Active(message) | Self::Deleted(message) => &message.id,
            Self::Missing(id) | Self::Unavailable(id) => id,
        }
    }

    /// A missing/unavailable root is a placeholder, never a fabricated message.
    /// Deleted-root snapshots keep identity but cannot present retained bodies.
    pub(crate) fn presentation(&self) -> Option<MessagePresentation<'_>> {
        match self {
            Self::Active(message) | Self::Deleted(message) => Some(message.presentation()),
            Self::Missing(_) | Self::Unavailable(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThreadProjectionError {
    Parent,
    Room,
    ParentState,
    ReplyParent,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ThreadPage {
    pub(crate) parent: ThreadParent,
    pub(crate) replies: MessagePage,
}

impl ThreadPage {
    pub(crate) fn new(
        expected_parent: &MessageId,
        expected_room: &RoomId,
        parent: ThreadParent,
        replies: MessagePage,
    ) -> Result<Self, ThreadProjectionError> {
        if parent.id() != expected_parent {
            return Err(ThreadProjectionError::Parent);
        }
        if replies.room_id != *expected_room {
            return Err(ThreadProjectionError::Room);
        }
        match &parent {
            ThreadParent::Active(message) | ThreadParent::Deleted(message) => {
                if message.room_id != *expected_room {
                    return Err(ThreadProjectionError::Room);
                }
                let expected_status = match &parent {
                    ThreadParent::Active(_) => MessageStatus::Active,
                    _ => MessageStatus::Deleted,
                };
                if message.status != expected_status {
                    return Err(ThreadProjectionError::ParentState);
                }
            }
            ThreadParent::Missing(_) | ThreadParent::Unavailable(_) => {}
        }
        for reply in &replies.messages {
            if reply.room_id != *expected_room {
                return Err(ThreadProjectionError::Room);
            }
            if reply.id == *expected_parent || reply.parent_id.as_ref() != Some(expected_parent) {
                return Err(ThreadProjectionError::ReplyParent);
            }
        }
        Ok(Self { parent, replies })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::{
        data::{OwnedMessageBody, PageCursor},
        models::DisplayBody,
    };

    fn message(id: &str, parent: Option<&str>, status: MessageStatus) -> MessageSnapshot {
        MessageSnapshot {
            id: MessageId::new(id),
            room_id: RoomId::new("room-a"),
            revision: Some(1),
            parent_id: parent.map(MessageId::new),
            sender_id: None,
            sender_email: None,
            created_at: Some("2026-10-09T12:00:00Z".into()),
            updated_at: None,
            status,
            body: Some(OwnedMessageBody::LegacyUnknown("fixture body".into())),
            attachment_count: Some(1),
        }
    }

    fn replies() -> MessagePage {
        MessagePage {
            room_id: RoomId::new("room-a"),
            messages: vec![message("reply-a", Some("root-a"), MessageStatus::Active)],
            next_cursor: Some(PageCursor::new("opaque/reply?cursor")),
        }
    }

    fn project(
        parent: ThreadParent,
        replies: MessagePage,
    ) -> Result<ThreadPage, ThreadProjectionError> {
        ThreadPage::new(
            &MessageId::new("root-a"),
            &RoomId::new("room-a"),
            parent,
            replies,
        )
    }

    #[test]
    fn active_parent_and_reply_page_preserve_data_and_cursor() {
        let parent = ThreadParent::Active(message("root-a", None, MessageStatus::Active));
        let original = replies();
        let page = project(parent, original.clone()).unwrap();
        assert!(page.replies == original);
        assert_eq!(
            page.parent.presentation().unwrap().body.display(),
            DisplayBody::Text("fixture body")
        );
    }

    #[test]
    fn missing_and_unavailable_roots_preserve_orphan_replies() {
        for parent in [
            ThreadParent::Missing(MessageId::new("root-a")),
            ThreadParent::Unavailable(MessageId::new("root-a")),
        ] {
            let page = project(parent, replies()).unwrap();
            assert!(page.parent.presentation().is_none());
            assert_eq!(page.parent.id().as_str(), "root-a");
            assert_eq!(page.replies.messages.len(), 1);
            assert_eq!(page.replies.messages[0].id.as_str(), "reply-a");
            assert_eq!(
                page.replies.next_cursor.unwrap().as_str(),
                "opaque/reply?cursor"
            );
        }
    }

    #[test]
    fn deleted_root_has_no_presented_body_but_does_not_hide_replies() {
        let parent = ThreadParent::Deleted(message("root-a", None, MessageStatus::Deleted));
        let page = project(parent, replies()).unwrap();
        let root = page.parent.presentation().unwrap();
        assert_eq!(root.body.display(), DisplayBody::Missing);
        assert_eq!(root.attachment_count, None);
        assert_eq!(page.replies.messages.len(), 1);
        assert_eq!(
            page.replies.messages[0].presentation().body.display(),
            DisplayBody::Text("fixture body")
        );
    }

    #[test]
    fn empty_reply_page_is_valid_without_fabricating_a_root() {
        let mut children = replies();
        children.messages.clear();
        children.next_cursor = None;
        let page = project(
            ThreadParent::Unavailable(MessageId::new("root-a")),
            children,
        )
        .unwrap();
        assert!(page.replies.messages.is_empty());
        assert!(page.parent.presentation().is_none());
    }

    #[test]
    fn reverse_page_order_is_not_regrouped_or_resorted() {
        let mut children = replies();
        children
            .messages
            .insert(0, message("reply-z", Some("root-a"), MessageStatus::Active));
        let page = project(ThreadParent::Missing(MessageId::new("root-a")), children).unwrap();
        assert_eq!(
            page.replies
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["reply-z", "reply-a"]
        );
    }

    #[test]
    fn root_id_and_status_mismatches_fail_without_repair() {
        assert!(matches!(
            project(
                ThreadParent::Missing(MessageId::new("wrong-root")),
                replies()
            ),
            Err(ThreadProjectionError::Parent)
        ));
        assert!(matches!(
            project(
                ThreadParent::Active(message("root-a", None, MessageStatus::Deleted)),
                replies()
            ),
            Err(ThreadProjectionError::ParentState)
        ));
        assert!(matches!(
            project(
                ThreadParent::Deleted(message("root-a", None, MessageStatus::Active)),
                replies()
            ),
            Err(ThreadProjectionError::ParentState)
        ));
    }

    #[test]
    fn cross_room_parent_page_and_reply_are_each_rejected() {
        let mut root = message("root-a", None, MessageStatus::Active);
        root.room_id = RoomId::new("room-b");
        assert!(matches!(
            project(ThreadParent::Active(root), replies()),
            Err(ThreadProjectionError::Room)
        ));
        let mut children = replies();
        children.room_id = RoomId::new("room-b");
        assert!(matches!(
            project(ThreadParent::Missing(MessageId::new("root-a")), children),
            Err(ThreadProjectionError::Room)
        ));
        let mut children = replies();
        children.messages[0].room_id = RoomId::new("room-b");
        assert!(matches!(
            project(ThreadParent::Missing(MessageId::new("root-a")), children),
            Err(ThreadProjectionError::Room)
        ));
    }

    #[test]
    fn wrong_parent_and_root_as_reply_are_rejected() {
        let mut children = replies();
        children.messages[0].parent_id = Some(MessageId::new("wrong-root"));
        assert!(matches!(
            project(ThreadParent::Missing(MessageId::new("root-a")), children),
            Err(ThreadProjectionError::ReplyParent)
        ));
        let mut children = replies();
        children.messages[0].id = MessageId::new("root-a");
        assert!(matches!(
            project(ThreadParent::Missing(MessageId::new("root-a")), children),
            Err(ThreadProjectionError::ReplyParent)
        ));
    }
}
