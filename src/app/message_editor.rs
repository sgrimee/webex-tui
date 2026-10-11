//! Neutral editor selection/display state.
use crate::frontend::data::MessageSnapshot;
use tui_textarea::{Input, TextArea};
#[derive(Default)]
pub(crate) struct MessageEditor<'a> {
    textarea: TextArea<'a>,
    is_composing: bool,
    response_to: Option<MessageSnapshot>,
    editing_of: Option<MessageSnapshot>,
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
        self.textarea.input(input)
    }
    pub(crate) fn insert_newline(&mut self) {
        self.textarea.insert_newline()
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
        self.editing_of = None
    }
    pub(crate) fn response_to(&self) -> Option<&MessageSnapshot> {
        self.response_to.as_ref()
    }
    pub(crate) fn set_response_to(&mut self, value: Option<MessageSnapshot>) {
        self.response_to = value
    }
    pub(crate) fn editing_of(&self) -> Option<&MessageSnapshot> {
        self.editing_of.as_ref()
    }
    pub(crate) fn set_editing_of(&mut self, value: Option<MessageSnapshot>) {
        self.editing_of = value
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::data::{MessageId, MessageStatus, RoomId};
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
    fn neutral_selection_cancels_and_restores() {
        let mut e = MessageEditor::default();
        e.reset_with_text("body".into());
        e.set_response_to(Some(message(MessageStatus::Active)));
        assert!(e.response_to().is_some());
        e.reset();
        assert!(e.response_to().is_none());
    }
}
