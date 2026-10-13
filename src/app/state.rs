//! State of the application

use crate::frontend::data::MessageSnapshot;
use color_eyre::{eyre::eyre, Result};
use enum_iterator::{next_cycle, previous_cycle, Sequence};
use itertools::concat;
use log::*;
use ratatui::layout::Rect;
use tui_logger::TuiWidgetState;

use super::actions::{Action, Actions};
use super::cache::room::{Room, RoomId};
use super::cache::Cache;
use super::message_editor::MessageEditor;
use super::messages_list::MessagesList;
use super::rooms_list::RoomsList;
use crate::theme::Theme;

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

    // Theme
    pub(crate) theme: Theme,

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
                // Search only within the current filter
                Box::new(
                    self.cache
                        .rooms_matching_search(query, self.rooms_list.filter())
                        .map(|(room, _score)| room),
                )
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
        debug!("Activating pane: {active_pane:?}");
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
                    actions.extend(vec![
                        Action::NextMessage,
                        Action::PreviousMessage,
                        Action::JumpToLastMessage,
                    ]);
                }
                if self.messages_list.has_selection() {
                    // A tombstone or placeholder row is selectable, but no
                    // message action may target it.
                    if self.selected_actionable_message().is_ok() {
                        actions.push(Action::CopyMessage);
                        actions.push(Action::RespondMessage);
                    }
                    actions.push(Action::UnselectMessage);
                    if self.selected_actionable_message().is_ok()
                        && self.selected_message_is_from_me().unwrap_or_default()
                    {
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
                // Check if we're in search entering mode
                if matches!(
                    self.rooms_list.search_state(),
                    super::rooms_list::SearchState::Entering
                ) {
                    // In search entering mode, only show essential actions
                    vec![
                        Action::EndRoomSearch, // Esc to exit search
                        Action::Quit,
                    ]
                } else {
                    // Normal rooms mode or search filtering mode
                    let common_actions = vec![
                        Action::NextRoom,
                        Action::PreviousRoom,
                        Action::NextRoomFilter,
                        Action::PreviousRoomFilter,
                        Action::StartRoomSearch,
                        Action::ToggleRoomSelection,
                        Action::SelectAllVisibleRooms,
                        Action::InvertSelection,
                        Action::ClearRoomSelections,
                        Action::ClearSearchFilter,
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
    pub(crate) fn selected_message(&self) -> Result<&MessageSnapshot> {
        let room_id = self
            .id_of_selected_room()
            .ok_or(eyre!("No room selected"))?;
        let index = self
            .messages_list
            .selected_index()
            .ok_or(eyre!("No message selected in room {}", room_id))?;
        self.cache.nth_message_in_room(index, &room_id)
    }

    /// The selected message, only if a user action (reply, edit, delete,
    /// copy) may target it. A deleted-root tombstone row is selectable, so the
    /// selection can stay anchored in its thread, but it is not actionable.
    pub(crate) fn selected_actionable_message(&self) -> Result<&MessageSnapshot> {
        actionable(self.selected_message()?)
    }

    /// Returns true if the selected message is from me.
    pub(crate) fn selected_message_is_from_me(&self) -> Result<bool> {
        let message = self.selected_message()?;
        Ok(self
            .cache
            .is_me(&message.sender_id.as_ref().map(|id| id.as_str().to_owned())))
    }

    /// Set the theme for the application
    #[allow(dead_code)]
    pub(crate) fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }
}

impl Default for AppState<'_> {
    fn default() -> Self {
        let log_state = TuiWidgetState::default();
        log_state.transition(tui_logger::TuiWidgetEvent::HideKey);
        AppState {
            actions: vec![
                Action::Quit,
                Action::ToggleHelp,
                Action::ToggleLogs,
                Action::ToggleRooms,
            ]
            .into(),
            active_pane: None,
            cache: Cache::default(),
            theme: Theme::default(),
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

/// Refuse tombstones as action targets (REVIEW-14 nit b).
fn actionable(message: &MessageSnapshot) -> Result<&MessageSnapshot> {
    if message.status == crate::frontend::data::MessageStatus::Active {
        Ok(message)
    } else {
        Err(eyre!("The selected message was deleted"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::data::{MessageId, MessageStatus, RoomId};

    #[test]
    fn action_hints_are_not_offered_for_tombstone_or_placeholder_rows() {
        use crate::frontend::data::{
            OwnedMessageBody, PersonId, PersonSnapshot, RoomKind, RoomSnapshot,
        };
        let mut state = AppState::default();
        let room_id: super::RoomId = "room".into();
        let room = Room::from_bridge_snapshot(&RoomSnapshot {
            id: crate::frontend::data::RoomId::new("room"),
            title: Some("synthetic".into()),
            kind: RoomKind::Group,
            team_id: None,
            last_activity: None,
            is_member: true,
            is_locked: None,
        })
        .unwrap();
        state.cache.rooms.update_with_room(&room);
        state.cache.set_me(PersonSnapshot {
            id: PersonId::new("me"),
            display_name: None,
            emails: None,
        });
        let snapshot = |id: &str, parent: Option<&str>, minute: u32| MessageSnapshot {
            id: MessageId::new(id),
            room_id: RoomId::new("room"),
            revision: Some(1),
            parent_id: parent.map(MessageId::new),
            sender_id: Some(PersonId::new("me")),
            sender_email: None,
            created_at: Some(format!("2026-01-01T00:0{minute}:00Z")),
            updated_at: None,
            status: MessageStatus::Active,
            body: Some(OwnedMessageBody::Forms {
                text: Some(id.into()),
                markdown: None,
                html: None,
            }),
            attachment_count: None,
        };
        let view = state.cache.canonical_view_mut(&room_id).unwrap();
        view.upsert(&snapshot("root", None, 1)).unwrap();
        view.upsert(&snapshot("reply", Some("root"), 2)).unwrap();
        view.upsert(&snapshot("orphan", Some("unloaded"), 3))
            .unwrap();
        view.tombstone(&RoomId::new("room"), &MessageId::new("root"), None)
            .unwrap();
        // Rows: deleted root, reply, missing-root placeholder, orphan.
        state.rooms_list.set_active_room_id(Some(room_id.clone()));
        state.update_room_selection_with_active_room();
        state
            .messages_list
            .set_nb_messages(state.num_messages_active_room());
        let hints = |state: &mut AppState, index: usize| {
            state.messages_list.select_index(index);
            state.update_actions(Some(ActivePane::Messages));
            let actions = state.actions.actions();
            [
                Action::CopyMessage,
                Action::RespondMessage,
                Action::EditSelectedMessage,
                Action::DeleteMessage,
            ]
            .map(|action| actions.contains(&action))
        };
        assert_eq!(hints(&mut state, 0), [false; 4], "deleted root");
        assert_eq!(hints(&mut state, 1), [true; 4], "own live reply");
        assert_eq!(hints(&mut state, 2), [false; 4], "missing-root placeholder");
        assert!(state.actions.actions().contains(&Action::UnselectMessage));
    }

    #[test]
    fn tombstones_are_not_action_targets() {
        let mut message = MessageSnapshot {
            id: MessageId::new("root"),
            room_id: RoomId::new("room"),
            revision: Some(1),
            parent_id: None,
            sender_id: None,
            sender_email: None,
            created_at: None,
            updated_at: None,
            status: MessageStatus::Active,
            body: None,
            attachment_count: None,
        };
        assert!(actionable(&message).is_ok());
        message.status = MessageStatus::Deleted;
        assert!(actionable(&message).is_err());
    }
}
