// app/mod.rs

//! Controller used to handle user input and interaction with the `Teams` thread.

pub mod actions;
pub mod callbacks;
pub mod message_editor;
pub mod messages_list;
pub mod rooms_list;
pub mod state;
pub mod teams_store;

use self::state::AppState;
use crate::app::actions::Action;
use crate::app::state::ActivePane;
use crate::inputs::key::Key;
use crate::teams::app_handler::AppCmdEvent;
use teams_store::room::RoomId;

use color_eyre::{eyre::eyre, Result};
use crossterm::event::KeyEvent;
use log::*;
use tui_textarea::Input;

/// Return status indicating whether the app should exit or not.
#[derive(Debug, PartialEq, Eq)]
pub enum AppReturn {
    Exit,
    Continue,
}

/// `App` contains the state of the application and a tx channel to the `Teams` thread.
pub struct App<'a> {
    app_to_teams_tx: tokio::sync::mpsc::UnboundedSender<AppCmdEvent>,
    pub state: AppState<'a>,
}

impl App<'_> {
    /// Returns an app with default state and the given channel to the `Teams` thread.
    ///
    /// # Arguments
    ///
    /// * `app_to_teams_tx` - An unbounded channel used to send commands to the `Teams` thread
    pub fn new(app_to_teams_tx: tokio::sync::mpsc::UnboundedSender<AppCmdEvent>) -> Self {
        Self {
            app_to_teams_tx,
            state: AppState::default(),
        }
    }

    /// Process a key event to the text editor if active, or to execute
    /// the corresponding action otherwise
    pub async fn process_key_event(&mut self, key_event: KeyEvent) -> AppReturn {
        if self.state.message_editor.is_composing() {
            trace!("Keyevent: {:?}", key_event);
            self.process_editing_key(key_event)
        } else {
            self.do_action(Key::from(key_event))
        }
    }

    /// Handle a user action (non-editing mode)
    fn do_action(&mut self, key: crate::inputs::key::Key) -> AppReturn {
        if let Some(action) = self.state.actions.find(key) {
            debug!("Run action [{:?}]", action);
            match action {
                Action::DeleteMessage => {
                    if let Err(e) = self.delete_selected_message() {
                        error!("Could not delete message: {}", e);
                    };
                }
                Action::ComposeNewMessage => {
                    self.state.message_editor.reset();
                    self.state.message_editor.set_is_composing(true);
                    self.state.set_active_pane(Some(ActivePane::Compose));
                }
                Action::EditSelectedMessage => {
                    if let Err(e) = self.edit_selected_message() {
                        error!("Could not respond to message: {}", e);
                    };
                    self.state.message_editor.set_is_composing(true);
                    self.state.set_active_pane(Some(ActivePane::Compose));
                }
                Action::MarkRead => {
                    self.state.mark_active_read();
                    self.next_room();
                }
                Action::NextPane => {
                    self.state.cycle_active_pane();
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
                        error!("Could not respond to message: {}", e);
                    };
                    self.state.message_editor.set_is_composing(true);
                    self.state.set_active_pane(Some(ActivePane::Compose));
                }
                Action::SendMessage => {
                    if let Err(e) = self.send_message_buffer() {
                        error!("Could not send message: {}", e);
                    };
                }
                Action::ToggleLogs => {
                    self.state.show_logs = !self.state.show_logs;
                }
                Action::ToggleHelp => {
                    self.state.show_help = !self.state.show_help;
                }
                Action::NextMessage => {
                    self.state.messages_list.select_next_message();
                }
                Action::NextRoom => {
                    self.next_room();
                }
                Action::PreviousMessage => {
                    self.state.messages_list.select_previous_message();
                }
                Action::PreviousRoom => {
                    self.previous_room();
                }
                Action::UnselectMessage => {
                    self.state.messages_list.deselect();
                }
                _ => {
                    warn!("Unsupported action {} in this context", action);
                }
            }
        } else {
            warn!("No action associated with {} in this mode", key);
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
                    error!("Could not send message: {}", e);
                };
            }
            _ => _ = self.state.message_editor.input(Input::from(key_event)),
        }
        AppReturn::Continue
    }

    /// We could update the app or dispatch event on tick
    pub async fn update_on_tick(&mut self) -> AppReturn {
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
        let room_id = room.id().clone();

        let lines = self.state.message_editor.lines().to_vec();
        if let Some(msg_to_edit) = self.state.message_editor.editing_of() {
            // Editing a message
            let msg_id = msg_to_edit
                .id
                .clone()
                .ok_or(eyre!("Cannot edit message without id"))?;
            let new_text = lines.join("\n");
            self.dispatch_to_teams(AppCmdEvent::EditMessage(msg_id, room_id.clone(), new_text));
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
                    room_id: Some(room_id.clone()),
                    text: Some(lines.join("\n")),
                    ..Default::default()
                },
            };
            self.dispatch_to_teams(AppCmdEvent::SendMessage(msg_to_send));
        }
        debug!("Sending message to room {:?}", room.title());
        self.state.message_editor.reset();
        self.state.teams_store.rooms_info.mark_read(&room_id);
        self.state.messages_list.deselect();
        Ok(())
    }

    /// Deletes the selected message, if there is one and it was authored by self.
    /// Otherwise does nothing.
    fn delete_selected_message(&mut self) -> Result<()> {
        let message = self.state.selected_message()?;
        let room_id = self.state.id_of_selected_room().unwrap();

        // Ensure we attempt to delete only our own messages
        if !self.state.is_me(&message.person_id) {
            return Err(eyre!("Cannot delete message, it was not authored by self"));
        }

        let msg_id = message
            .id
            .clone()
            .ok_or(eyre!("Selected message does not have an id"))?;

        // Dispatch a delete event and remove the message from the store
        self.state.messages_list.select_previous_message();
        self.dispatch_to_teams(AppCmdEvent::DeleteMessage(msg_id.clone()));
        self.state.teams_store.delete_message(&msg_id, &room_id)?;
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
        if !self.state.is_me(&message.person_id) {
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
        if self.state.teams_store.room_is_empty(id) {
            self.dispatch_to_teams(AppCmdEvent::ListMessagesInRoom(id.clone()));
        }
    }

    /// Send a command to the teams thread
    /// Does not block
    pub fn dispatch_to_teams(&self, action: AppCmdEvent) {
        if let Err(e) = self.app_to_teams_tx.send(action) {
            error!("Error from dispatch {}", e);
        };
    }

    /// Sets the active room to that highlighted by the list selection
    fn set_active_room_to_selection(&mut self) {
        let id_option = self.state.id_of_selected_room();
        self.state.rooms_list.set_active_room_id(id_option.clone());
        // Changing active room may have affected the selection
        // e.g. with Unread filter which includes active room
        self.state.update_selection_with_active_room();
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
        self.state
            .rooms_list
            .previous_filter(&self.state.teams_store);
        self.set_active_room_to_selection();
    }

    /// Change the rooms list filter to the next one
    fn next_filtering_mode(&mut self) {
        self.state.rooms_list.set_active_room_id(None);
        self.state.rooms_list.next_filter(&self.state.teams_store);
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
}
