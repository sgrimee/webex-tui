//! Owned, backend-neutral data for frontend intent/result envelopes.
//!
//! These are disposable presentation snapshots, not a canonical mirror/schema.
//! IDs are opaque; this module never parses, decodes or rewrites them.

use super::models::{MessageBody, MessagePresentation};

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub(crate) struct $name(String);

        impl $name {
            pub(crate) fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub(crate) fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

opaque_id!(RoomId);
opaque_id!(MessageId);
opaque_id!(PersonId);
opaque_id!(TeamId);

/// Outbound composition has an explicit format; HTML is read-only presentation.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ComposedBody {
    PlainText(String),
    Markdown(String),
}

/// Retain source body forms without fabricating Markdown/plain-text provenance
/// for already-normalized legacy data. Neither variant fetches resources.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum OwnedMessageBody {
    Forms {
        text: Option<String>,
        markdown: Option<String>,
        html: Option<String>,
    },
    LegacyUnknown(String),
}

impl OwnedMessageBody {
    pub(crate) fn presentation(&self) -> MessageBody<'_> {
        match self {
            Self::Forms {
                text,
                markdown,
                html,
            } => MessageBody {
                text: text.as_deref(),
                markdown: markdown.as_deref(),
                html: html.as_deref(),
            },
            // Raw text is a presentation fallback only. The owned variant keeps
            // its unknown provenance; it must not be converted into a send body.
            Self::LegacyUnknown(content) => MessageBody {
                text: Some(content),
                ..Default::default()
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MessageStatus {
    Active,
    Deleted,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct MessageSnapshot {
    pub(crate) id: MessageId,
    pub(crate) room_id: RoomId,
    /// Unknown for the legacy backend; otherwise the daemon's canonical revision.
    pub(crate) revision: Option<u64>,
    pub(crate) parent_id: Option<MessageId>,
    pub(crate) sender_id: Option<PersonId>,
    pub(crate) sender_email: Option<String>,
    pub(crate) created_at: Option<String>,
    pub(crate) updated_at: Option<String>,
    pub(crate) status: MessageStatus,
    pub(crate) body: Option<OwnedMessageBody>,
    /// Unknown/absent metadata is not a known zero. No URLs or bytes are exposed.
    pub(crate) attachment_count: Option<usize>,
}

impl MessageSnapshot {
    /// Read-only projection. A tombstone never presents an old body/attachment
    /// count, even if a malformed input snapshot retained them accidentally.
    pub(crate) fn presentation(&self) -> MessagePresentation<'_> {
        let active = self.status == MessageStatus::Active;
        MessagePresentation {
            id: Some(self.id.as_str()),
            parent_id: self.parent_id.as_ref().map(MessageId::as_str),
            sender_id: self.sender_id.as_ref().map(PersonId::as_str),
            sender_email: self.sender_email.as_deref(),
            created_at: self.created_at.as_deref(),
            updated_at: self.updated_at.as_deref(),
            body: if active {
                self.body
                    .as_ref()
                    .map(OwnedMessageBody::presentation)
                    .unwrap_or_default()
            } else {
                MessageBody::default()
            },
            attachment_count: if active { self.attachment_count } else { None },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RoomKind {
    Direct,
    Group,
    Unknown,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct RoomSnapshot {
    pub(crate) id: RoomId,
    pub(crate) title: Option<String>,
    pub(crate) kind: RoomKind,
    pub(crate) team_id: Option<TeamId>,
    pub(crate) last_activity: Option<String>,
    pub(crate) is_member: bool,
    /// Missing moderation metadata must not be interpreted as unlocked.
    pub(crate) is_locked: Option<bool>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PersonSnapshot {
    pub(crate) id: PersonId,
    pub(crate) display_name: Option<String>,
    pub(crate) emails: Option<Vec<String>>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct TeamSnapshot {
    pub(crate) id: TeamId,
    pub(crate) name: Option<String>,
}

/// Cursor is opaque backend pagination state, not a path or message display index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PageCursor(String);

impl PageCursor {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct MessagePage {
    pub(crate) room_id: RoomId,
    pub(crate) messages: Vec<MessageSnapshot>,
    pub(crate) next_cursor: Option<PageCursor>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Initialization {
    /// First-run/offline identity may be unavailable while rooms remain readable.
    pub(crate) account: Option<PersonSnapshot>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::models::DisplayBody;

    #[test]
    fn ids_and_cursors_round_trip_without_interpretation() {
        let opaque = "room/id?x=1%2F../not-base64";
        assert_eq!(RoomId::new(opaque).as_str(), opaque);
        assert_eq!(MessageId::new(opaque).as_str(), opaque);
        assert_eq!(PersonId::new(opaque).as_str(), opaque);
        assert_eq!(TeamId::new(opaque).as_str(), opaque);
        assert_eq!(PageCursor::new(opaque).as_str(), opaque);
    }

    #[test]
    fn owned_forms_reuse_existing_display_and_copy_precedence() {
        let body = OwnedMessageBody::Forms {
            text: Some("plain".into()),
            markdown: Some("**markdown**".into()),
            html: Some("<p>html</p>".into()),
        };
        assert_eq!(
            body.presentation().display(),
            DisplayBody::Html("<p>html</p>")
        );
        assert_eq!(body.presentation().copy_text().unwrap(), "plain");
    }

    #[test]
    fn unknown_legacy_format_is_retained_while_presented_as_raw_text() {
        let body = OwnedMessageBody::LegacyUnknown("**unclassified**".into());
        assert!(matches!(body, OwnedMessageBody::LegacyUnknown(_)));
        assert_eq!(
            body.presentation().display(),
            DisplayBody::Text("**unclassified**")
        );
        assert_eq!(body.presentation().copy_text().unwrap(), "**unclassified**");
    }

    #[test]
    fn tombstone_projection_never_presents_retained_body_or_attachments() {
        let message = MessageSnapshot {
            id: MessageId::new("message-a"),
            room_id: RoomId::new("room-a"),
            revision: Some(2),
            parent_id: Some(MessageId::new("parent-a")),
            sender_id: Some(PersonId::new("person-a")),
            sender_email: None,
            created_at: None,
            updated_at: None,
            status: MessageStatus::Deleted,
            body: Some(OwnedMessageBody::LegacyUnknown("old body".into())),
            attachment_count: Some(1),
        };
        let view = message.presentation();
        assert_eq!(view.id, Some("message-a"));
        assert_eq!(view.parent_id, Some("parent-a"));
        assert_eq!(view.body.display(), DisplayBody::Missing);
        assert_eq!(view.attachment_count, None);
        // Presentation does not repair/mutate the input or claim a body purge.
        assert!(message.body.is_some());
    }

    #[test]
    fn composition_formats_are_distinct_and_never_include_html() {
        let text = ComposedBody::PlainText("**literal**".into());
        let markdown = ComposedBody::Markdown("**literal**".into());
        assert!(text != markdown);
    }
}
