// app/messages_list.rs

//! List of messages, keeping state of the UI scrolling offset and selected item.

use ratatui::widgets::{ScrollbarState, TableState};

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
    pub fn selected_index(&self) -> Option<usize> {
        if !self.has_selection {
            return None;
        }
        self.table_state.selected()
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
    /// PANICS if self.has_selection() is true while self.table_state.selected() is usize::MAX
    pub fn scroll_to_selection(&mut self) {
        if self.has_selection() {
            if let Some(selected) = self.table_state.selected() {
                assert_ne!(selected, usize::MAX); // because has_selection() is true
                if let Some(position) = position_for(selected, self.nb_messages, self.nb_lines) {
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
        *self.scroll_state_mut() = self.scroll_state.content_length(nb_lines);
        self.nb_lines = nb_lines;
    }

    pub fn scroll_state_mut(&mut self) -> &mut ScrollbarState {
        &mut self.scroll_state
    }
}

// generate tests for position_for only, using rstest
/// Calculates the scrollbar position for the given selection, number of messages and number of
/// lines.
/// Returns None if nb_messages is 0 or if the result is too big for usize.
/// PANICS:
///    - if selected >= nb_messages
///    - if nb_lines < nb_messages
fn position_for(selected: usize, nb_messages: usize, nb_lines: usize) -> Option<usize> {
    if nb_messages == 0 {
        return None;
    }
    if selected >= nb_messages {
        return Some(nb_messages);
    }
    assert!(nb_lines >= nb_messages);
    let pos_f = selected as f64 / nb_messages as f64 * nb_lines as f64;
    Some(pos_f as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

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

    #[cfg(test)]
    #[rstest(
        selected,
        nb_messages,
        nb_lines,
        expected,
        case(0, 0, 10, None),
        case(0, 1, 10, Some(0)),
        case(0, 2, 20, Some(0)),
        case(1, 2, 20, Some(10)),
        case(9, 10, 100, Some(90)),
        case(9, 10, 10, Some(9))
    )]
    fn test_position_for(
        selected: usize,
        nb_messages: usize,
        nb_lines: usize,
        expected: Option<usize>,
    ) {
        assert_eq!(position_for(selected, nb_messages, nb_lines), expected);
    }
}
