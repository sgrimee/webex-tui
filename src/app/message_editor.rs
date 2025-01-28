// app/message_editor.rs

//! Editor for typing messages.

use tui_textarea::{Input, TextArea};
use webex::Message;

#[derive(Default)]
pub(crate) struct MessageEditor<'a> {
    textarea: TextArea<'a>,
    is_composing: bool,
    response_to: Option<Message>,
    editing_of: Option<Message>,
}

impl MessageEditor<'_> {
    /// Returns the text in the editor.
    pub(crate) fn lines(&self) -> &[String] {
        self.textarea.lines()
    }

    /// Whether the editor captures key events.
    pub(crate) fn is_composing(&self) -> bool {
        self.is_composing
    }

    /// Sets whether the editor should capture key events.
    pub(crate) fn set_is_composing(&mut self, is_editing: bool) {
        self.is_composing = is_editing;
    }

    /// Sends a character to the editor.
    pub(crate) fn input(&mut self, input: impl Into<Input>) -> bool {
        self.textarea.input(input)
    }

    /// Inserts a newline in the editor.
    pub(crate) fn insert_newline(&mut self) {
        self.textarea.insert_newline();
    }

    /// Returns the textarea.
    pub(crate) fn textarea(&self) -> &TextArea {
        &self.textarea
    }

    /// Returns whether the message editor is empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.textarea.is_empty()
    }

    /// Resets the message editor content.
    pub(crate) fn reset(&mut self) {
        self.reset_with_text("".to_string());
    }

    /// Sets the message editor content to the given text.
    pub(crate) fn reset_with_text(&mut self, text: String) {
        // Textarea does not support newlines in the text.
        let lines = text.split('\n').map(|s| s.to_string()).collect::<Vec<_>>();
        self.textarea = TextArea::new(lines);
        self.response_to = None;
        self.editing_of = None;
    }

    /// Returns the message to which the message is replying.
    pub(crate) fn response_to(&self) -> Option<&Message> {
        self.response_to.as_ref()
    }

    /// Sets the message to which the message is replying.
    pub(crate) fn set_response_to(&mut self, message: Option<Message>) {
        self.response_to = message;
    }

    /// Returns the message being edited.
    pub(crate) fn editing_of(&self) -> Option<&Message> {
        self.editing_of.as_ref()
    }

    /// Sets the message being edited.
    pub(crate) fn set_editing_of(&mut self, editing_of: Option<Message>) {
        self.editing_of = editing_of;
    }
}
