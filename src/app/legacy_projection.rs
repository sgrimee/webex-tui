//! Temporary read-only mapping for the existing direct backend.
//!
//! Keep Webex fields at this boundary while render/copy logic uses neutral views.
//! This adapter owns no network, cache mutation or canonical reconciliation.

use crate::frontend::models::{MessageBody, MessagePresentation};
use webex::Message;

impl<'a> From<&'a Message> for MessagePresentation<'a> {
    fn from(message: &'a Message) -> Self {
        Self {
            id: message.id.as_deref(),
            parent_id: message.parent_id.as_deref(),
            sender_id: message.person_id.as_deref(),
            sender_email: message.person_email.as_deref(),
            created_at: message.created.as_deref(),
            updated_at: message.updated.as_deref(),
            body: MessageBody {
                text: message.text.as_deref(),
                markdown: message.markdown.as_deref(),
                html: message.html.as_deref(),
            },
            attachment_count: message.files.as_ref().map(Vec::len),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_preserves_message_metadata_and_body_forms() {
        let message = Message {
            id: Some("message-a".into()),
            parent_id: Some("parent-a".into()),
            person_id: Some("sender-a".into()),
            person_email: Some("synthetic@example.invalid".into()),
            created: Some("2026-10-09T12:00:00Z".into()),
            updated: Some("2026-10-09T12:01:00Z".into()),
            text: Some("plain text".into()),
            markdown: Some("**markdown**".into()),
            html: Some("<p>html</p>".into()),
            files: Some(vec!["https://example.invalid/attachment".into()]),
            ..Default::default()
        };
        let view = MessagePresentation::from(&message);
        assert_eq!(view.id, message.id.as_deref());
        assert_eq!(view.parent_id, message.parent_id.as_deref());
        assert_eq!(view.sender_id, message.person_id.as_deref());
        assert_eq!(view.sender_email, message.person_email.as_deref());
        assert_eq!(view.created_at, message.created.as_deref());
        assert_eq!(view.updated_at, message.updated.as_deref());
        assert_eq!(view.body.text, message.text.as_deref());
        assert_eq!(view.body.markdown, message.markdown.as_deref());
        assert_eq!(view.body.html, message.html.as_deref());
        assert_eq!(view.attachment_count, Some(1));
    }

    #[test]
    fn projection_distinguishes_absent_and_empty_attachment_metadata() {
        let mut message = Message::default();
        assert_eq!(MessagePresentation::from(&message).attachment_count, None);
        message.files = Some(Vec::new());
        assert_eq!(
            MessagePresentation::from(&message).attachment_count,
            Some(0)
        );
    }
}
