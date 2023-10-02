// app/state.rs

use ratatui_textarea::TextArea;
use webex::Room;

use super::{
    actions::{Action, Actions},
    rooms_list::RoomsList,
    teams_store::{RoomId, TeamsStore},
};

pub struct AppState<'a> {
    // App
    pub actions: Actions,
    pub editing_mode: bool,
    pub is_loading: bool,

    // Webex
    pub teams_store: TeamsStore,

    // UI
    pub show_logs: bool,
    pub show_help: bool,
    pub msg_input_textarea: TextArea<'a>,
    pub rooms_list: RoomsList,
    active_room_id: Option<RoomId>,
}

impl AppState<'_> {
    pub fn active_room_id(&self) -> &Option<RoomId> {
        &self.active_room_id
    }

    pub fn set_active_room_id(&mut self, active_room_id: Option<RoomId>) {
        self.active_room_id = active_room_id;
    }

    pub fn active_room(&self) -> Option<&Room> {
        self.active_room_id()
            .clone()
            .and_then(|id| self.teams_store.room_with_id(&id))
    }

    pub fn set_active_room_to_selection(&mut self) {
        self.set_active_room_id(self.id_of_selected_room());
    }

    pub fn visible_rooms(&self) -> impl Iterator<Item = &Room> {
        self.teams_store.rooms_filtered_by(self.rooms_list.mode())
    }

    pub fn num_of_visible_rooms(&self) -> usize {
        self.visible_rooms().collect::<Vec<_>>().len()
    }

    pub fn id_of_selected_room(&self) -> Option<RoomId> {
        self.rooms_list
            .id_of_selected(self.visible_rooms().collect::<Vec<_>>().as_slice())
    }

    pub fn update_selection_with_active_room(&mut self) {
        if let Some(id) = self.active_room_id() {
            let pos_option = self.visible_rooms().position(|room| &room.id == id);
            if let Some(position) = pos_option {
                self.rooms_list.table_state_mut().select(Some(position))
            }
        }
    }

    pub fn next_filtering_mode(&mut self) {
        self.rooms_list.next_mode(&self.teams_store);
        self.set_active_room_to_selection();
    }

    pub fn next_room(&mut self) {
        let num_rooms = self.num_of_visible_rooms();
        self.rooms_list.select_next_room(num_rooms);
        self.set_active_room_to_selection();
    }

    pub fn previous_room(&mut self) {
        let num_rooms = self.num_of_visible_rooms();
        self.rooms_list.select_previous_room(num_rooms);
        self.set_active_room_to_selection();
    }

    pub fn mark_active_read(&mut self) {
        if let Some(id) = self.id_of_selected_room() {
            self.teams_store.mark_read(&id);
        }
    }
}

impl Default for AppState<'_> {
    fn default() -> Self {
        AppState {
            actions: vec![Action::Quit, Action::ToggleHelp, Action::ToggleLogs].into(),
            editing_mode: false,
            is_loading: false,
            msg_input_textarea: TextArea::default(),
            show_logs: false,
            show_help: true,
            teams_store: TeamsStore::default(),
            rooms_list: RoomsList::new(),
            active_room_id: None,
        }
    }
}
