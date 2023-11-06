// app/messages_list.rs

//! List of messages, keeping state of the UI scrolling offset and selected item.

use ratatui::widgets::{ScrollbarState, TableState};
use webex::Message;

#[derive(Default)]
pub struct MessagesList {
    table_state: TableState,
    // workaround so we can set the Option in TableState to usize::MAX when there is no selection
    // to scroll the table to the last message
    has_selection: bool,
    scroll_state: ScrollbarState,
    nb_messages: usize,
    nb_lines: usize,
}

impl MessagesList {
    pub fn new() -> Self {
        let mut list = MessagesList::default();
        list.table_state.select(Some(usize::MAX));
        list
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
        if !self.has_selection {
            self.select_first_message();
            return;
        }
        match self.table_state.selected() {
            Some(_) if self.nb_messages == 0 => {
                // no items so deselect
                self.deselect();
            }
            Some(selected) if (selected >= self.nb_messages - 1) => {
                // last element or beyond is selected, do nothing
            }
            Some(selected) => {
                // select next element
                self.has_selection = true;
                self.table_state.select(Some(selected + 1));
            }
            None => {
                self.select_first_message();
            }
        }
    }

    /// Selects the previous message in the list and updates the table_state
    pub fn select_previous_message(&mut self) {
        if !self.has_selection {
            self.select_last_message();
            return;
        }
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
            None => self.select_last_message(),
        }
    }

    fn select_first_message(&mut self) {
        if self.nb_messages == 0 {
            self.deselect();
            return;
        }
        self.has_selection = true;
        self.table_state.select(Some(0));
    }

    fn select_last_message(&mut self) {
        if self.nb_messages == 0 {
            self.deselect();
            return;
        }
        self.has_selection = true;
        self.table_state.select(Some(self.nb_messages - 1));
    }

    pub fn deselect(&mut self) {
        self.has_selection = false;
        // Workaround to show the last message instead of the first one
        self.table_state.select(Some(usize::MAX));
    }

    /// Position the scrollbar according to the TableState selection.
    pub fn scroll_to_selection(&mut self) {
        if self.nb_messages == 0 {
            return;
        }
        if self.has_selection() {
            if let Some(selected) = self.table_state.selected() {
                assert_ne!(selected, usize::MAX); // because has_selection() is true
                if let Some(position) = (selected / self.nb_messages).checked_mul(self.nb_lines) {
                    self.scroll_state = self.scroll_state.position(position);
                }
            }
        } else {
            self.scroll_state.last();
        }
    }

    /// Sets the number of messages in the list.
    /// This needs to be kept up to date for other functions to work.
    /// A good place to call it is at UI render time.
    pub fn set_nb_messages(&mut self, nb_messages: usize) {
        self.nb_messages = nb_messages;
    }

    pub fn table_state_mut(&mut self) -> &mut TableState {
        &mut self.table_state
    }

    pub fn has_selection(&self) -> bool {
        self.has_selection
    }

    pub fn set_nb_lines(&mut self, nb_lines: usize) {
        self.nb_lines = nb_lines;
    }

    pub fn scroll_state_mut(&mut self) -> &mut ScrollbarState {
        &mut self.scroll_state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_next_message() {
        let mut list = MessagesList::new();
        list.set_nb_messages(3);
        assert_eq!(list.table_state.selected(), Some(usize::MAX));
        list.select_next_message();
        assert_eq!(list.table_state.selected(), Some(0));
        list.select_next_message();
        assert_eq!(list.table_state.selected(), Some(1));
        list.select_next_message();
        assert_eq!(list.table_state.selected(), Some(2));
        list.select_next_message();
        assert_eq!(list.table_state.selected(), Some(2));
    }

    #[test]
    fn test_select_previous_message() {
        let mut list = MessagesList::new();
        list.set_nb_messages(3);
        assert_eq!(list.table_state.selected(), Some(usize::MAX));
        list.select_previous_message();
        assert_eq!(list.table_state.selected(), Some(2));
        list.select_previous_message();
        assert_eq!(list.table_state.selected(), Some(1));
        list.select_previous_message();
        assert_eq!(list.table_state.selected(), Some(0));
        list.select_previous_message();
        assert_eq!(list.table_state.selected(), Some(0));
    }

    #[test]
    fn test_select_first_message() {
        let mut list = MessagesList::new();
        list.set_nb_messages(3);
        assert_eq!(list.table_state.selected(), Some(usize::MAX));
        list.select_first_message();
        assert!(list.has_selection());
        assert_eq!(list.table_state.selected(), Some(0));
        list.select_first_message();
        assert_eq!(list.table_state.selected(), Some(0));
    }

    #[test]
    fn test_select_last_message() {
        let mut list = MessagesList::new();
        list.set_nb_messages(3);
        assert!(!list.has_selection());
        assert_eq!(list.table_state.selected(), Some(usize::MAX));
        list.select_last_message();
        assert!(list.has_selection());
        assert_eq!(list.table_state.selected(), Some(2));
        list.select_last_message();
        assert_eq!(list.table_state.selected(), Some(2));
    }

    #[test]
    fn test_deselect() {
        let mut list = MessagesList::new();
        list.set_nb_messages(3);
        assert_eq!(list.table_state.selected(), Some(usize::MAX));
        assert!(!list.has_selection());
        list.select_first_message();
        assert_eq!(list.table_state.selected(), Some(0));
        assert!(list.has_selection());
        list.deselect();
        assert_eq!(list.table_state.selected(), Some(usize::MAX));
        assert!(!list.has_selection());
    }
}
