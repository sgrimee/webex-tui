// app/state.rs

//! State of the application

use enum_iterator::{next_cycle, Sequence};
use log::*;
use ratatui_textarea::TextArea;
use webex::Room;

use super::actions::{Action, Actions};
use super::messages_list::MessagesList;
use super::rooms_list::RoomsList;
use super::teams_store::{RoomId, TeamsStore};

/// State of the application, including
/// - available `actions`` in the current context
/// - whether `editing_mode` is enabled or not
/// - whether a background thread `is_loading`
/// - a `teams_store` cache for Webex messages and rooms
/// and other UI state
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
    pub messages_list: MessagesList,
    active_pane: Option<ActivePane>,
}

/// The active pane is used by the UI to draw attention to what
/// key mappings are in use
#[derive(Clone, Debug, PartialEq, Sequence, Default)]
pub enum ActivePane {
    #[default]
    /// The list of rooms
    Rooms,
    /// The list of messages in the active room
    Messages,
    /// The text editor when composing a message
    Compose,
}

impl AppState<'_> {
    /// Return the `RoomId` of the active room, if any.
    pub fn active_room_id(&self) -> Option<RoomId> {
        self.rooms_list.active_room_id.clone()
    }

    /// Sets the active room to `id`
    pub fn set_active_room_id(&mut self, id: &Option<RoomId>) {
        self.rooms_list.active_room_id = id.clone();
    }

    /// Returns the active `Room` if any.
    /// This is the room displayed in the messages view and
    /// where messages are sent to.
    pub fn active_room(&self) -> Option<&Room> {
        self.active_room_id()
            .and_then(|id| self.teams_store.room_with_id(&id))
    }

    /// Returns an iterator over all visible rooms with the current filter.
    pub fn visible_rooms(&self) -> impl Iterator<Item = &Room> {
        self.teams_store
            .rooms_filtered_by(self.rooms_list.filter(), self.active_room_id())
    }

    /// Returns the number of visible rooms with the current filter.
    pub fn num_of_visible_rooms(&self) -> usize {
        self.visible_rooms().collect::<Vec<_>>().len()
    }

    /// Returns the `RoomId` of the room selection in the list.
    /// This is used to set the active room.
    pub fn id_of_selected_room(&self) -> Option<RoomId> {
        self.rooms_list
            .id_of_selected(self.visible_rooms().collect::<Vec<_>>().as_slice())
    }

    /// Reset the list selection to the active room.
    /// This is useful after the number or order of items in the list changes.
    pub fn update_selection_with_active_room(&mut self) {
        if let Some(id) = self.active_room_id() {
            let pos_option = self.visible_rooms().position(|room| room.id == id);
            if let Some(position) = pos_option {
                self.rooms_list.table_state_mut().select(Some(position))
            }
        }
    }

    /// Mark the active room as being read.
    /// Only local storage for now, this is not synced between multiple clients,
    /// or multiple invocations of the same client.
    pub fn mark_active_read(&mut self) {
        if let Some(id) = self.id_of_selected_room() {
            self.teams_store.mark_read(&id);
        }
    }

    /// Returns the active pane.
    pub fn active_pane(&self) -> &Option<ActivePane> {
        &self.active_pane
    }

    /// Sets the active pane to `active_pane` and updates the list of possible
    /// according to what can be do in that pane.
    pub fn set_active_pane_and_actions(&mut self, active_pane: Option<ActivePane>) {
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

    /// Selects the next active pane, cycling through all panes.
    pub fn cycle_active_pane(&mut self) {
        trace!("Previous pane: {:#?}", self.active_pane());
        match self.active_pane() {
            None => self.set_active_pane_and_actions(Some(ActivePane::default())),
            Some(active_pane) => {
                self.set_active_pane_and_actions(Some(next_cycle(active_pane).unwrap_or_default()))
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
