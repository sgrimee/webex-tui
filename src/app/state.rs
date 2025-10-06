//! State of the application

use color_eyre::{eyre::eyre, Result};
use enum_iterator::{next_cycle, previous_cycle, Sequence};
use itertools::concat;
use log::*;
use ratatui::layout::Rect;
use tui_logger::TuiWidgetState;
use webex::Message;

use super::actions::{Action, Actions};
use super::cache::room::{Room, RoomId};
use super::cache::Cache;
use super::message_editor::MessageEditor;
use super::messages_list::MessagesList;
use super::rooms_list::RoomsList;

/// State of the application, including
/// - available `actions`` in the current context
/// - whether `editing_mode` is enabled or not
/// - whether a background thread `is_loading`
/// - a `Cache` for Webex messages and rooms
///   and other UI state
pub(crate) struct AppState<'a> {
    // App
    pub(crate) actions: Actions,
    pub(crate) debug: bool,
    pub(crate) is_loading: bool,
    pub(crate) messages_to_load: u32,

    // Webex
    pub(crate) cache: Cache,

    // UI
    pub(crate) active_pane: Option<ActivePane>,
    pub(crate) last_frame_size: Rect,
    pub(crate) log_state: TuiWidgetState,
    pub(crate) message_editor: MessageEditor<'a>,
    pub(crate) messages_list: MessagesList,
    pub(crate) rooms_list: RoomsList,
    pub(crate) show_help: bool,
    pub(crate) show_logs: bool,
    pub(crate) show_rooms: bool,
}

/// The active pane is used by the UI to draw attention to what
/// key mappings are in use
#[derive(Clone, Debug, Default, PartialEq, Sequence)]
pub(crate) enum ActivePane {
    #[default]
    /// The list of rooms
    Rooms,
    /// The list of messages in the active room
    Messages,
    /// The text editor when composing a message
    Compose,
    /// Configurable logs output
    Logs,
    /// Room search mode
    Search,
}

impl AppState<'_> {
    /// Returns the active `Room` if any.
    /// This is the room displayed in the messages view and
    /// where messages are sent to.
    pub(crate) fn active_room(&self) -> Option<&Room> {
        self.rooms_list
            .active_room_id()
            .and_then(|id| self.cache.rooms.room_with_id(id))
    }

    /// Returns whether the given room is the active room.
    pub(crate) fn is_active_room(&self, room_id: &RoomId) -> bool {
        match self.rooms_list.active_room_id() {
            Some(id) => id == room_id,
            None => false,
        }
    }

    /// Returns an iterator over all visible rooms with the current filter.
    pub(crate) fn visible_rooms(&self) -> Box<dyn Iterator<Item = &Room> + '_> {
        if let Some(query) = self.rooms_list.search_query() {
            if !query.trim().is_empty() {
                // Return search results (just the rooms, ignoring scores)
                Box::new(self.cache.rooms_matching_search(query).map(|(room, _score)| room))
            } else {
                // Empty query - show all rooms with current filter
                Box::new(self.cache.rooms.rooms_filtered_by(self.rooms_list.filter()))
            }
        } else {
            // No search active - use normal filtering
            Box::new(self.cache.rooms.rooms_filtered_by(self.rooms_list.filter()))
        }
    }

    /// Returns the number of visible rooms with the current filter.
    pub(crate) fn num_of_visible_rooms(&self) -> usize {
        self.visible_rooms().collect::<Vec<_>>().len()
    }

    /// Returns the number of messages in the active room.
    pub(crate) fn num_messages_active_room(&self) -> usize {
        match self.rooms_list.active_room_id() {
            Some(id) => self.cache.nb_messages_in_room(id),
            None => 0,
        }
    }

    /// Returns the `RoomId` of the room selection in the list.
    /// This is used to set the active room.
    pub(crate) fn id_of_selected_room(&self) -> Option<RoomId> {
        self.rooms_list
            .id_of_selected(self.visible_rooms().collect::<Vec<_>>().as_slice())
    }

    /// Reset the room list selection to the active room.
    /// This is useful after the number or order of items in the list changes.
    pub(crate) fn update_room_selection_with_active_room(&mut self) {
        if let Some(id) = self.rooms_list.active_room_id() {
            let pos_option = self.visible_rooms().position(|room| room.id == *id);
            if let Some(position) = pos_option {
                self.rooms_list.table_state_mut().select(Some(position))
            }
        }
    }

    /// Mark the active room as being read.
    /// Only local storage for now, this is not synced between multiple clients,
    /// or multiple invocations of the same client.
    pub(crate) fn mark_active_read(&mut self) {
        if let Some(id) = self.id_of_selected_room() {
            self.cache.rooms.mark_read(&id);
        }
    }

    /// Returns the active pane.
    pub(crate) fn active_pane(&self) -> &Option<ActivePane> {
        &self.active_pane
    }

    /// Sets the active pane to `active_pane` and updates the list of possible actions
    /// according to what can be do in that pane.
    /// It also removes any message selection when switching to non Messages panes.
    pub(crate) fn set_active_pane(&mut self, active_pane: Option<ActivePane>) {
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
    pub(crate) fn update_actions(&mut self, active_pane: Option<ActivePane>) {
        let actions = match &active_pane {
            Some(ActivePane::Compose) => {
                vec![
                    Action::EndComposeMessage,
                    Action::SendMessage,
                    Action::NextPane,
                    Action::PreviousPane,
                    Action::ToggleDebug,
                    Action::ToggleHelp,
                    Action::ToggleLogs,
                    Action::ToggleRooms,
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
                    Action::DumpRoomContentToFile,
                    Action::NextPane,
                    Action::PreviousPane,
                    Action::ToggleDebug,
                    Action::ToggleHelp,
                    Action::ToggleLogs,
                    Action::ToggleRooms,
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
                    Action::StartRoomSearch,
                    Action::ToggleRoomSelection,
                    Action::SelectAllVisibleRooms,
                    Action::ClearRoomSelections,
                    Action::DeleteSelectedRooms,
                    Action::NextPane,
                    Action::PreviousPane,
                    Action::ToggleDebug,
                    Action::ToggleHelp,
                    Action::ToggleLogs,
                    Action::ToggleRooms,
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
            Some(ActivePane::Logs) => {
                vec![
                    Action::LogToggleTargetSelector,
                    Action::LogSelectNextTarget,
                    Action::LogSelectPreviousTarget,
                    Action::LogFocusSelectedTarget,
                    Action::LogIncreaseCapturedOneLevel,
                    Action::LogReduceCapturedOneLevel,
                    Action::LogIncreaseShownOneLevel,
                    Action::LogReduceShownOneLevel,
                    Action::LogPageUp,
                    Action::LogPageDown,
                    Action::LogExitPageMode,
                    Action::LogToggleFilteredTargets,
                    Action::NextPane,
                    Action::PreviousPane,
                    Action::ToggleDebug,
                    Action::ToggleHelp,
                    Action::ToggleLogs,
                    Action::ToggleRooms,
                    Action::Quit,
                ]
            }
            Some(ActivePane::Search) => {
                vec![
                    Action::NextRoom,
                    Action::PreviousRoom,
                    Action::EndRoomSearch,
                    Action::NextPane,
                    Action::PreviousPane,
                    Action::ToggleDebug,
                    Action::ToggleHelp,
                    Action::ToggleLogs,
                    Action::ToggleRooms,
                    Action::Quit,
                ]
            }
            None => {
                vec![
                    Action::ToggleHelp,
                    Action::ToggleLogs,
                    Action::ToggleRooms,
                    Action::ToggleDebug,
                    Action::Quit,
                ]
            }
        };
        self.actions = actions.into();
    }

    /// Selects the next active pane in a cycle.
    /// The message compose pane is skipped.
    pub(crate) fn next_active_pane(&mut self) {
        match self.active_pane() {
            None => self.set_active_pane(Some(ActivePane::default())),
            Some(active_pane) => {
                let mut next_pane = next_cycle(active_pane);
                // Skip the message compose pane
                if next_pane == ActivePane::Compose {
                    next_pane = next_cycle(&next_pane);
                };
                // Skip the search pane
                if next_pane == ActivePane::Search {
                    next_pane = next_cycle(&next_pane);
                };
                // Skip the logs pane if not enabled
                if next_pane == ActivePane::Logs && !self.show_logs {
                    next_pane = next_cycle(&next_pane);
                };
                // Skip the rooms pane if not enabled
                if next_pane == ActivePane::Rooms && !self.show_rooms {
                    next_pane = next_cycle(&next_pane);
                };
                self.set_active_pane(Some(next_pane))
            }
        }
    }

    /// Selects the previous active pane in a cycle.
    /// The message compose pane is skipped.
    pub(crate) fn previous_active_pane(&mut self) {
        match self.active_pane() {
            None => self.set_active_pane(Some(ActivePane::default())),
            Some(active_pane) => {
                let mut previous_pane = previous_cycle(active_pane);
                // Skip the message compose pane
                if previous_pane == ActivePane::Compose {
                    previous_pane = previous_cycle(&previous_pane);
                };
                // Skip the search pane
                if previous_pane == ActivePane::Search {
                    previous_pane = previous_cycle(&previous_pane);
                };
                // Skip the logs pane if not enabled
                if previous_pane == ActivePane::Logs && !self.show_logs {
                    previous_pane = previous_cycle(&previous_pane);
                };
                // Skip the rooms pane if not enabled
                if previous_pane == ActivePane::Rooms && !self.show_rooms {
                    previous_pane = previous_cycle(&previous_pane);
                };
                self.set_active_pane(Some(previous_pane))
            }
        }
    }

    pub(crate) fn update_on_tick(&mut self) {
        self.update_actions(self.active_pane.clone());
    }

    /// Returns the selected message, if there is one
    pub(crate) fn selected_message(&self) -> Result<&Message> {
        let room_id = self
            .id_of_selected_room()
            .ok_or(eyre!("No room selected"))?;
        let index = self
            .messages_list
            .selected_index()
            .ok_or(eyre!("No message selected in room {}", room_id))?;
        self.cache.nth_message_in_room(index, &room_id)
    }

    /// Returns true if the selected message is from me.
    pub(crate) fn selected_message_is_from_me(&self) -> Result<bool> {
        let message = self.selected_message()?;
        Ok(self.cache.is_me(&message.person_id))
    }
}

impl Default for AppState<'_> {
    fn default() -> Self {
        let log_state = TuiWidgetState::default();
        log_state.transition(tui_logger::TuiWidgetEvent::HideKey);
        AppState {
            actions: vec![Action::Quit, Action::ToggleHelp, Action::ToggleLogs, Action::ToggleRooms].into(),
            active_pane: None,
            cache: Cache::default(),
            debug: false,
            is_loading: false,
            last_frame_size: Rect::new(0, 0, 0, 0),
            log_state,
            messages_to_load: 10,
            message_editor: MessageEditor::default(),
            messages_list: MessagesList::new(),
            rooms_list: RoomsList::default(),
            show_help: true,
            show_logs: false,
            show_rooms: true,
        }
    }
}
