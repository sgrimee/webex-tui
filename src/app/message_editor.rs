// app/message_editor.rs

//! Editor for typing messages.

use tui_textarea::{Input, TextArea};

use super::teams_store::MessageId;

#[derive(Clone, Debug)]
pub struct Respondee {
    pub parent_msg_id: MessageId,
    pub author: String,
}

#[derive(Default)]
pub struct MessageEditor<'a> {
    textarea: TextArea<'a>,
    /// Whether the editor captures key events.
    is_editing: bool,
    /// Thread to which the message is replying.
    respondee: Option<Respondee>,
    /// Existing message being corrected.
    message_id: Option<MessageId>,
}

impl<'a> MessageEditor<'a> {
    pub fn lines(&self) -> &[String] {
        self.textarea.lines()
    }

    pub fn is_editing(&self) -> bool {
        self.is_editing
    }

    pub fn set_is_editing(&mut self, is_editing: bool) {
        self.is_editing = is_editing;
    }

    pub fn input(&mut self, input: impl Into<Input>) -> bool {
        self.textarea.input(input)
    }

    pub fn insert_newline(&mut self) {
        self.textarea.insert_newline();
    }

    pub fn textarea(&self) -> &TextArea {
        &self.textarea
    }

    pub fn is_empty(&self) -> bool {
        self.textarea.is_empty()
    }

    pub(crate) fn reset(&mut self) {
        self.textarea = TextArea::default();
        self.respondee = None;
        self.message_id = None;
    }

    /// Sets the message editor content to the given text.
    pub(crate) fn reset_with_text(&mut self, text: String) {
        // Textarea does not support newlines in the text.
        let lines = text.split('\n').map(|s| s.to_string()).collect::<Vec<_>>();
        self.textarea = TextArea::new(lines);
        self.respondee = None;
        self.message_id = None;
    }

    pub fn set_respondee(&mut self, respondee: Option<Respondee>) {
        self.respondee = respondee;
    }

    pub fn respondee(&self) -> Option<&Respondee> {
        self.respondee.as_ref()
    }
}
