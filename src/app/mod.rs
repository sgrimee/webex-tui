// app/mod.rs

//! Controller used to handle user input and interaction with the `Teams` thread.

pub(crate) mod actions;
pub(crate) mod cache;
pub(crate) mod callbacks;
pub(crate) mod message_editor;
pub(crate) mod messages_list;
pub(crate) mod rooms_list;
pub(crate) mod state;

use self::state::AppState;
use crate::app::actions::Action;
use crate::app::rooms_list::SearchState;
use crate::app::state::ActivePane;
use crate::inputs::key::Key;
use crate::teams::app_handler::AppCmdEvent;
use crate::theme::Theme;
use cache::room::RoomId;

use color_eyre::{eyre::eyre, Result};
use crossterm::event::KeyEvent;
use log::*;
use tui_logger::TuiWidgetEvent;
use tui_textarea::Input;
use webex::Message;

/// Return status indicating whether the app should exit or not.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AppReturn {
    Exit,
    Continue,
}

/// Priority level for events from the app to the webex thread
pub(crate) enum Priority {
    Low,
    High,
}

/// `App` contains the state of the application and a tx channel to the `Teams` thread.
pub(crate) struct App<'a> {
    app_to_teams_tx_low: tokio::sync::mpsc::UnboundedSender<AppCmdEvent>,
    app_to_teams_tx_high: tokio::sync::mpsc::UnboundedSender<AppCmdEvent>,
    pub(crate) state: AppState<'a>,
}

impl App<'_> {
    /// Returns an app with default state and the given channel to the `Teams` thread.
    ///
    /// # Arguments
    ///
    /// * `app_to_teams_tx` - An unbounded channel used to send commands to the `Teams` thread
    pub(crate) fn new(
        app_to_teams_tx_low: tokio::sync::mpsc::UnboundedSender<AppCmdEvent>,
        app_to_teams_tx_high: tokio::sync::mpsc::UnboundedSender<AppCmdEvent>,
        debug: bool,
        messages_to_load: u32,
        theme: Theme,
    ) -> Self {
        Self {
            app_to_teams_tx_low,
            app_to_teams_tx_high,
            state: AppState {
                debug,
                messages_to_load,
                theme,
                ..Default::default()
            },
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
                }
                Action::PreviousRoom => {
                    self.previous_room();
                }
                Action::UnselectMessage => {
                    self.state.messages_list.deselect();
                }
                Action::JumpToLastMessage => {
                    self.state.messages_list.select_last_message();
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
        self.state.update_on_tick();
        AppReturn::Continue
    }

    /// Send a message with the text contained in the editor
    /// to the active person or room.
    fn send_message_buffer(&mut self) -> Result<()> {
        if self.state.message_editor.is_empty() {
            return Err(eyre!("An empty message cannot be sent."));
        };
        let room = self
            .state
            .active_room()
            .ok_or(eyre!("Cannot send message, no room selected."))?;
        let lines = self.state.message_editor.lines().to_vec();
        if let Some(msg_to_edit) = self.state.message_editor.editing_of() {
            // Editing a message
            let msg_id = msg_to_edit
                .id
                .clone()
                .ok_or(eyre!("Cannot edit message without id"))?;
            let new_text = lines.join("\n");
            self.dispatch_to_teams(
                AppCmdEvent::EditMessage(msg_id, room.id.clone(), new_text),
                &Priority::High,
            );
        } else {
            let msg_to_send = match self.state.message_editor.response_to() {
                Some(orig_msg) => {
                    // Replying to a message
                    let mut reply = orig_msg.reply();
                    reply.text = Some(lines.join("\n"));
                    reply
                }
                None => webex::types::MessageOut {
                    // Sending a new message
                    room_id: Some(room.id.clone()),
                    text: Some(lines.join("\n")),
                    ..Default::default()
                },
            };
            self.dispatch_to_teams(AppCmdEvent::SendMessage(msg_to_send), &Priority::High);
        }
        debug!("Sending message to room {:?}", room.title);
        self.state.cache.rooms.mark_read(&room.id.clone());
        self.state.message_editor.reset();
        self.state.messages_list.deselect();
        Ok(())
    }

    /// Deletes the selected message, if there is one and it was authored by self.
    /// Otherwise does nothing.
    fn delete_selected_message(&mut self) -> Result<()> {
        let message = self.state.selected_message()?;
        let room_id = self.state.id_of_selected_room().unwrap();

        // Ensure we attempt to delete only our own messages
        if !self.state.cache.is_me(&message.person_id) {
            return Err(eyre!("Cannot delete message, it was not authored by self"));
        }

        let msg_id = message
            .id
            .clone()
            .ok_or(eyre!("Selected message does not have an id"))?;

        // Dispatch a delete event and remove the message from the store
        self.state.messages_list.select_previous_message();
        self.dispatch_to_teams(AppCmdEvent::DeleteMessage(msg_id.clone()), &Priority::High);
        self.state.cache.delete_message(&msg_id, &room_id)?;
        Ok(())
    }

    /// Extracts copyable text content from a message in priority order: text > markdown > html (converted)
    fn extract_message_content(message: &Message) -> Result<String> {
        match (&message.text, &message.markdown, &message.html) {
            (Some(text), _, _) => Ok(text.clone()),
            (None, Some(md), _) => Ok(md.clone()),
            (None, None, Some(html)) => {
                // Use html2text to convert HTML to plain text
                html2text::from_read(html.as_bytes(), usize::MAX)
                    .map_err(|e| {
                        warn!("Failed to convert HTML to plain text: {e}");
                        eyre!("Failed to convert HTML to plain text: {e}")
                    })
                    .or_else(|_| {
                        warn!("HTML conversion failed, using raw HTML as fallback");
                        Ok(html.clone())
                    })
            }
            _ => Err(eyre!("Message has no content")),
        }
    }

    /// Copies the selected message content to clipboard
    fn copy_selected_message(&mut self) -> Result<()> {
        use arboard::Clipboard;

        let message = self.state.selected_message()?;
        let content = Self::extract_message_content(message)?;

        let mut clipboard =
            Clipboard::new().map_err(|e| eyre!("Failed to access clipboard: {}", e))?;
        clipboard
            .set_text(content)
            .map_err(|e| eyre!("Failed to set clipboard content: {}", e))?;

        info!("Message copied to clipboard");
        Ok(())
    }

    /// Prepares the message editor to respond to the selected message
    fn respond_to_selected_message(&mut self) -> Result<()> {
        let message = self.state.selected_message()?;
        self.state
            .message_editor
            .set_response_to(Some(message.clone()));
        Ok(())
    }

    /// Prepeares the message editor with the contents of the selected message
    fn edit_selected_message(&mut self) -> Result<()> {
        let message = self.state.selected_message()?.clone();
        // return an error if the message is not from self
        if !self.state.cache.is_me(&message.person_id) {
            return Err(eyre!("Cannot edit message, it was not authored by self"));
        }

        // set the message editor text to the message text
        let text = message.text.clone().unwrap_or_default();
        self.state.message_editor.reset_with_text(text);
        // keep a copy of the message we are editing
        self.state
            .message_editor
            .set_editing_of(Some(message.clone()));
        Ok(())
    }

    /// Retrieves the latest messages in the room, only if it is empty
    fn get_messages_if_room_empty(&mut self, id: &RoomId) {
        if self.state.cache.room_is_empty(id) {
            self.dispatch_to_teams(
                AppCmdEvent::ListMessagesInRoom(id.clone(), None, self.state.messages_to_load),
                &Priority::High,
            );
        }
    }

    /// Retrieves messages before the first message in the room
    fn get_messages_before_first(&mut self, id: &RoomId) {
        if let Some(first_message) = self.state.cache.messages_in_room(id).next() {
            self.dispatch_to_teams(
                AppCmdEvent::ListMessagesInRoom(
                    id.clone(),
                    first_message.id.clone(),
                    self.state.messages_to_load,
                ),
                &Priority::High,
            );
        }
    }

    /// Send a command to the teams thread
    /// Does not block
    pub(crate) fn dispatch_to_teams(&self, action: AppCmdEvent, priority: &Priority) {
        let tx = match priority {
            Priority::High => &self.app_to_teams_tx_high,
            Priority::Low => &self.app_to_teams_tx_low,
        };
        if let Err(e) = tx.send(action) {
            error!("Error from dispatch {e}");
        };
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
        // Dump only specific fields of the message
        let messages: Vec<_> = self
            .state
            .cache
            .messages_in_room(&room.id)
            .map(|msg| Message {
                person_email: msg.person_email.clone(),
                text: msg.text.clone(),
                html: msg.html.clone(),
                markdown: msg.markdown.clone(),
                created: msg.created.clone(),
                id: msg.id.clone(),
                ..Default::default()
            })
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

        // Send leave room commands for all selected rooms
        for room_id in selected_room_ids.clone() {
            debug!("Sending leave room command for room: {room_id}");
            // Also log the room title if available
            if let Some(room) = self.state.cache.rooms.room_with_id(&room_id) {
                if let Some(title) = &room.title {
                    debug!("Room title: {title}");
                }
            }
            self.dispatch_to_teams(AppCmdEvent::LeaveRoom(room_id), &Priority::High);
        }

        // Clear selections after sending the commands
        self.state.rooms_list.clear_room_selections();
        debug!(
            "Cleared room selections, sent {} leave commands",
            selected_room_ids.len()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use webex::Message;

    /// Helper function to create a test message with specified content
    fn make_message(
        id: &str,
        text: Option<String>,
        markdown: Option<String>,
        html: Option<String>,
    ) -> Message {
        Message {
            id: Some(id.to_string()),
            text,
            markdown,
            html,
            created: Some(chrono::Utc::now().to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_extract_message_content_prioritizes_text() {
        let message = make_message(
            "msg1",
            Some("Plain text content".to_string()),
            Some("**Markdown** content".to_string()),
            Some("<p>HTML content</p>".to_string()),
        );

        let result = App::extract_message_content(&message).unwrap();
        assert_eq!(result, "Plain text content");
    }

    #[test]
    fn test_extract_message_content_falls_back_to_markdown() {
        let message = make_message(
            "msg1",
            None,
            Some("**Markdown** content".to_string()),
            Some("<p>HTML content</p>".to_string()),
        );

        let result = App::extract_message_content(&message).unwrap();
        assert_eq!(result, "**Markdown** content");
    }

    #[test]
    fn test_extract_message_content_converts_html_to_text() {
        let message = make_message(
            "msg1",
            None,
            None,
            Some("<p>Hello <strong>world</strong>!</p>".to_string()),
        );

        let result = App::extract_message_content(&message).unwrap();
        // html2text should convert HTML to plain text
        assert!(result.contains("Hello"));
        assert!(result.contains("world"));
        // Should not contain HTML tags
        assert!(!result.contains("<p>"));
        assert!(!result.contains("<strong>"));
    }

    #[test]
    fn test_extract_message_content_handles_malformed_html() {
        let message = make_message(
            "msg1",
            None,
            None,
            Some("<p>Malformed HTML <strong>without closing".to_string()),
        );

        // Should not panic and should return some content
        let result = App::extract_message_content(&message);
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(!content.is_empty());
    }

    #[test]
    fn test_extract_message_content_fails_with_no_content() {
        let message = make_message("msg1", None, None, None);

        let result = App::extract_message_content(&message);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Message has no content"));
    }

    #[test]
    fn test_extract_message_content_handles_empty_strings() {
        // Test with empty text (should use it even if empty)
        let message = make_message(
            "msg1",
            Some("".to_string()),
            Some("Markdown content".to_string()),
            None,
        );

        let result = App::extract_message_content(&message).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_extract_message_content_trims_whitespace_from_html() {
        let message = make_message(
            "msg1",
            None,
            None,
            Some("  <p>  Content with spaces  </p>  ".to_string()),
        );

        let result = App::extract_message_content(&message).unwrap();
        // html2text should handle whitespace appropriately
        assert!(result.contains("Content with spaces"));
    }
}
