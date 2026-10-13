//! Neutral editor selection/display state.
//!
//! Every change to the text, reply target or edit target advances a draft
//! revision, so a write result can clear exactly the composition it was
//! submitted from and never newer text (see `frontend::editor_submission`).
use crate::frontend::{
    data::{ComposedBody, MessageSnapshot, OwnedMessageBody, RoomId},
    editor_submission::{DraftRevision, DraftSnapshot, DraftStamp, DraftTarget},
};
use tui_textarea::{Input, TextArea};
#[derive(Default)]
pub(crate) struct MessageEditor<'a> {
    textarea: TextArea<'a>,
    is_composing: bool,
    response_to: Option<MessageSnapshot>,
    editing_of: Option<MessageSnapshot>,
    revision: DraftRevision,
}
impl MessageEditor<'_> {
    pub(crate) fn lines(&self) -> &[String] {
        self.textarea.lines()
    }
    pub(crate) fn is_composing(&self) -> bool {
        self.is_composing
    }
    pub(crate) fn set_is_composing(&mut self, value: bool) {
        self.is_composing = value
    }
    pub(crate) fn input(&mut self, input: impl Into<Input>) -> bool {
        let modified = self.textarea.input(input);
        if modified {
            self.advance();
        }
        modified
    }
    pub(crate) fn insert_newline(&mut self) {
        self.textarea.insert_newline();
        self.advance();
    }
    pub(crate) fn textarea(&self) -> &TextArea<'_> {
        &self.textarea
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.textarea.is_empty()
    }
    pub(crate) fn reset(&mut self) {
        self.reset_with_text(String::new())
    }
    pub(crate) fn reset_with_text(&mut self, text: String) {
        self.textarea = TextArea::new(text.split('\n').map(str::to_owned).collect());
        self.response_to = None;
        self.editing_of = None;
        self.advance();
    }
    pub(crate) fn response_to(&self) -> Option<&MessageSnapshot> {
        self.response_to.as_ref()
    }
    pub(crate) fn set_response_to(&mut self, value: Option<MessageSnapshot>) {
        self.response_to = value;
        self.advance();
    }
    pub(crate) fn editing_of(&self) -> Option<&MessageSnapshot> {
        self.editing_of.as_ref()
    }
    pub(crate) fn set_editing_of(&mut self, value: Option<MessageSnapshot>) {
        self.editing_of = value;
        self.advance();
    }

    /// Start editing `message`: pre-fill the editor with the same source form
    /// that `draft()` will send. A message with a Markdown form is edited as
    /// its Markdown source and sent back as Markdown; otherwise its plain copy
    /// text is edited and sent as plain text. (Pre-filling the rendered text
    /// of a Markdown message and sending it as Markdown would reinterpret it,
    /// e.g. `snake_case_name` turning into italics.)
    pub(crate) fn start_editing(&mut self, message: MessageSnapshot) -> color_eyre::Result<()> {
        let text = match markdown_source(&message) {
            Some(markdown) => markdown.to_owned(),
            None => message.presentation().body.copy_text()?,
        };
        self.reset_with_text(text);
        self.set_editing_of(Some(message));
        Ok(())
    }

    /// Whether the current edit is sent as Markdown (see `start_editing`).
    pub(crate) fn edits_markdown(&self) -> bool {
        self.editing_of
            .as_ref()
            .is_some_and(|edited| markdown_source(edited).is_some())
    }

    #[cfg(test)]
    pub(crate) fn revision(&self) -> DraftRevision {
        self.revision
    }

    fn advance(&mut self) {
        // Saturating at the end of a 64-bit counter keeps the editor usable;
        // the submission ledger refuses to reuse an exhausted revision.
        if let Ok(next) = self.revision.next() {
            self.revision = next;
        }
    }

    /// The current composition as an explicit write draft. An edit is bound to
    /// the edited message's own room and keeps its known format (Markdown only
    /// if the original was Markdown; unknown legacy formats become explicit
    /// plain text). A reply is bound to the replied-to message's room and its
    /// thread root. Only a plain send uses `active_room`.
    pub(crate) fn draft(&self, active_room: Option<&RoomId>) -> Option<DraftSnapshot> {
        let text = self.lines().join("\n");
        let (target, markdown) = if let Some(edited) = &self.editing_of {
            let markdown = self.edits_markdown();
            (
                DraftTarget::Edit {
                    room_id: edited.room_id.clone(),
                    message_id: edited.id.clone(),
                },
                markdown,
            )
        } else if let Some(replied) = &self.response_to {
            (
                DraftTarget::Reply {
                    room_id: replied.room_id.clone(),
                    parent_id: replied
                        .parent_id
                        .clone()
                        .unwrap_or_else(|| replied.id.clone()),
                },
                false,
            )
        } else {
            (
                DraftTarget::Send {
                    room_id: active_room?.clone(),
                },
                false,
            )
        };
        Some(DraftSnapshot {
            stamp: DraftStamp {
                target,
                revision: self.revision,
            },
            body: if markdown {
                ComposedBody::Markdown(text)
            } else {
                ComposedBody::PlainText(text)
            },
        })
    }
}
/// Markdown source of an active message, if it has one. Legacy-unknown
/// bodies never qualify: their provenance cannot prove Markdown.
fn markdown_source(message: &MessageSnapshot) -> Option<&str> {
    match &message.body {
        Some(OwnedMessageBody::Forms {
            markdown: Some(markdown),
            ..
        }) if message.status == crate::frontend::data::MessageStatus::Active => Some(markdown),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::data::{MessageId, MessageStatus};
    fn message(status: MessageStatus) -> MessageSnapshot {
        MessageSnapshot {
            id: MessageId::new("m"),
            room_id: RoomId::new("r"),
            revision: None,
            parent_id: None,
            sender_id: None,
            sender_email: None,
            created_at: None,
            updated_at: None,
            status,
            body: None,
            attachment_count: None,
        }
    }
    #[test]
    fn every_change_advances_the_draft_revision() {
        let mut e = MessageEditor::default();
        let mut last = e.revision();
        let mut check = |e: &MessageEditor, what: &str| {
            assert_ne!(e.revision(), last, "{what}");
            last = e.revision();
        };
        e.input(Input {
            key: tui_textarea::Key::Char('x'),
            ..Default::default()
        });
        check(&e, "input");
        e.insert_newline();
        check(&e, "newline");
        e.set_response_to(Some(message(MessageStatus::Active)));
        check(&e, "reply target");
        e.set_editing_of(None);
        check(&e, "edit target");
        e.reset();
        check(&e, "reset");
        // A non-modifying input does not advance.
        let before = e.revision();
        assert!(!e.input(Input {
            key: tui_textarea::Key::Left,
            ..Default::default()
        }));
        assert_eq!(e.revision(), before);
    }

    #[test]
    fn drafts_bind_targets_to_their_own_rooms_and_formats() {
        let active = RoomId::new("active");
        let mut e = MessageEditor::default();
        e.reset_with_text("hello\nworld".into());
        assert!(e.draft(None).is_none(), "a send needs a room");
        let send = e.draft(Some(&active)).unwrap();
        assert!(
            send.stamp.target
                == DraftTarget::Send {
                    room_id: active.clone()
                }
        );
        assert!(send.body == ComposedBody::PlainText("hello\nworld".into()));
        assert_eq!(send.stamp.revision, e.revision());

        // A reply to a reply targets the thread root in the message's room.
        let mut reply = message(MessageStatus::Active);
        reply.room_id = RoomId::new("other");
        reply.parent_id = Some(MessageId::new("root"));
        e.set_response_to(Some(reply));
        let draft = e.draft(Some(&active)).unwrap();
        assert!(
            draft.stamp.target
                == DraftTarget::Reply {
                    room_id: RoomId::new("other"),
                    parent_id: MessageId::new("root"),
                }
        );

        // Edits keep a known Markdown format and pre-fill its Markdown source,
        // never the rendered text (REVIEW-13 LOW-2: `snake_case_name` must
        // not be edited as text and re-sent as Markdown italics).
        let mut edited = message(MessageStatus::Active);
        edited.room_id = RoomId::new("edited-room");
        edited.body = Some(OwnedMessageBody::Forms {
            text: Some("old snake case name".into()),
            markdown: Some("**old** `snake_case_name`".into()),
            html: None,
        });
        e.set_response_to(None);
        e.start_editing(edited.clone()).unwrap();
        assert_eq!(e.lines(), ["**old** `snake_case_name`"]);
        assert!(e.edits_markdown());
        let draft = e.draft(Some(&active)).unwrap();
        assert!(
            draft.stamp.target
                == DraftTarget::Edit {
                    room_id: RoomId::new("edited-room"),
                    message_id: MessageId::new("m"),
                }
        );
        assert!(draft.body == ComposedBody::Markdown("**old** `snake_case_name`".into()));

        // A plain-text message is edited and sent as its plain text.
        edited.body = Some(OwnedMessageBody::Forms {
            text: Some("snake_case_name".into()),
            markdown: None,
            html: None,
        });
        e.start_editing(edited.clone()).unwrap();
        assert_eq!(e.lines(), ["snake_case_name"]);
        assert!(!e.edits_markdown());
        assert!(
            e.draft(Some(&active)).unwrap().body
                == ComposedBody::PlainText("snake_case_name".into())
        );

        // Unknown legacy provenance is edited and sent as plain text.
        edited.body = Some(OwnedMessageBody::LegacyUnknown("old".into()));
        e.start_editing(edited).unwrap();
        assert_eq!(e.lines(), ["old"]);
        assert!(matches!(
            e.draft(Some(&active)).unwrap().body,
            ComposedBody::PlainText(_)
        ));
    }

    #[test]
    fn neutral_selection_cancels_and_restores() {
        let mut e = MessageEditor::default();
        e.reset_with_text("body".into());
        e.set_response_to(Some(message(MessageStatus::Active)));
        assert!(e.response_to().is_some());
        e.reset();
        assert!(e.response_to().is_none());
    }
}
