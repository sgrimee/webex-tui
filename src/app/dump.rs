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
            sender_email: message.sender_email.as_deref(),
            text,
            markdown,
            html,
            created_at: message.created_at.as_deref(),
        }
    }
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
}
