//! Single source of the frontend's local editor-write admission and canonical
//! success-identity rules. The ledger, mailbox and bridge projection all use
//! these predicates so their bounds cannot drift apart. The daemon remains
//! authoritative: these checks only refuse obviously invalid local work early.
//!
//! The bounds and primitive predicates are the bridge's client-neutral DTO
//! exports (also used by the daemon itself), so the two cannot drift.
//!
//! Pure predicates; no logging, no payload formatting and no Debug output.

use super::{
    data::{ComposedBody, MessageSnapshot},
    envelopes::{FrontendIntent, FrontendResult, WriteOutcome},
};
use webex_bridge::dto;

/// Current UI API body bound, shared with the daemon via the bridge DTOs.
pub(crate) const MAX_WRITE_BODY_BYTES: usize = dto::MAX_MESSAGE_BODY_BYTES;
/// Opaque resource-ID bound, shared with the daemon via the bridge DTOs.
pub(crate) const MAX_OPAQUE_ID_BYTES: usize = dto::MAX_RESOURCE_ID_BYTES;

/// Nonempty, bounded and free of control characters. IDs are never decoded.
pub(crate) fn valid_opaque_id(value: &str) -> bool {
    dto::valid_resource_id(value)
}

/// 1..=7439 UTF-8 bytes with no NUL, for either explicit body format.
pub(crate) fn valid_write_body(body: &ComposedBody) -> bool {
    let (ComposedBody::PlainText(content) | ComposedBody::Markdown(content)) = body;
    dto::valid_message_body(content)
}

/// Editor writes only (send/reply/edit): bounded IDs/anchor and body. Reads,
/// delete and leave are never admitted through the editor write path.
pub(crate) fn valid_editor_write(intent: &FrontendIntent) -> bool {
    let (room, anchor, body) = match intent {
        FrontendIntent::SendMessage {
            room_id,
            parent_id,
            body,
        } => (room_id, parent_id.as_ref(), body),
        FrontendIntent::EditMessage {
            room_id,
            message_id,
            body,
        } => (room_id, Some(message_id), body),
        _ => return false,
    };
    valid_opaque_id(room.as_str())
        && anchor.is_none_or(|id| valid_opaque_id(id.as_str()))
        && valid_write_body(body)
}

/// Any single-slot write: an editor write, or a delete/leave with bounded
/// opaque IDs. Reads are never admitted through the write path.
pub(crate) fn valid_write(intent: &FrontendIntent) -> bool {
    match intent {
        FrontendIntent::DeleteMessage {
            room_id,
            message_id,
        } => valid_opaque_id(room_id.as_str()) && valid_opaque_id(message_id.as_str()),
        FrontendIntent::LeaveRoom(room_id) => valid_opaque_id(room_id.as_str()),
        intent => valid_editor_write(intent),
    }
}

/// A canonical write success must name the exact requested room and, for a
/// send, the exact parent (None for a root); for an edit, the exact message;
/// a delete or leave success must name exactly the deleted message / left room.
/// Non-success outcomes carry no resource and are accepted here; callers check
/// the operation pairing separately through `RequestEnvelope::complete`.
pub(crate) fn write_success_matches(intent: &FrontendIntent, result: &FrontendResult) -> bool {
    match (intent, result) {
        (
            FrontendIntent::SendMessage {
                room_id, parent_id, ..
            },
            FrontendResult::SendMessage(WriteOutcome::Success(message)),
        ) => canonical(message) && message.room_id == *room_id && message.parent_id == *parent_id,
        (
            FrontendIntent::EditMessage {
                room_id,
                message_id,
                ..
            },
            FrontendResult::EditMessage(WriteOutcome::Success(message)),
        ) => canonical(message) && message.room_id == *room_id && message.id == *message_id,
        (
            FrontendIntent::DeleteMessage { message_id, .. },
            FrontendResult::DeleteMessage(WriteOutcome::Success(deleted)),
        ) => deleted == message_id,
        (
            FrontendIntent::LeaveRoom(room_id),
            FrontendResult::LeaveRoom(WriteOutcome::Success(left)),
        ) => left == room_id,
        (_, FrontendResult::SendMessage(WriteOutcome::Success(_)))
        | (_, FrontendResult::EditMessage(WriteOutcome::Success(_)))
        | (_, FrontendResult::DeleteMessage(WriteOutcome::Success(_)))
        | (_, FrontendResult::LeaveRoom(WriteOutcome::Success(_))) => false,
        _ => true,
    }
}

/// A canonical daemon result carries a valid ID and a revision; a
/// revision-less snapshot is refused by the canonical message view, so the
/// ledger must not record it as a confirmed success either.
fn canonical(message: &MessageSnapshot) -> bool {
    valid_opaque_id(message.id.as_str()) && message.revision.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::{
        data::{MessageId, MessageStatus, RoomId},
        envelopes::FrontendError,
    };

    fn snapshot(id: &str, room: &str, parent: Option<&str>) -> MessageSnapshot {
        MessageSnapshot {
            id: MessageId::new(id),
            room_id: RoomId::new(room),
            parent_id: parent.map(MessageId::new),
            revision: Some(1),
            sender_id: None,
            sender_email: None,
            created_at: None,
            updated_at: None,
            status: MessageStatus::Active,
            body: None,
            attachment_count: None,
        }
    }

    fn send(parent: Option<&str>, body: ComposedBody) -> FrontendIntent {
        FrontendIntent::SendMessage {
            room_id: RoomId::new("room/a?opaque"),
            parent_id: parent.map(MessageId::new),
            body,
        }
    }

    #[test]
    fn id_and_body_bounds_are_exact() {
        // Shared with the daemon; pin the documented UI API values.
        assert_eq!(MAX_WRITE_BODY_BYTES, 7439);
        assert_eq!(MAX_OPAQUE_ID_BYTES, 1024);
        assert!(valid_opaque_id(&"x".repeat(MAX_OPAQUE_ID_BYTES)));
        for bad in [
            "".to_owned(),
            "a\nb".into(),
            "x".repeat(MAX_OPAQUE_ID_BYTES + 1),
        ] {
            assert!(!valid_opaque_id(&bad));
        }
        // 3719 two-byte chars + 1 = exactly 7439 bytes.
        let exact = "é".repeat(3719) + "x";
        assert_eq!(exact.len(), MAX_WRITE_BODY_BYTES);
        for body in [
            ComposedBody::PlainText(exact.clone()),
            ComposedBody::Markdown(exact),
        ] {
            assert!(valid_write_body(&body));
        }
        for bad in ["".to_owned(), "nul\0".into(), "é".repeat(3720)] {
            assert!(!valid_write_body(&ComposedBody::PlainText(bad.clone())));
            assert!(!valid_write_body(&ComposedBody::Markdown(bad)));
        }
    }

    #[test]
    fn editor_write_admits_only_bounded_send_and_edit() {
        let body = ComposedBody::PlainText("private".into());
        assert!(valid_editor_write(&send(None, body.clone())));
        assert!(valid_editor_write(&send(Some("p"), body.clone())));
        assert!(!valid_editor_write(&send(Some("bad\u{7}"), body.clone())));
        assert!(!valid_editor_write(&send(
            None,
            ComposedBody::PlainText(String::new())
        )));
        assert!(valid_editor_write(&FrontendIntent::EditMessage {
            room_id: RoomId::new("r"),
            message_id: MessageId::new("m"),
            body: body.clone(),
        }));
        for other in [
            FrontendIntent::DeleteMessage {
                room_id: RoomId::new("r"),
                message_id: MessageId::new("m"),
            },
            FrontendIntent::LeaveRoom(RoomId::new("r")),
            FrontendIntent::WhoAmI,
        ] {
            assert!(!valid_editor_write(&other));
        }
    }

    #[test]
    fn success_identity_requires_exact_room_parent_and_message() {
        let body = ComposedBody::PlainText("private".into());
        let ok = |m| FrontendResult::SendMessage(WriteOutcome::Success(m));
        let root = send(None, body.clone());
        let reply = send(Some("p"), body.clone());
        assert!(write_success_matches(
            &root,
            &ok(snapshot("n", "room/a?opaque", None))
        ));
        assert!(!write_success_matches(
            &root,
            &ok(snapshot("n", "room/a?opaque", Some("p")))
        ));
        assert!(!write_success_matches(
            &root,
            &ok(snapshot("n", "room/b", None))
        ));
        assert!(!write_success_matches(
            &root,
            &ok(snapshot("", "room/a?opaque", None))
        ));
        let mut legacy = snapshot("n", "room/a?opaque", None);
        legacy.revision = None;
        assert!(!write_success_matches(&root, &ok(legacy)));
        assert!(write_success_matches(
            &reply,
            &ok(snapshot("n", "room/a?opaque", Some("p")))
        ));
        assert!(!write_success_matches(
            &reply,
            &ok(snapshot("n", "room/a?opaque", None))
        ));
        assert!(!write_success_matches(
            &reply,
            &ok(snapshot("n", "room/a?opaque", Some("q")))
        ));

        let edit = FrontendIntent::EditMessage {
            room_id: RoomId::new("r"),
            message_id: MessageId::new("m"),
            body,
        };
        let edited = |m| FrontendResult::EditMessage(WriteOutcome::Success(m));
        assert!(write_success_matches(
            &edit,
            &edited(snapshot("m", "r", None))
        ));
        assert!(!write_success_matches(
            &edit,
            &edited(snapshot("other", "r", None))
        ));
        assert!(!write_success_matches(
            &edit,
            &edited(snapshot("m", "other", None))
        ));
        // Cross-operation success is never an identity match.
        assert!(!write_success_matches(&edit, &ok(snapshot("m", "r", None))));
        assert!(!write_success_matches(
            &root,
            &edited(snapshot("n", "room/a?opaque", None))
        ));
        // Non-success outcomes carry no resource.
        let failed = FrontendResult::SendMessage(WriteOutcome::Rejected(FrontendError {
            code: "safe".into(),
            message: "safe".into(),
        }));
        assert!(write_success_matches(&root, &failed));
    }

    #[test]
    fn delete_and_leave_are_bounded_and_identity_checked() {
        let delete = FrontendIntent::DeleteMessage {
            room_id: RoomId::new("r"),
            message_id: MessageId::new("m"),
        };
        let leave = FrontendIntent::LeaveRoom(RoomId::new("r"));
        assert!(valid_write(&delete) && valid_write(&leave));
        assert!(!valid_write(&FrontendIntent::LeaveRoom(RoomId::new(""))));
        assert!(!valid_write(&FrontendIntent::DeleteMessage {
            room_id: RoomId::new("r"),
            message_id: MessageId::new("bad\nid"),
        }));
        assert!(!valid_write(&FrontendIntent::ListAllRooms));
        let deleted =
            |id: &str| FrontendResult::DeleteMessage(WriteOutcome::Success(MessageId::new(id)));
        let left = |id: &str| FrontendResult::LeaveRoom(WriteOutcome::Success(RoomId::new(id)));
        assert!(write_success_matches(&delete, &deleted("m")));
        assert!(!write_success_matches(&delete, &deleted("other")));
        assert!(write_success_matches(&leave, &left("r")));
        assert!(!write_success_matches(&leave, &left("other")));
        assert!(
            !write_success_matches(&leave, &deleted("m")),
            "cross-operation"
        );
    }
}
