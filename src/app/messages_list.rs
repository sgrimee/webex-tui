// app/messages_list.rs

//! List of messages, keeping state of the UI scrolling offset and selected item.

use ratatui::widgets::{ScrollbarState, TableState};
use webex::Message;

#[derive(Default)]
pub struct MessagesList {
    table_state: TableState,
    // workaround as the Option in TableState does not reflect the actual state
    has_selection: bool,
    // scroll_state: ScrollbarState,
    nb_messages: usize,
    // nb_lines: usize,
}

impl MessagesList {
    pub fn new() -> Self {
        MessagesList::default()
    }

    /// Returns the message corresponding to the selection, if there is one.
    /// If the selection is out of bounds, returns None.
    pub fn selected_message<'a>(&'a self, messages: &'a [Message]) -> Option<&Message> {
        if !self.has_selection {
            return None;
        }
        let msg = self
            .table_state
            .selected()
            .and_then(|selected| messages.get(selected));
        msg
    }

    /// Selects the next message in the list and updates the table_state.
    pub fn select_next_message(&mut self) {
        match self.table_state.selected() {
            Some(_) if self.nb_messages == 0 => {
                // no items so deselect
                self.deselect();
            }
            Some(selected) if (selected >= self.nb_messages - 1) => {
                // last element selected, do nothing
            }
            Some(selected) => {
                // select next element
                self.has_selection = true;
                self.table_state.select(Some(selected + 1));
            }
            None => {
                if self.nb_messages > 0 {
                    // no selection but we have items, select first
                    self.select_first_message();
                }
            }
        }
    }

    /// Selects the previous message in the list and updates the table_state
    /// Does not update the active message
    pub fn select_previous_message(&mut self) {
        match self.table_state.selected() {
            Some(_) if self.nb_messages == 0 => {
                // no items so deselect
                self.deselect();
            }
            Some(0) => {
                // first was selected, do nothing
            }
            Some(selected) if selected > self.nb_messages => {
                // selection is out of bounds, select last
                self.select_last_message();
            }
            Some(selected) => {
                // selected is not first, select previous
                self.has_selection = true;
                self.table_state.select(Some(selected - 1));
            }
            None if self.nb_messages > 0 => {
                // no selection but we have items, select last
                self.has_selection = true;
                self.table_state.select(Some(self.nb_messages - 1));
            }
            None => {}
        }
    }

    fn select_first_message(&mut self) {
        self.table_state.select(Some(0));
        self.has_selection = true;
    }

    fn select_last_message(&mut self) {
        self.table_state.select(Some(self.nb_messages - 1));
        self.has_selection = true;
    }

    pub(crate) fn deselect(&mut self) {
        self.has_selection = false;
        // Workaround to show the last message instead of the first one
        self.table_state.select(Some(usize::MAX));
    }

    // Scrolls to the last message if there is no selection
    // pub fn scroll_to_last_if_no_selection(&mut self, self.nb_messages: usize) {
    //     if self.table_state.selected().is_none() {
    //         // When selection is None, offset if set to 0 by the library,
    //         // but we want to see the last message, so set a selection beyond
    //         // the last message.
    //         self.table_state.select(Some(self.nb_messages));
    //     }
    // }

    // /// Scrolls the view to make the selection visible if there is one,
    // /// or to the last message otherwise.
    // pub fn scroll_to_selection_or_last(&mut self) {
    //     if self.nb_messages == 0 {
    //         return;
    //     }
    //     if let Some(selected) = self.table_state.selected() {
    //         let position = selected / self.nb_messages * self.nb_lines;
    //         trace!("Scrolling to {}", position);
    //         self.scroll_state = self.scroll_state.position(position);
    //     } else {
    //         trace!("Scrolling to last");
    //         self.scroll_state.last();
    //         *self.table_state.offset_mut() = self.nb_messages - 1;
    //     }
    // }

    // pub fn scroll_state_mut(&mut self) -> &mut ScrollbarState {
    //     &mut self.scroll_state
    // }

    // pub fn set_nb_lines(&mut self, nb_lines: usize) {
    //     self.nb_lines = nb_lines;
    //     self.scroll_state.content_length(nb_lines);
    // }

    /// Sets the number of messages in the list.
    /// This needs to be kept up to date for other functions to work.
    /// A good place to call it is at UI render time.
    pub fn set_nb_messages(&mut self, nb_messages: usize) {
        self.nb_messages = nb_messages;
    }

    pub fn table_state_mut(&mut self) -> &mut TableState {
        &mut self.table_state
    }
}
