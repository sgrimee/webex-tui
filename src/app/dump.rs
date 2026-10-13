//! Read-only views of cache snapshots: the rendering presentation and the
//! neutral JSON dump of a room (the `D` action).

use crate::frontend::{
    data::{MessageSnapshot, MessageStatus, OwnedMessageBody},
    models::MessagePresentation,
};

impl<'a> From<&'a MessageSnapshot> for MessagePresentation<'a> {
    fn from(message: &'a MessageSnapshot) -> Self {
        message.presentation()
    }
}

/// Neutral JSON dump of one message (metadata and body forms only).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MessageDumpView<'a> {
    id: &'a str,
    parent_id: Option<&'a str>,
    sender_id: Option<&'a str>,
    sender_email: Option<&'a str>,
    text: Option<&'a str>,
    markdown: Option<&'a str>,
    html: Option<&'a str>,
    created_at: Option<&'a str>,
}

impl<'a> From<&'a MessageSnapshot> for MessageDumpView<'a> {
    fn from(message: &'a MessageSnapshot) -> Self {
        let (text, markdown, html) = match (&message.status, &message.body) {
            (
                MessageStatus::Active,
                Some(OwnedMessageBody::Forms {
                    text,
                    markdown,
                    html,
                }),
            ) => (text.as_deref(), markdown.as_deref(), html.as_deref()),
            (MessageStatus::Active, Some(OwnedMessageBody::LegacyUnknown(text))) => {
                (Some(text.as_str()), None, None)
            }
            _ => (None, None, None),
        };
        Self {
            id: message.id.as_str(),
            parent_id: message.parent_id.as_ref().map(|id| id.as_str()),
            sender_id: message.sender_id.as_ref().map(|id| id.as_str()),
            sender_email: message.sender_email.as_deref(),
            text,
            markdown,
            html,
            created_at: message.created_at.as_deref(),
        }
    }
}

/// Write a room dump readable only by the user: it holds plaintext bodies.
pub(crate) fn write_dump(
    path: &std::path::Path,
    messages: &[MessageDumpView<'_>],
) -> color_eyre::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    // `mode` applies only on creation; tighten an older, wider file too.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer_pretty(file, messages)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::{
        data::{MessageId, RoomId},
        models::DisplayBody,
    };

    fn snapshot(status: MessageStatus, body: Option<OwnedMessageBody>) -> MessageSnapshot {
        MessageSnapshot {
            id: MessageId::new("message-a"),
            room_id: RoomId::new("room-a"),
            revision: None,
            parent_id: None,
            sender_id: None,
            sender_email: Some("person@example.invalid".into()),
            created_at: Some("2026-01-01T00:00:00Z".into()),
            updated_at: None,
            status,
            body,
            attachment_count: Some(1),
        }
    }

    #[test]
    fn snapshot_projection_preserves_forms_and_tombstone_redaction() {
        let active = snapshot(
            MessageStatus::Active,
            Some(OwnedMessageBody::Forms {
                text: Some("plain".into()),
                markdown: Some("**markdown**".into()),
                html: Some("<p>html</p>".into()),
            }),
        );
        assert_eq!(
            MessagePresentation::from(&active).body.display(),
            DisplayBody::Html("<p>html</p>")
        );
        let deleted = snapshot(
            MessageStatus::Deleted,
            Some(OwnedMessageBody::LegacyUnknown("old body".into())),
        );
        let dump = MessageDumpView::from(&deleted);
        assert_eq!(
            MessagePresentation::from(&deleted).body.display(),
            DisplayBody::Missing
        );
        assert!(dump.text.is_none());
        assert!(dump.html.is_none());
    }

    #[test]
    fn the_dump_carries_thread_and_sender_ids_and_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let mut reply = snapshot(
            MessageStatus::Active,
            Some(OwnedMessageBody::Forms {
                text: Some("body".into()),
                markdown: None,
                html: None,
            }),
        );
        reply.parent_id = Some(MessageId::new("root-a"));
        reply.sender_id = Some(crate::frontend::data::PersonId::new("person-a"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("room.json");
        // An older dump with wider permissions is tightened too.
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_dump(&path, &[MessageDumpView::from(&reply)]).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json[0]["parentId"], "root-a");
        assert_eq!(json[0]["senderId"], "person-a");
        assert_eq!(json[0]["text"], "body");
    }
}
