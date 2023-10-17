// app/mod.rs

pub mod actions;
pub mod messages_list;
pub mod rooms_list;
pub mod state;
pub mod teams_store;

use self::{actions::Actions, state::AppState, teams_store::RoomId};
use crate::app::actions::Action;
use crate::inputs::key::Key;
use crate::teams::app_handler::AppCmdEvent;

use crossterm::event::KeyEvent;
use log::*;
use ratatui_textarea::{Input, TextArea};
use std::collections::HashSet;
use webex::{Message, Person, Room};

#[derive(Debug, PartialEq, Eq)]
pub enum AppReturn {
    Exit,
    Continue,
}

pub struct App<'a> {
    app_to_teams_tx: tokio::sync::mpsc::Sender<AppCmdEvent>,
    pub state: AppState<'a>,
}

impl App<'_> {
    pub fn new(app_to_teams_tx: tokio::sync::mpsc::Sender<AppCmdEvent>) -> Self {
        Self {
            app_to_teams_tx,
            state: AppState::default(),
        }
    }

    pub fn is_editing(&mut self) -> bool {
        self.state.editing_mode
    }

    pub fn set_me_user(&mut self, me: Person) {
        self.state.teams_store.set_me_user(me);
    }

    /// Handle a user action (non-editing mode)
    pub async fn do_action(&mut self, key: crate::inputs::key::Key) -> AppReturn {
        if let Some(action) = self.state.actions.find(key) {
            debug!("Run action [{:?}]", action);
            match action {
                Action::Quit => AppReturn::Exit,
                Action::EditMessage => {
                    self.state.editing_mode = true;
                    self.set_state_message_writing();
                    AppReturn::Continue
                }
                Action::MarkRead => {
                    self.state.mark_active_read();
                    AppReturn::Continue
                }
                Action::NextRoomsListMode => {
                    self.next_filtering_mode().await;
                    AppReturn::Continue
                }
                Action::SendMessage => {
                    self.send_message_buffer().await;
                    AppReturn::Continue
                }
                Action::ToggleLogs => {
                    self.state.show_logs = !self.state.show_logs;
                    AppReturn::Continue
                }
                Action::ToggleHelp => {
                    self.state.show_help = !self.state.show_help;
                    AppReturn::Continue
                }
                Action::ArrowDown => {
                    self.next_room().await;
                    AppReturn::Continue
                }
                Action::ArrowUp => {
                    self.previous_room().await;
                    AppReturn::Continue
                }
                _ => {
                    warn!("Unsupported action {} in this context", action);
                    AppReturn::Continue
                }
            }
        } else {
            warn!("No action associated with {} in this mode", key);
            // If the key actually corresponds to an action, it needs to be added to the list of active actions too.
            AppReturn::Continue
        }
    }
    // Handle a key while in text editing mode
    pub async fn process_editing_key(&mut self, key_event: KeyEvent) -> AppReturn {
        let key = Key::from(key_event);
        match key {
            Key::Ctrl('c') => return AppReturn::Exit,
            Key::Esc => {
                self.state.editing_mode = false;
                self.set_state_room_selection();
            }
            Key::AltEnter => self.state.msg_input_textarea.insert_newline(),
            Key::Enter => {
                self.send_message_buffer().await;
            }
            _ => _ = self.state.msg_input_textarea.input(Input::from(key_event)),
        }
        AppReturn::Continue
    }

    pub async fn send_message_buffer(&mut self) {
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
                self.dispatch_to_teams(AppCmdEvent::SendMessage(msg_to_send))
                    .await;
                self.state.msg_input_textarea = TextArea::default();
                self.state.teams_store.mark_read(&id);
            }
            None => warn!("Cannot send message, no room selected."),
        }
    }

    pub async fn get_messages_if_room_empty(&mut self, id: &RoomId) {
        if self.state.teams_store.messages_in_room(id).next().is_none() {
            self.dispatch_to_teams(AppCmdEvent::ListMessagesInRoom(id.clone()))
                .await;
        }
    }

    /// We could update the app or dispatch event on tick
    pub async fn update_on_tick(&mut self) -> AppReturn {
        AppReturn::Continue
    }

    /// Send a command to the teams thread
    pub async fn dispatch_to_teams(&mut self, action: AppCmdEvent) {
        // `is_loading` will be set to false again after the async action has finished in io/handler.rs
        self.state.is_loading = true;
        if let Err(e) = self.app_to_teams_tx.send(action).await {
            self.state.is_loading = false;
            error!("Error from dispatch {}", e);
        };
    }

    pub fn actions(&self) -> &Actions {
        &self.state.actions
    }

    pub fn is_loading(&self) -> bool {
        self.state.is_loading
    }

    pub async fn set_state_initialized(&mut self) {
        self.state.actions = vec![Action::Quit, Action::ToggleHelp, Action::ToggleLogs].into();
        // Some more heavy tasks that we put after init to ensure quick startup
        self.dispatch_to_teams(AppCmdEvent::ListAllRooms()).await;
    }

    pub fn set_state_room_selection(&mut self) {
        self.state.actions = vec![
            Action::ArrowDown,
            Action::ArrowUp,
            Action::EditMessage,
            Action::MarkRead,
            Action::NextRoomsListMode,
            Action::Quit,
            Action::SendMessage,
            Action::ToggleHelp,
            Action::ToggleLogs,
        ]
        .into();
    }

    pub async fn set_active_room_to_selection(&mut self) {
        let id_option = self.state.id_of_selected_room();
        self.state.set_active_room_id(&id_option);
        // Changing active room may have affected the selection
        // e.g. with Unread filter which includes active room
        self.state.update_selection_with_active_room();
        if let Some(id) = id_option {
            self.get_messages_if_room_empty(&id).await;
        }
    }

    pub async fn next_filtering_mode(&mut self) {
        self.state.set_active_room_id(&None);
        self.state.rooms_list.next_mode(&self.state.teams_store);
        self.set_active_room_to_selection().await;
    }

    pub async fn next_room(&mut self) {
        let num_rooms = self.state.num_of_visible_rooms();
        self.state.rooms_list.select_next_room(num_rooms);
        self.set_active_room_to_selection().await;
    }

    pub async fn previous_room(&mut self) {
        let num_rooms = self.state.num_of_visible_rooms();
        self.state.rooms_list.select_previous_room(num_rooms);
        self.set_active_room_to_selection().await;
    }

    pub fn set_state_message_writing(&mut self) {
        self.state.actions = vec![Action::SendMessage, Action::EndEditMessage].into();
    }

    // indicate the completion of a pending teams task
    pub fn loaded(&mut self) {
        self.state.is_loading = false;
    }

    pub fn message_sent(&mut self) {
        trace!("Message was sent.");
    }

    pub async fn message_received(&mut self, msg: &Message, mark_unread: bool) {
        let messages: [Message; 1] = [msg.clone()];
        self.messages_received(&messages, mark_unread).await
    }

    pub async fn messages_received(&mut self, messages: &[Message], mark_unread: bool) {
        // keep track of rooms we add messages to
        let mut room_ids = HashSet::new();
        // messages came in with most recent first, so reverse them
        for msg in messages.iter().rev() {
            if let Some(id) = &msg.room_id {
                room_ids.insert(id);
                if mark_unread && !self.state.teams_store.is_me(&msg.person_id) {
                    self.state.teams_store.mark_unread(id);
                }
            }
            self.state.teams_store.add_message(msg);
        }
        // update room details, including title, adding room if needed
        for room_id in room_ids {
            self.dispatch_to_teams(AppCmdEvent::UpdateRoom(room_id.to_owned()))
                .await;
        }
    }

    pub fn room_updated(&mut self, room: Room) {
        self.state.teams_store.update_room(room);
        self.state.update_selection_with_active_room();
    }

    pub fn show_log_window(&self) -> bool {
        self.state.show_logs
    }
}
