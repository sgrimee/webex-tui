// rooms_list.rs

use enum_iterator::Sequence;
use ratatui::widgets::TableState;
use webex::Room;

use super::teams_store::{RoomId, TeamsStore};

#[derive(Debug, PartialEq, Sequence)]
pub enum RoomsListMode {
    All,
    // Direct,
    // Public,
    Recent,
    // Spaces,
    Unread,
}

pub struct RoomsList {
    active_room: Option<RoomId>,
    mode: RoomsListMode,
    state: TableState,
}

impl RoomsList {
    pub fn new_with_teams_store(teams_store: &mut TeamsStore) -> Self {
        Self {
            active_room: None,
            mode: RoomsListMode::All,
            state: TableState::default(),
        }
    }

    pub fn selected_room_id(&self) -> Option<RoomId> {
        match self.room_list_state.selected() {
            Some(selected) => self
                .teams_store
                .rooms()
                .collect::<Vec<&Room>>()
                .get(selected)
                .map(|room| room.id.to_owned()),
            None => None,
        }
    }
}
