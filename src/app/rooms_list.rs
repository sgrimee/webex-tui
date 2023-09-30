// rooms_list.rs

use enum_iterator::{next_cycle, Sequence};
use log::*;
use ratatui::widgets::TableState;
use webex::Room;

use super::teams_store::{RoomId, TeamsStore};

#[derive(Clone, Debug, PartialEq, Sequence)]
pub enum RoomsListMode {
    All,
    // Direct,
    // Public,
    Recent,
    // Spaces,
    Unread,
}

pub struct RoomsList {
    mode: RoomsListMode,
    table_state: TableState,
}

impl RoomsList {
    pub fn new() -> Self {
        Self {
            mode: RoomsListMode::All,
            table_state: TableState::default(),
        }
    }

    /// Switch the rooms list table to the next filtering mode
    /// Does not update the active room
    pub fn next_mode(&mut self, store: &TeamsStore) {
        if let Some(new_mode) = next_cycle(&self.mode) {
            debug!("Rooms list mode set to {:?}", new_mode);
            self.mode = new_mode;
            // Reset selection when we change modes
            let num_rooms = store
                .rooms_filtered_by(self.mode())
                .collect::<Vec<_>>()
                .len();
            let selected = if num_rooms == 0 { None } else { Some(0) };
            self.table_state.select(selected);
        }
    }

    pub fn id_of_selected(&self, rooms: &[&Room]) -> Option<RoomId> {
        let id = match self.table_state.selected() {
            Some(selected) => rooms.get(selected).map(|room| room.id.to_owned()),
            None => None,
        };
        trace!("Id of selected room: {:?}", id);
        id
    }

    // pub fn selected_room<'a>(&'a self, rooms: &'a [&Room]) -> Option<&Room> {
    //     let room = self
    //         .table_state
    //         .selected()
    //         .and_then(|position| rooms.get(position))
    //         .copied();
    //     trace!("Selected room: {:?}", room);
    //     room
    // }

    /// Selects the next room in the list and updates the table_state
    /// Does not update the active room
    pub fn select_next_room(&mut self, num_rooms: usize) {
        match self.table_state.selected() {
            Some(_) if num_rooms == 0 => {
                // no items so deselect
                self.table_state.select(None)
            }
            Some(selected) if (selected >= num_rooms - 1) => {
                // last element selected, wrap around
                self.table_state.select(Some(0));
            }
            Some(selected) => {
                // select next element
                self.table_state.select(Some(selected + 1));
            }
            None => {
                if num_rooms > 0 {
                    // no selection but we have items, select first
                    self.table_state.select(Some(0));
                }
            }
        }
    }

    /// Selects the previous room in the list and updates the table_state
    /// Does not update the active room
    pub fn select_previous_room(&mut self, num_rooms: usize) {
        match self.table_state.selected() {
            Some(_) if num_rooms == 0 => {
                // no items so deselect
                self.table_state.select(None)
            }
            Some(selected) if (selected == 0) => {
                // first was selected, select last
                self.table_state.select(Some(num_rooms - 1));
            }
            Some(selected) => {
                // selected is not first, select previous
                self.table_state.select(Some(selected - 1));
            }

            None if num_rooms > 0 => {
                // no selection but we have items, select first
                self.table_state.select(Some(0));
            }
            None => {}
        }
    }

    pub fn table_state_mut(&mut self) -> &mut TableState {
        &mut self.table_state
    }

    pub fn mode(&self) -> RoomsListMode {
        self.mode.clone()
    }
}
