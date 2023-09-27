// app/state.rs

use enum_iterator::Sequence;
use ratatui::widgets::TableState;
use ratatui_textarea::TextArea;

use webex::Room;

use super::{
    actions::{Action, Actions},
    teams_store::{RoomId, TeamsStore},
};

// #[allow(dead_code)]
#[derive(Debug, PartialEq, Sequence)]
pub enum RoomsListMode {
    All,
    // Direct,
    // Public,
    Recent,
    // Spaces,
    Unread,
}

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
    pub room_list_state: TableState,
    pub room_list_mode: RoomsListMode,
}

impl Default for AppState<'_> {
    fn default() -> Self {
        let mut room_list_state = TableState::default();
        room_list_state.select(Some(0));
        let room_list_mode = RoomsListMode::All;
        AppState {
            actions: vec![Action::Quit, Action::ToggleHelp, Action::ToggleLogs].into(),
            editing_mode: false,
            is_loading: false,
            msg_input_textarea: TextArea::default(),
            show_logs: false,
            show_help: true,
            teams_store: TeamsStore::default(),
            room_list_state,
            room_list_mode,
        }
    }
}

impl AppState<'_> {
    pub fn selected_room_id(&self) -> Option<RoomId> {
        self.teams_store
            .rooms()
            .collect::<Vec<&Room>>()
            .get(
                self.room_list_state
                    .selected()
                    .expect("there is always a selected room"),
            )
            .map(|room| room.id.to_owned())
    }
}
