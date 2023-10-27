// app/messages_list.rs

//! List of messages, keeping state of the UI scrolling offset and selected item.

#![allow(unused_imports)]
#![allow(dead_code)]

use core::num;

use log::*;
use ratatui::widgets::TableState;
use webex::Message;

use super::teams_store::{MessageId, TeamsStore};

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

    /// Returns the id of the selected message if there is one.
    pub fn id_of_selected(&self, messages: &[&Message]) -> Option<MessageId> {
        let id = self
            .table_state
            .selected()
            .and_then(|selected| messages.get(selected))
            .and_then(|message| message.id.clone());
        trace!("Id of selected message: {:?}", id);
        id
    }

    /// Selects the next message in the list and updates the table_state.
    pub fn select_next_message(&mut self, num_messages: usize) {
        match self.table_state.selected() {
            Some(_) if num_messages == 0 => {
                // no items so deselect
                self.table_state.select(None)
            }
            Some(selected) if (selected >= num_messages - 1) => {
                // last element selected, do nothing
            }
            Some(selected) => {
                // select next element
                self.table_state.select(Some(selected + 1));
            }
            None => {
                if num_messages > 0 {
                    // no selection but we have items, select first
                    self.table_state.select(Some(0));
                }
            }
        }
    }

    /// Selects the previous message in the list and updates the table_state
    /// Does not update the active message
    pub fn select_previous_message(&mut self, num_messages: usize) {
        match self.table_state.selected() {
            Some(_) if num_messages == 0 => {
                // no items so deselect
                self.table_state.select(None)
            }
            Some(0) => {
                // first was selected, do nothing
            }
            Some(selected) => {
                // selected is not first, select previous
                self.table_state.select(Some(selected - 1));
            }
            None if num_messages > 0 => {
                // no selection but we have items, select first
                self.table_state.select(Some(0));
            }
            None => {}
        }
    }

    /// Selects the last message in the list and updates the table_state.
    /// Unselect if there are no messages.
    pub fn select_last_message(&mut self, num_messages: usize) {
        if num_messages > 0 {
            self.table_state.select(Some(num_messages - 1));
        } else {
            self.table_state.select(None);
        }
    }

    pub fn table_state_mut(&mut self) -> &mut TableState {
        &mut self.table_state
    }

    /// Scrolls the view to the last message without affecting the selection
    pub(crate) fn scroll_to_last(&mut self, num_messages: usize) {
        // TODO: fix this, use a proper scroll bar
        if num_messages > 0 {
            *self.table_state.offset_mut() = num_messages - 1;
        }
    }
}
