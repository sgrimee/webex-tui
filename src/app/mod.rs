// app/mod.rs

//! Controller: handles user input and routes Webex reads/writes to the bridge glue.

pub(crate) mod actions;
mod bridge_app;
pub(crate) mod bridge_glue;
pub(crate) mod cache;
mod dump;
pub(crate) mod message_editor;
pub(crate) mod messages_list;
pub(crate) mod rooms_list;
pub(crate) mod state;

use self::state::AppState;
use crate::app::actions::Action;
use crate::app::rooms_list::SearchState;
use crate::app::state::ActivePane;
use crate::frontend::models::MessagePresentation;
use crate::inputs::key::Key;
use crate::theme::Theme;
use cache::room::RoomId;

use color_eyre::{eyre::eyre, Result};
use log::*;
use ratatui::crossterm::event::KeyEvent;
use tui_logger::TuiWidgetEvent;
use tui_textarea::Input;

/// Return status indicating whether the app should exit or not.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AppReturn {
    Exit,
    Continue,
}

/// `App` holds the UI state and the bridge glue.
pub(crate) struct App<'a> {
    pub(crate) state: AppState<'a>,
    /// Bridge backend (see bridge_app); `None` only before `attach_bridge`.
    bridge: Option<bridge_glue::BridgeGlue>,
    /// Leaves waiting for the single bridge write slot.
    pending_leaves: std::collections::VecDeque<RoomId>,
    /// Set by `begin_quit`: no further bridge writes are submitted.
    quitting: bool,
}

impl App<'_> {
    /// Returns an app with default state and no bridge attached yet.
    pub(crate) fn new(debug: bool, theme: Theme) -> Self {
        Self {
            state: AppState {
                debug,
                theme,
                ..Default::default()
            },
            bridge: None,
            pending_leaves: std::collections::VecDeque::new(),
            quitting: false,
        }
    }

    /// Process a key event to the text editor if active, or to execute
    /// the corresponding action otherwise
    pub(crate) async fn process_key_event(&mut self, key_event: KeyEvent) -> AppReturn {
        if self.state.message_editor.is_composing() {
            trace!("Keyevent: {key_event:?}");
            self.process_editing_key(key_event)
        } else if self.state.rooms_list.search_state() == &SearchState::Entering {
            trace!("Search entering keyevent: {key_event:?}");
            self.process_search_entering_key(key_event)
        } else {
            self.do_action(Key::from(key_event))
        }
    }

    /// Handle a user action (non-editing mode)
    fn do_action(&mut self, key: crate::inputs::key::Key) -> AppReturn {
        if let Some(action) = self.state.actions.find(key) {
            debug!("Run action [{action:?}]");
            match action {
                Action::DeleteMessage => {
                    if let Err(e) = self.delete_selected_message() {
                        error!("Could not delete message: {e}");
                    };
                }
                Action::DumpRoomContentToFile => {
                    if let Err(e) = self.dump_room_content_to_file() {
                        error!("Could not dump room content to file: {e}");
                    };
                }
                Action::ComposeNewMessage => {
                    self.state.message_editor.reset();
                    self.state.message_editor.set_is_composing(true);
                    self.state.set_active_pane(Some(ActivePane::Compose));
                }
                Action::EditSelectedMessage => {
                    if let Err(e) = self.edit_selected_message() {
                        error!("Could not respond to message: {e}");
                    };
                    self.state.message_editor.set_is_composing(true);
                    self.state.set_active_pane(Some(ActivePane::Compose));
                }
                Action::EndComposeMessage => {
                    self.state.message_editor.set_is_composing(false);
                    self.state.set_active_pane(Some(ActivePane::Messages));
                }
                Action::LogExitPageMode => {
                    self.state.log_state.transition(TuiWidgetEvent::EscapeKey);
                }
                Action::LogFocusSelectedTarget => {
                    self.state.log_state.transition(TuiWidgetEvent::FocusKey);
                }
                Action::LogIncreaseCapturedOneLevel => {
                    self.state.log_state.transition(TuiWidgetEvent::PlusKey);
                }
                Action::LogIncreaseShownOneLevel => {
                    self.state.log_state.transition(TuiWidgetEvent::RightKey);
                }
                Action::LogPageDown => {
                    self.state.log_state.transition(TuiWidgetEvent::NextPageKey);
                }
                Action::LogPageUp => {
                    self.state.log_state.transition(TuiWidgetEvent::PrevPageKey);
                }
                Action::LogReduceCapturedOneLevel => {
                    self.state.log_state.transition(TuiWidgetEvent::MinusKey);
                }
                Action::LogReduceShownOneLevel => {
                    self.state.log_state.transition(TuiWidgetEvent::LeftKey);
                }
                Action::LogSelectNextTarget => {
                    self.state.log_state.transition(TuiWidgetEvent::DownKey);
                }
                Action::LogSelectPreviousTarget => {
                    self.state.log_state.transition(TuiWidgetEvent::UpKey);
                }
                Action::LogToggleFilteredTargets => {
                    self.state.log_state.transition(TuiWidgetEvent::SpaceKey);
                }
                Action::LogToggleTargetSelector => {
                    self.state.log_state.transition(TuiWidgetEvent::HideKey);
                }
                Action::MarkRead => {
                    self.state.mark_active_read();
                    self.next_room();
                }
                Action::NextPane => {
                    self.state.next_active_pane();
                }
                Action::PreviousPane => {
                    self.state.previous_active_pane();
                }
                Action::NextRoomFilter => {
                    self.next_filtering_mode();
                }
                Action::PreviousRoomFilter => {
                    self.previous_filtering_mode();
                }
                Action::Quit => return AppReturn::Exit,
                Action::RespondMessage => {
                    if let Err(e) = self.respond_to_selected_message() {
                        error!("Could not respond to message: {e}");
                    };
                    self.state.message_editor.set_is_composing(true);
                    self.state.set_active_pane(Some(ActivePane::Compose));
                }
                Action::SendMessage => {
                    if let Err(e) = self.send_message_buffer() {
                        error!("Could not send message: {e}");
                    };
                }
                Action::ToggleDebug => {
                    self.state.debug = !self.state.debug;
                }
                Action::ToggleLogs => {
                    self.state.show_logs = !self.state.show_logs;
                    if !self.state.show_logs && self.state.active_pane == Some(ActivePane::Logs) {
                        self.state.next_active_pane();
                    }
                }
                Action::ToggleHelp => {
                    self.state.show_help = !self.state.show_help;
                }
                Action::ToggleRooms => {
                    self.state.show_rooms = !self.state.show_rooms;
                    if !self.state.show_rooms && self.state.active_pane == Some(ActivePane::Rooms) {
                        self.state.next_active_pane();
                    }
                }
                Action::NextMessage => {
                    self.state.messages_list.select_next_message();
                    self.bridge_selection_changed();
                }
                Action::NextRoom => {
                    self.next_room();
                }
                Action::PreviousMessage => {
                    let was_first = self.state.messages_list.select_previous_message();
                    if was_first {
                        let room_id = self.state.id_of_selected_room().unwrap();
                        self.get_messages_before_first(&room_id);
                    }
                    self.bridge_selection_changed();
                }
                Action::PreviousRoom => {
                    self.previous_room();
                }
                Action::UnselectMessage => {
                    self.state.messages_list.deselect();
                }
                Action::JumpToLastMessage => {
                    self.state.messages_list.select_last_message();
                    self.bridge_selection_changed();
                }
                Action::DismissWriteReview => {
                    self.bridge_dismiss_review();
                }
                Action::StartRoomSearch => {
                    // If already in filtering mode, keep the existing query but switch to entering mode
                    if self.state.rooms_list.search_state() == &SearchState::Filtering {
                        self.state
                            .rooms_list
                            .set_search_state(SearchState::Entering);
                    } else {
                        // Start fresh search
                        self.state.rooms_list.set_search_query(Some(String::new()));
                        self.state
                            .rooms_list
                            .set_search_state(SearchState::Entering);
                    }
                }
                Action::EndRoomSearch => {
                    match self.state.rooms_list.search_state() {
                        SearchState::Entering => {
                            // Switch to filtering mode if we have a non-empty query
                            if let Some(query) = self.state.rooms_list.search_query() {
                                if !query.is_empty() {
                                    self.state
                                        .rooms_list
                                        .set_search_state(SearchState::Filtering);
                                } else {
                                    // Empty query, clear search entirely
                                    self.state.rooms_list.clear_search();
                                }
                            } else {
                                self.state.rooms_list.set_search_state(SearchState::None);
                            }
                        }
                        SearchState::Filtering => {
                            // Exit filtering mode but keep the search query active
                            self.state.rooms_list.set_search_state(SearchState::None);
                        }
                        SearchState::None => {
                            // Do nothing - already in normal mode
                        }
                    }
                }
                Action::ToggleRoomSelection => {
                    // Collect room info first to avoid borrowing conflicts
                    let room_ids: Vec<_> =
                        self.state.visible_rooms().map(|r| r.id.clone()).collect();
                    let visible_rooms: Vec<_> = room_ids
                        .iter()
                        .filter_map(|id| self.state.cache.rooms.room_with_id(id))
                        .collect();
                    self.state
                        .rooms_list
                        .toggle_current_room_selection_and_advance(&visible_rooms);
                    self.set_active_room_to_selection();
                }
                Action::SelectAllVisibleRooms => {
                    // Collect room info first to avoid borrowing conflicts
                    let room_ids: Vec<_> =
                        self.state.visible_rooms().map(|r| r.id.clone()).collect();
                    let visible_rooms: Vec<_> = room_ids
                        .iter()
                        .filter_map(|id| self.state.cache.rooms.room_with_id(id))
                        .collect();
                    self.state
                        .rooms_list
                        .select_all_visible_rooms(&visible_rooms);
                }
                Action::InvertSelection => {
                    // Collect room info first to avoid borrowing conflicts
                    let room_ids: Vec<_> =
                        self.state.visible_rooms().map(|r| r.id.clone()).collect();
                    let visible_rooms: Vec<_> = room_ids
                        .iter()
                        .filter_map(|id| self.state.cache.rooms.room_with_id(id))
                        .collect();
                    self.state
                        .rooms_list
                        .invert_visible_room_selection(&visible_rooms);
                }
                Action::ClearRoomSelections => {
                    self.state.rooms_list.clear_room_selections();
                }
                Action::ClearSearchFilter => {
                    self.state.rooms_list.clear_search();
                }
                Action::CopyMessage => {
                    if let Err(e) = self.copy_selected_message() {
                        error!("Could not copy message: {e}");
                    };
                }
                Action::DeleteSelectedRooms => {
                    debug!("User triggered DeleteSelectedRooms action");
                    if let Err(e) = self.delete_selected_rooms() {
                        error!("Could not delete selected rooms: {e}");
                    }
                }
            }
        } else {
            warn!("No action associated with {key} in this mode");
            // If the key actually corresponds to an action, it needs to be added to the list of active actions too.
        }
        AppReturn::Continue
    }

    // Handle a key while in text editing mode
    fn process_editing_key(&mut self, key_event: KeyEvent) -> AppReturn {
        let key: Key = key_event.into();
        match key {
            Key::Ctrl('c') => return AppReturn::Exit,
            Key::Esc => {
                self.state.message_editor.set_is_composing(false);
                self.state.set_active_pane(Some(ActivePane::Messages))
            }
            Key::AltEnter => self.state.message_editor.insert_newline(),
            Key::Enter => {
                if let Err(e) = self.send_message_buffer() {
                    error!("Could not send message: {e}");
                };
            }
            _ => _ = self.state.message_editor.input(Input::from(key_event)),
        }
        AppReturn::Continue
    }

    // Handle a key while in search entering mode
    fn process_search_entering_key(&mut self, key_event: KeyEvent) -> AppReturn {
        let key: Key = key_event.into();
        match key {
            Key::Ctrl('c') => return AppReturn::Exit,
            Key::Esc => {
                // Exit search entering mode
                return self.do_action(Key::Esc);
            }
            Key::Enter => {
                // Switch to filtering mode
                return self.do_action(Key::Esc);
            }
            Key::Up => {
                // Navigate search results
                let num_rooms = self.state.num_of_visible_rooms();
                self.state.rooms_list.select_previous_room(num_rooms);
            }
            Key::Down => {
                // Navigate search results
                let num_rooms = self.state.num_of_visible_rooms();
                self.state.rooms_list.select_next_room(num_rooms);
            }
            Key::Backspace => {
                // Remove last character from search query
                if let Some(query) = self.state.rooms_list.search_query() {
                    let mut new_query = query.clone();
                    new_query.pop();
                    if new_query.is_empty() {
                        // If query becomes empty, clear search entirely
                        self.state.rooms_list.clear_search();
                    } else {
                        self.state.rooms_list.set_search_query(Some(new_query));
                        // Reset selection when search changes
                        let num_rooms = self.state.num_of_visible_rooms();
                        let selected = if num_rooms == 0 { None } else { Some(0) };
                        self.state.rooms_list.table_state_mut().select(selected);
                    }
                }
            }
            Key::Char(c) => {
                // Add character to search query (including '/')
                let query = self
                    .state
                    .rooms_list
                    .search_query()
                    .map(|q| format!("{q}{c}"))
                    .unwrap_or_else(|| c.to_string());
                self.state.rooms_list.set_search_query(Some(query));
                // Reset selection when search changes
                let num_rooms = self.state.num_of_visible_rooms();
                let selected = if num_rooms == 0 { None } else { Some(0) };
                self.state.rooms_list.table_state_mut().select(selected);
            }
            _ => {
                // Handle other actions like NextPane, etc.
                return self.do_action(key);
            }
        }
        AppReturn::Continue
    }

    /// We could update the app or dispatch event on tick
    pub(crate) async fn update_on_tick(&mut self) -> AppReturn {
        self.bridge_tick();
        self.state.update_on_tick();
        AppReturn::Continue
    }

    /// Submit the editor's composition (send, reply or edit) to the bridge.
    fn send_message_buffer(&mut self) -> Result<()> {
        self.bridge_send()
    }

    /// Deletes the selected message, if there is one and it was authored by self.
    /// Otherwise does nothing.
    fn delete_selected_message(&mut self) -> Result<()> {
        let message = self.state.selected_actionable_message()?;
        let room_id = self.state.id_of_selected_room().unwrap();

        // Ensure we attempt to delete only our own messages.
        if !self
            .state
            .cache
            .is_me(&message.sender_id.as_ref().map(|id| id.as_str().to_owned()))
        {
            return Err(eyre!("Cannot delete message, it was not authored by self"));
        }

        // Only a confirmed delete tombstones it; selection stays put.
        let id = message.id.clone();
        self.bridge_delete(&room_id, &id)
    }

    /// Copies the selected message content to clipboard
    fn copy_selected_message(&mut self) -> Result<()> {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let message = self.state.selected_actionable_message()?;
        let content = MessagePresentation::from(message).body.copy_text()?;

        // On Wayland, use wl-copy for better compatibility
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            let mut child = Command::new("wl-copy")
                .stdin(Stdio::piped())
                .spawn()
                .map_err(|e| eyre!("Failed to spawn wl-copy: {}", e))?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(content.as_bytes())
                    .map_err(|e| eyre!("Failed to write to wl-copy: {}", e))?;
            }
            let status = child
                .wait()
                .map_err(|e| eyre!("Failed to wait for wl-copy: {}", e))?;
            if !status.success() {
                return Err(eyre!("wl-copy exited with status: {}", status));
            }
        } else {
            // Fallback to arboard for X11 or other
            use arboard::Clipboard;
            let mut clipboard =
                Clipboard::new().map_err(|e| eyre!("Failed to access clipboard: {}", e))?;
            clipboard
                .set_text(content)
                .map_err(|e| eyre!("Failed to set clipboard content: {}", e))?;
        }

        info!("Message copied to clipboard");
        Ok(())
    }

    /// Prepares the message editor to respond to the selected message
    fn respond_to_selected_message(&mut self) -> Result<()> {
        let message = self.state.selected_reply_target()?;
        self.state
            .message_editor
            .set_response_to(Some(message.clone()));
        Ok(())
    }

    /// Prepeares the message editor with the contents of the selected message
    fn edit_selected_message(&mut self) -> Result<()> {
        let message = self.state.selected_actionable_message()?.clone();
        if !self
            .state
            .cache
            .is_me(&message.sender_id.as_ref().map(|id| id.as_str().to_owned()))
        {
            return Err(eyre!("Cannot edit message, it was not authored by self"));
        }
        self.state.message_editor.start_editing(message)
    }

    /// Loads the newest page of the room once (bridge).
    fn get_messages_if_room_empty(&mut self, id: &RoomId) {
        self.bridge_open_room(id);
    }

    /// Loads the page before the oldest loaded message (bridge).
    fn get_messages_before_first(&mut self, id: &RoomId) {
        self.bridge_load_older(id);
    }

    /// Sets the active room to that highlighted by the list selection
    fn set_active_room_to_selection(&mut self) {
        let id_option = self.state.id_of_selected_room();
        self.state.rooms_list.set_active_room_id(id_option.clone());
        // Changing active room may have affected the selection
        // e.g. with Unread filter which includes active room
        self.state.update_room_selection_with_active_room();
        if let Some(id) = id_option {
            self.get_messages_if_room_empty(&id);
        }
        // Update the number of messages in the active room
        self.state
            .messages_list
            .set_nb_messages(self.state.num_messages_active_room());
        // Deselect the message selection
        self.state.messages_list.deselect();
    }

    /// Change the rooms list filter to the previous one
    fn previous_filtering_mode(&mut self) {
        self.state.rooms_list.set_active_room_id(None);
        self.state.rooms_list.previous_filter(&self.state.cache);
        self.set_active_room_to_selection();
    }

    /// Change the rooms list filter to the next one
    fn next_filtering_mode(&mut self) {
        self.state.rooms_list.set_active_room_id(None);
        self.state.rooms_list.next_filter(&self.state.cache);
        self.set_active_room_to_selection();
    }

    /// Select the next room in the list
    fn next_room(&mut self) {
        let num_rooms = self.state.num_of_visible_rooms();
        self.state.rooms_list.select_next_room(num_rooms);
        self.set_active_room_to_selection();
    }

    /// Select the previous room in the list
    fn previous_room(&mut self) {
        let num_rooms = self.state.num_of_visible_rooms();
        self.state.rooms_list.select_previous_room(num_rooms);
        self.set_active_room_to_selection();
    }

    fn dump_room_content_to_file(&self) -> Result<()> {
        // Write the messages in the room to a json file
        let room = self
            .state
            .active_room()
            .ok_or(eyre!("Cannot dump room content to file, no room selected."))?;
        let filename = format!("room_{}.json", room.id);
        let file = std::fs::File::create(filename)?;
        let messages: Vec<_> = self
            .state
            .cache
            .messages_in_room(&room.id)
            .map(crate::app::dump::MessageDumpView::from)
            .collect();
        serde_json::to_writer_pretty(file, &messages)?;
        Ok(())
    }

    /// Delete all selected rooms by leaving them
    fn delete_selected_rooms(&mut self) -> Result<()> {
        let selected_room_ids = self.state.rooms_list.selected_room_ids();
        debug!("Selected room IDs for deletion: {selected_room_ids:?}");

        if selected_room_ids.is_empty() {
            debug!("No rooms selected for deletion");
            return Err(eyre!("No rooms selected for deletion"));
        }

        let count = selected_room_ids.len();
        self.bridge_leave_rooms(selected_room_ids);
        self.state.rooms_list.clear_room_selections();
        debug!("Queued {count} bridge leave(s)");
        Ok(())
    }
}
