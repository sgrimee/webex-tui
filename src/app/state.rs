use ratatui::widgets::TableState;
use tui_textarea::TextArea;
use webex::Room;

use super::{
    actions::{Action, Actions},
    teams_store::{RoomId, TeamsStore},
};

pub struct AppState<'a> {
    pub actions: Actions,
    pub editing_mode: bool,
    pub is_loading: bool,
    pub msg_input_textarea: TextArea<'a>,
    pub show_logs: bool,
    pub show_help: bool,
    pub teams_store: TeamsStore,
    pub room_list_state: TableState,
}

impl Default for AppState<'_> {
    fn default() -> Self {
        let mut room_list_state = TableState::default();
        room_list_state.select(Some(0));
        AppState {
            actions: vec![Action::Quit, Action::ToggleLogs].into(),
            editing_mode: false,
            is_loading: false,
            msg_input_textarea: TextArea::default(),
            show_logs: false,
            show_help: true,
            teams_store: TeamsStore::default(),
            room_list_state,
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
