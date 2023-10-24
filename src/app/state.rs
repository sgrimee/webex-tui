// app/state.rs

use enum_iterator::{next_cycle, Sequence};
use log::*;
use ratatui_textarea::TextArea;
use webex::Room;

use super::actions::{Action, Actions};
use super::messages_pane::MessagesList;
use super::rooms_pane::RoomsList;
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
    active_pane: Option<ActivePane>,
}

#[derive(Clone, Debug, PartialEq, Sequence, Default)]
pub enum ActivePane {
    #[default]
    Rooms,
    Messages,
    Compose,
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

    pub fn active_pane(&self) -> &Option<ActivePane> {
        &self.active_pane
    }

    pub fn set_active_pane(&mut self, active_pane: Option<ActivePane>) {
        self.active_pane = active_pane.clone();
        self.actions = match active_pane {
            Some(ActivePane::Compose) => vec![
                Action::EndEditMessage,
                Action::NextPane,
                Action::Quit,
                Action::SendMessage,
            ]
            .into(),
            Some(ActivePane::Messages) => vec![
                Action::NextPane,
                Action::NextMessage,
                Action::PreviousMessage,
                Action::Quit,
                Action::ToggleHelp,
                Action::ToggleLogs,
            ]
            .into(),
            Some(ActivePane::Rooms) => vec![
                Action::EditMessage,
                Action::MarkRead,
                Action::NextPane,
                Action::NextRoom,
                Action::NextRoomFilter,
                Action::PreviousRoom,
                Action::PreviousRoomFilter,
                Action::Quit,
                Action::SendMessage,
                Action::ToggleHelp,
                Action::ToggleLogs,
            ]
            .into(),
            None => { vec![Action::Quit, Action::ToggleHelp, Action::ToggleLogs] }.into(),
        }
    }

    pub fn cycle_active_pane(&mut self) {
        trace!("Previous pane: {:#?}", self.active_pane());
        match self.active_pane() {
            None => self.set_active_pane(Some(ActivePane::default())),
            Some(active_pane) => {
                self.set_active_pane(Some(next_cycle(active_pane).unwrap_or_default()))
            }
        }
        trace!("New pane: {:#?}", self.active_pane());
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
            active_pane: None,
        }
    }
}
