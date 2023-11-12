// app/state.rs

//! State of the application

use color_eyre::{eyre::eyre, Result};
use enum_iterator::{next_cycle, Sequence};
use itertools::concat;
use log::*;
use webex::{Message, Person, Room};

use super::actions::{Action, Actions};
use super::message_editor::MessageEditor;
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
    pub is_loading: bool,

    // Webex
    pub teams_store: TeamsStore,
    pub me: Option<webex::Person>,

    // UI
    pub show_logs: bool,
    pub show_help: bool,
    pub rooms_list: RoomsList,
    pub messages_list: MessagesList,
    pub message_editor: MessageEditor<'a>,
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
    /// Returns the active `Room` if any.
    /// This is the room displayed in the messages view and
    /// where messages are sent to.
    pub fn active_room(&self) -> Option<&Room> {
        self.rooms_list
            .active_room_id()
            .and_then(|id| self.teams_store.room_with_id(id))
    }

    /// Returns an iterator over all visible rooms with the current filter.
    pub fn visible_rooms(&self) -> impl Iterator<Item = &Room> {
        self.teams_store.rooms_filtered_by(self.rooms_list.filter())
    }

    /// Returns the number of visible rooms with the current filter.
    pub fn num_of_visible_rooms(&self) -> usize {
        self.visible_rooms().collect::<Vec<_>>().len()
    }

    /// Returns the number of messages in the active room.
    pub fn num_messages_active_room(&self) -> usize {
        match self.rooms_list.active_room_id() {
            Some(id) => self.teams_store.nb_messages_in_room(id),
            None => 0,
        }
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
        if let Some(id) = self.rooms_list.active_room_id() {
            let pos_option = self.visible_rooms().position(|room| &room.id == id);
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

    /// Sets the active pane to `active_pane` and updates the list of possible actions
    /// according to what can be do in that pane.
    /// It also removes any message selection when switching to non Messages panes.
    pub fn set_active_pane(&mut self, active_pane: Option<ActivePane>) {
        debug!("Activating pane: {:?}", active_pane);
        // Deselect messages when switching to non Messages panes
        match self.active_pane {
            Some(ActivePane::Messages) | Some(ActivePane::Compose) => (),
            _ => self.messages_list.deselect(),
        }
        self.update_actions(active_pane.clone());
        self.active_pane = active_pane;
    }

    /// Updates the list of possible actions according to what can be done in the pane
    pub fn update_actions(&mut self, active_pane: Option<ActivePane>) {
        let actions = match &active_pane {
            Some(ActivePane::Compose) => {
                vec![
                    Action::EndComposeMessage,
                    Action::SendMessage,
                    Action::NextPane,
                    Action::Quit,
                ]
            }
            Some(ActivePane::Messages) => {
                let mut actions: Vec<Action> = Vec::new();
                if self.rooms_list.active_room_id().is_some() {
                    actions.push(Action::ComposeNewMessage);
                }
                if self.num_messages_active_room() > 0 {
                    actions.extend(vec![Action::NextMessage, Action::PreviousMessage]);
                }
                if self.messages_list.has_selection() {
                    actions.push(Action::RespondMessage);
                    actions.push(Action::UnselectMessage);
                    if self.selected_message_is_from_me().unwrap_or_default() {
                        actions.push(Action::EditSelectedMessage);
                        actions.push(Action::DeleteMessage);
                    }
                }
                actions.extend(vec![
                    Action::NextPane,
                    Action::ToggleHelp,
                    Action::ToggleLogs,
                    Action::Quit,
                ]);
                actions
            }
            Some(ActivePane::Rooms) => {
                let common_actions = vec![
                    Action::NextRoom,
                    Action::PreviousRoom,
                    Action::NextRoomFilter,
                    Action::PreviousRoomFilter,
                    Action::NextPane,
                    Action::ToggleHelp,
                    Action::ToggleLogs,
                    Action::Quit,
                ];
                let selection_actions = vec![
                    Action::ComposeNewMessage,
                    Action::MarkRead,
                    Action::SendMessage,
                ];
                match self.rooms_list.has_selection() {
                    true => concat([selection_actions, common_actions]),
                    false => common_actions,
                }
            }
            None => {
                vec![Action::ToggleHelp, Action::ToggleLogs, Action::Quit]
            }
        };
        self.actions = actions.into();
    }

    /// Cycles between the room selection and message selection panes.
    /// The message compose pane is skipped.
    pub fn cycle_active_pane(&mut self) {
        match self.active_pane() {
            None => self.set_active_pane(Some(ActivePane::default())),
            Some(active_pane) => {
                let mut next_pane = next_cycle(active_pane).unwrap_or_default();
                // Skip the message compose pane
                if next_pane == ActivePane::Compose {
                    next_pane = next_cycle(&next_pane).unwrap_or_default();
                };
                self.set_active_pane(Some(next_pane))
            }
        }
    }

    pub(crate) fn update_on_tick(&mut self) {
        self.update_actions(self.active_pane.clone());
    }

    /// Returns the selected message, if there is one
    pub fn selected_message(&self) -> Result<&Message> {
        let room_id = self
            .id_of_selected_room()
            .ok_or(eyre!("No room selected"))?;
        let index = self
            .messages_list
            .selected_index()
            .ok_or(eyre!("No message selected in room {}", room_id))?;
        self.teams_store.nth_message_in_room(index, &room_id)
    }

    /// Sets the user of the app, used to filter its own messages.
    pub fn set_me(&mut self, me: Person) {
        self.me = Some(me);
    }

    /// Returns true if me is not None, person_id is not None and person_id equals me.
    /// Returns false if they are different or either is None.
    pub fn is_me(&self, person_id: &Option<String>) -> bool {
        match (&self.me, person_id) {
            (Some(me), Some(id)) => me.id.eq(id),
            _ => false,
        }
    }

    /// Returns true if the selected message is from me.
    pub fn selected_message_is_from_me(&self) -> Result<bool> {
        let message = self.selected_message()?;
        Ok(self.is_me(&message.person_id))
    }
}

impl Default for AppState<'_> {
    fn default() -> Self {
        AppState {
            actions: vec![Action::Quit, Action::ToggleHelp, Action::ToggleLogs].into(),
            is_loading: false,

            teams_store: TeamsStore::default(),
            me: None,
            show_logs: false,
            show_help: true,
            rooms_list: RoomsList::default(),
            messages_list: MessagesList::new(),
            message_editor: MessageEditor::default(),
            active_pane: None,
        }
    }
}
