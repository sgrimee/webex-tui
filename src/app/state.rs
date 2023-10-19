// app/state.rs

use ratatui_textarea::TextArea;
use webex::Room;

use super::actions::{Action, Actions};
use super::messages_list::MessagesList;
use super::rooms_list::RoomsList;
use super::teams_store::{RoomId, TeamsStore};

pub struct AppState<'a> {
    // App
    pub actions: Actions,
    pub editing_mode: bool,
    is_loading: bool,

    // Webex
    pub teams_store: TeamsStore,

    // UI
    pub show_logs: bool,
    pub show_help: bool,
    pub msg_input_textarea: TextArea<'a>,
    pub rooms_list: RoomsList,
    pub messages_list: MessagesList,
}

impl AppState<'_> {
    pub fn active_room_id(&self) -> Option<RoomId> {
        self.rooms_list.active_room_id.clone()
    }

    pub fn set_active_room_id(&mut self, active_room_id: &Option<RoomId>) {
        self.rooms_list.active_room_id = active_room_id.clone();
    }

    pub fn active_room(&self) -> Option<&Room> {
        self.active_room_id()
            .and_then(|id| self.teams_store.room_with_id(&id))
    }

    pub fn visible_rooms(&self) -> impl Iterator<Item = &Room> {
        self.teams_store
            .rooms_filtered_by(self.rooms_list.mode(), self.active_room_id())
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
            let pos_option = self.visible_rooms().position(|room| room.id == id);
            if let Some(position) = pos_option {
                self.rooms_list.table_state_mut().select(Some(position))
            }
        }
    }

    pub fn mark_active_read(&mut self) {
        if let Some(id) = self.id_of_selected_room() {
            self.teams_store.mark_read(&id);
        }
    }

    pub fn set_loading(&mut self, loading: bool) {
        self.is_loading = loading;
    }

    pub fn is_loading(&self) -> bool {
        self.is_loading
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
            messages_list: MessagesList::new(),
            rooms_list: RoomsList::new(),
        }
    }
}
