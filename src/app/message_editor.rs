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
    // whether the editor is in text editing mode
    is_editing: bool,
    respondee: Option<Respondee>,
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

    pub(crate) fn clear(&mut self) {
        self.textarea = TextArea::default();
    }

    pub fn set_respondee(&mut self, respondee: Option<Respondee>) {
        self.respondee = respondee;
    }

    pub fn respondee(&self) -> Option<&Respondee> {
        self.respondee.as_ref()
    }
}
