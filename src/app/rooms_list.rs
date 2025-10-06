// app/rooms_list.rs

//! List of rooms, with UI scrolling and selection state and display filters.

use super::cache::room::{Room, RoomId};
use super::cache::room_list_filter::RoomsListFilter;
use super::cache::Cache;
use enum_iterator::{next_cycle, previous_cycle};
use log::*;
use ratatui::widgets::TableState;
use std::collections::HashSet;

#[derive(Default)]
pub(crate) struct RoomsList {
    filter: RoomsListFilter,
    table_state: TableState,
    active_room_id: Option<RoomId>,
    search_query: Option<String>,
    selected_rooms: HashSet<RoomId>,
}

impl RoomsList {
    /// Switches the rooms list table to the next filtering mode.
    /// Does not update the active room.
    pub(crate) fn next_filter(&mut self, store: &Cache) {
        let new_mode = next_cycle(&self.filter);
        debug!("Rooms list filter set to {:?}", new_mode);
        self.filter = new_mode;
        // Reset selection when we change filter
        let num_rooms = store
            .rooms
            .rooms_filtered_by(self.filter())
            .collect::<Vec<_>>()
            .len();
        let selected = if num_rooms == 0 { None } else { Some(0) };
        self.table_state.select(selected);
    }

    /// Switches the rooms list table to the previous filtering mode.
    /// Does not update the active room.
    pub(crate) fn previous_filter(&mut self, store: &Cache) {
        let new_mode = previous_cycle(&self.filter);
        debug!("Rooms list mode set to {:?}", new_mode);
        self.filter = new_mode;
        // Reset selection when we change filter
        let num_rooms = store
            .rooms
            .rooms_filtered_by(self.filter())
            .collect::<Vec<_>>()
            .len();
        let selected = if num_rooms == 0 { None } else { Some(0) };
        self.table_state.select(selected);
    }

    /// Returns the id of the selected room if there is one.
    pub(crate) fn id_of_selected(&self, rooms: &[&Room]) -> Option<RoomId> {
        let id = match self.table_state.selected() {
            Some(selected) => rooms.get(selected).map(|room| room.id.clone()),
            None => None,
        };
        id
    }

    pub(crate) fn has_selection(&self) -> bool {
        self.table_state.selected().is_some()
    }

    /// Selects the next room in the list and updates the table_state.
    /// Does not update the active room.
    pub(crate) fn select_next_room(&mut self, num_rooms: usize) {
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
    pub(crate) fn select_previous_room(&mut self, num_rooms: usize) {
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

    pub(crate) fn table_state_mut(&mut self) -> &mut TableState {
        &mut self.table_state
    }

    pub(crate) fn filter(&self) -> &RoomsListFilter {
        &self.filter
    }

    pub(crate) fn active_room_id(&self) -> Option<&String> {
        self.active_room_id.as_ref()
    }

    pub(crate) fn set_active_room_id(&mut self, active_room_id: Option<RoomId>) {
        self.active_room_id = active_room_id;
    }

    pub(crate) fn search_query(&self) -> Option<&String> {
        self.search_query.as_ref()
    }

    pub(crate) fn set_search_query(&mut self, query: Option<String>) {
        self.search_query = query;
    }

    /// Toggle selection of a room
    pub(crate) fn toggle_room_selection(&mut self, room_id: &RoomId) {
        if self.selected_rooms.contains(room_id) {
            self.selected_rooms.remove(room_id);
        } else {
            self.selected_rooms.insert(room_id.clone());
        }
    }

    /// Check if a room is selected
    pub(crate) fn is_room_selected(&self, room_id: &RoomId) -> bool {
        self.selected_rooms.contains(room_id)
    }

    /// Get all selected room IDs
    pub(crate) fn selected_room_ids(&self) -> Vec<RoomId> {
        self.selected_rooms.iter().cloned().collect()
    }

    /// Clear all room selections
    pub(crate) fn clear_room_selections(&mut self) {
        self.selected_rooms.clear();
    }

    /// Select all visible rooms
    pub(crate) fn select_all_visible_rooms(&mut self, rooms: &[&Room]) {
        for room in rooms {
            self.selected_rooms.insert(room.id.clone());
        }
    }

    /// Get the number of selected rooms
    pub(crate) fn selected_room_count(&self) -> usize {
        self.selected_rooms.len()
    }

    /// Toggle selection of the currently highlighted room
    pub(crate) fn toggle_current_room_selection(&mut self, rooms: &[&Room]) {
        if let Some(room_id) = self.id_of_selected(rooms) {
            self.toggle_room_selection(&room_id);
        }
    }
}
