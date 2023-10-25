// app/messages_list.rs

//! List of messages, keeping state of the UI scrolling offset and selected item.

// use log::*;
use ratatui::widgets::TableState;

// TODO: implement this module

#[derive(Default)]
pub struct MessagesList {
    table_state: TableState,
    // scroll_state: ScrollbarState,
    // scroll: usize,
}

impl MessagesList {
    pub fn new() -> Self {
        MessagesList::default()
    }

    // Return the id of the selected message if there is one
    // pub fn id_of_selected(&self, messages: &[&message]) -> Option<messageId> {
    //     let id = match self.table_state.selected() {
    //         Some(selected) => messages.get(selected).map(|message| message.id.to_owned()),
    //         None => None,
    //     };
    //     trace!("Id of selected message: {:?}", id);
    //     id
    // }

    /// Selects the next message in the list and updates the table_state
    /// Does not update the active message
    // pub fn select_next_message(&mut self, num_messages: usize) {
    //     match self.table_state.selected() {
    //         Some(_) if num_messages == 0 => {
    //             // no items so deselect
    //             self.table_state.select(None)
    //         }
    //         Some(selected) if (selected >= num_messages - 1) => {
    //             // last element selected, wrap around
    //             self.table_state.select(Some(0));
    //         }
    //         Some(selected) => {
    //             // select next element
    //             self.table_state.select(Some(selected + 1));
    //         }
    //         None => {
    //             if num_messages > 0 {
    //                 // no selection but we have items, select first
    //                 self.table_state.select(Some(0));
    //             }
    //         }
    //     }
    // }

    /// Selects the previous message in the list and updates the table_state
    /// Does not update the active message
    // pub fn select_previous_message(&mut self, num_messages: usize) {
    //     match self.table_state.selected() {
    //         Some(_) if num_messages == 0 => {
    //             // no items so deselect
    //             self.table_state.select(None)
    //         }
    //         Some(0) => {
    //             // first was selected, select last
    //             self.table_state.select(Some(num_messages - 1));
    //         }
    //         Some(selected) => {
    //             // selected is not first, select previous
    //             self.table_state.select(Some(selected - 1));
    //         }

    //         None if num_messages > 0 => {
    //             // no selection but we have items, select first
    //             self.table_state.select(Some(0));
    //         }
    //         None => {}
    //     }
    // }

    pub fn table_state_mut(&mut self) -> &mut TableState {
        &mut self.table_state
    }
}
