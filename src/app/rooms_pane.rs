// rooms_list.rs

use enum_iterator::{next_cycle, previous_cycle, Sequence};
use log::*;
use ratatui::widgets::TableState;
use webex::Room;

use super::teams_store::{RoomId, TeamsStore};

#[derive(Clone, Debug, PartialEq, Default, Sequence)]
pub enum RoomsListFilter {
    All,
    Direct,
    #[default]
    Recent,
    Spaces,
    Unread,
}

pub struct RoomsList {
    mode: RoomsListFilter,
    table_state: TableState,
    pub active_room_id: Option<RoomId>,
}

impl RoomsList {
    pub fn new() -> Self {
        Self {
            mode: RoomsListFilter::default(),
            table_state: TableState::default(),
            active_room_id: None,
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
                .rooms_filtered_by(self.mode(), self.active_room_id.clone())
                .collect::<Vec<_>>()
                .len();
            let selected = if num_rooms == 0 { None } else { Some(0) };
            self.table_state.select(selected);
        }
    }

    /// Switch the rooms list table to the previous filtering mode
    /// Does not update the active room
    pub fn previous_mode(&mut self, store: &TeamsStore) {
        if let Some(new_mode) = previous_cycle(&self.mode) {
            debug!("Rooms list mode set to {:?}", new_mode);
            self.mode = new_mode;
            // Reset selection when we change modes
            let num_rooms = store
                .rooms_filtered_by(self.mode(), self.active_room_id.clone())
                .collect::<Vec<_>>()
                .len();
            let selected = if num_rooms == 0 { None } else { Some(0) };
            self.table_state.select(selected);
        }
    }

    // Return the id of the selected room if there is one
    pub fn id_of_selected(&self, rooms: &[&Room]) -> Option<RoomId> {
        let id = match self.table_state.selected() {
            Some(selected) => rooms.get(selected).map(|room| room.id.to_owned()),
            None => None,
        };
        trace!("Id of selected room: {:?}", id);
        id
    }

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
            Some(0) => {
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

    pub fn mode(&self) -> RoomsListFilter {
        self.mode.clone()
    }
}
