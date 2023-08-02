use tui_textarea::TextArea;

use super::{
    actions::{Action, Actions},
    teams_store::TeamsStore,
};

pub struct AppState<'a> {
    pub actions: Actions,
    pub active_room: Option<String>,
    pub editing_mode: bool,
    pub is_loading: bool,
    pub msg_input_textarea: TextArea<'a>,
    pub show_logs: bool,
    pub show_help: bool,
    pub teams_store: TeamsStore,
}

impl Default for AppState<'_> {
    fn default() -> Self {
        AppState {
            actions: vec![Action::Quit, Action::ToggleLogs].into(),
            active_room: None,
            editing_mode: false,
            is_loading: false,
            msg_input_textarea: TextArea::default(),
            show_logs: false,
            show_help: true,
            teams_store: TeamsStore::default(),
        }
    }
}
