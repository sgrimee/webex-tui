// app/mod.rs

//! Controller used to handle user input and interaction with the `Teams` thread.

pub mod actions;
pub mod callbacks;
pub mod messages_list;
pub mod rooms_list;
pub mod state;
pub mod teams_store;

use self::{state::AppState, teams_store::RoomId};
use crate::app::actions::Action;
use crate::app::state::ActivePane;
use crate::inputs::key::Key;
use crate::teams::app_handler::AppCmdEvent;

use crossterm::event::KeyEvent;
use log::*;
use tui_textarea::{Input, TextArea};

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
        if self.state.editing_mode {
            trace!("Keyevent: {:?}", key_event);
            self.process_editing_key(key_event).await
        } else {
            self.do_action(Key::from(key_event)).await
        }
    }

    /// Handle a user action (non-editing mode)
    async fn do_action(&mut self, key: crate::inputs::key::Key) -> AppReturn {
        if let Some(action) = self.state.actions.find(key) {
            debug!("Run action [{:?}]", action);
            match action {
                Action::DeleteMessage => {
                    if let Err(e) = self.delete_selected_message().await {
                        warn!("Could not delete message: {}", e);
                    };
                }
                Action::ComposeNewMessage => {
                    self.state.editing_mode = true;
                    self.state.set_active_pane(Some(ActivePane::Compose));
                }
                Action::MarkRead => {
                    self.state.mark_active_read();
                }
                Action::NextPane => {
                    self.state.cycle_active_pane();
                }
                Action::NextRoomFilter => {
                    self.next_filtering_mode().await;
                }
                Action::PreviousRoomFilter => {
                    self.previous_filtering_mode().await;
                }
                Action::Quit => return AppReturn::Exit,
                Action::SendMessage => {
                    self.send_message_buffer().await;
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
                    self.next_room().await;
                }
                Action::PreviousMessage => {
                    self.state.messages_list.select_previous_message();
                }
                Action::PreviousRoom => {
                    self.previous_room().await;
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
    async fn process_editing_key(&mut self, key_event: KeyEvent) -> AppReturn {
        let key: Key = key_event.into();
        match key {
            Key::Ctrl('c') => return AppReturn::Exit,
            Key::Esc => {
                self.state.editing_mode = false;
                self.state.set_active_pane(Some(ActivePane::Rooms))
            }
            Key::AltEnter => self.state.msg_input_textarea.insert_newline(),
            Key::Enter => {
                self.send_message_buffer().await;
            }
            _ => _ = self.state.msg_input_textarea.input(Input::from(key_event)),
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
    async fn send_message_buffer(&mut self) {
        if self.state.msg_input_textarea.is_empty() {
            warn!("An empty message cannot be sent.");
            return;
        };
        match self.state.active_room() {
            Some(room) => {
                let id = room.id.clone();
                let lines = self.state.msg_input_textarea.lines();
                let msg_to_send = webex::types::MessageOut {
                    room_id: Some(id.clone()),
                    text: Some(lines.join("\n")),
                    ..Default::default()
                };
                debug!("Sending message to room {:?}", room.title);
                self.dispatch_to_teams(AppCmdEvent::SendMessage(msg_to_send));
                self.state.msg_input_textarea = TextArea::default();
                self.state.teams_store.mark_read(&id);
            }
            None => warn!("Cannot send message, no room selected."),
        }
    }

    /// Deletes the selected message, if there is one and it was authored by self.
    /// Otherwise does nothing.
    async fn delete_selected_message(&mut self) -> Result<(), &'static str> {
        // Check if there is a selected room
        let room_id = match self.state.id_of_selected_room() {
            Some(id) => id,
            None => return Err("No room selected"),
        };

        // Get the messages in the selected room
        let messages = self.state.teams_store.messages_in_room_slice(&room_id);

        // Get the id of the selected message if the message is from the current user
        let msg_id_option = self
            .state
            .messages_list
            .selected_message(messages)
            .and_then(|msg| match self.state.teams_store.is_me(&msg.person_id) {
                false => None,
                true => msg.id.clone(),
            });

        // If a message id was found, dispatch a delete event and remove the message from the store
        match msg_id_option {
            Some(msg_id) => {
                self.dispatch_to_teams(AppCmdEvent::DeleteMessage(msg_id.clone()));
                self.state.teams_store.delete_message(&msg_id, &room_id);
                Ok(())
            }
            None => {
                Err("No message selected or the selected message was not from the current user")
            }
        }
    }

    /// Retrieves the latest messages in the room, only if it is empty
    async fn get_messages_if_room_empty(&mut self, id: &RoomId) {
        if self.state.teams_store.messages_in_room_slice(id).is_empty() {
            self.dispatch_to_teams(AppCmdEvent::ListMessagesInRoom(id.clone()));
        }
    }

    /// Send a command to the teams thread
    /// Does not block
    pub fn dispatch_to_teams(&mut self, action: AppCmdEvent) {
        if let Err(e) = self.app_to_teams_tx.send(action) {
            error!("Error from dispatch {}", e);
        };
    }

    /// Sets the active room to that highlighted by the list selection
    async fn set_active_room_to_selection(&mut self) {
        let id_option = self.state.id_of_selected_room();
        self.state.rooms_list.set_active_room_id(id_option.clone());
        // Changing active room may have affected the selection
        // e.g. with Unread filter which includes active room
        self.state.update_selection_with_active_room();
        if let Some(id) = id_option {
            self.get_messages_if_room_empty(&id).await;
        }
        // Update the number of messages in the active room
        self.state
            .messages_list
            .set_nb_messages(self.state.num_messages_active_room());
        // Deselect the message selection
        self.state.messages_list.deselect();
    }

    /// Change the rooms list filter to the previous one
    async fn previous_filtering_mode(&mut self) {
        self.state.rooms_list.set_active_room_id(None);
        self.state.rooms_list.previous_mode(&self.state.teams_store);
        self.set_active_room_to_selection().await;
    }

    /// Change the rooms list filter to the next one
    async fn next_filtering_mode(&mut self) {
        self.state.rooms_list.set_active_room_id(None);
        self.state.rooms_list.next_mode(&self.state.teams_store);
        self.set_active_room_to_selection().await;
    }

    /// Select the next room in the list
    async fn next_room(&mut self) {
        let num_rooms = self.state.num_of_visible_rooms();
        self.state.rooms_list.select_next_room(num_rooms);
        self.set_active_room_to_selection().await;
    }

    /// Select the previous room in the list
    async fn previous_room(&mut self) {
        let num_rooms = self.state.num_of_visible_rooms();
        self.state.rooms_list.select_previous_room(num_rooms);
        self.set_active_room_to_selection().await;
    }
}
