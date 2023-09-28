// app/mod.rs

pub mod actions;
pub mod state;
pub mod teams_store;

use self::{
    actions::Actions,
    state::{AppState, RoomsListMode},
};
use crate::app::actions::Action;
use crate::inputs::key::Key;
use crate::teams::app_handler::AppCmdEvent;

use chrono::Duration;
use crossterm::event::KeyEvent;
use enum_iterator::next_cycle;
use log::*;
use ratatui_textarea::{Input, TextArea};
use webex::{Person, Room};

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
                    AppReturn::Continue
                }
                Action::MarkRead => {
                    if let Some(id) = self.state.selected_room_id() {
                        self.state.teams_store.mark_read(&id);
                    }
                    AppReturn::Continue
                }
                Action::NextRoomsListMode => {
                    if let Some(new_mode) = next_cycle(&self.state.room_list_mode) {
                        debug!("Rooms list mode set to {:?}", new_mode);
                        self.state.room_list_mode = new_mode;
                        let selected = if self.number_of_displayed_rooms() > 0 {
                            Some(0)
                        } else {
                            None
                        };
                        self.state.room_list_state.select(selected);
                    }
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
                    if let Some(selected) = self.state.room_list_state.selected() {
                        if selected >= self.number_of_displayed_rooms() - 1 {
                            self.state.room_list_state.select(Some(0));
                        } else {
                            self.state.room_list_state.select(Some(selected + 1));
                        }
                    }
                    AppReturn::Continue
                }
                Action::ArrowUp => {
                    if let Some(selected) = self.state.room_list_state.selected() {
                        if selected > 0 {
                            self.state.room_list_state.select(Some(selected - 1));
                        } else {
                            self.state
                                .room_list_state
                                .select(Some(self.number_of_displayed_rooms() - 1));
                        }
                    }
                    AppReturn::Continue
                }
            }
        } else {
            warn!("No action associated with {}", key);
            debug!(
                "If the key actually corresponds to an action, it needs to be added to the list
            of active actions too."
            );
            AppReturn::Continue
        }
    }
    // Handle a key while in text editing mode
    pub async fn process_editing_key(&mut self, key_event: KeyEvent) -> AppReturn {
        let key = Key::from(key_event);
        match key {
            Key::Ctrl('c') => return AppReturn::Exit,
            Key::Esc => self.state.editing_mode = false,
            Key::AltEnter => self.state.msg_input_textarea.insert_newline(),
            Key::Enter => self.send_message_buffer().await,
            _ => _ = self.state.msg_input_textarea.input(Input::from(key_event)),
        }
        AppReturn::Continue
    }

    pub async fn send_message_buffer(&mut self) {
        if self.state.msg_input_textarea.is_empty() {
            warn!("An empty message cannot be sent.");
            return;
        };
        match &self.state.selected_room_id() {
            Some(active_room) => {
                let lines = self.state.msg_input_textarea.lines();
                let msg_to_send = webex::types::MessageOut {
                    room_id: Some(active_room.clone()),
                    text: Some(lines.join("\n")),
                    ..Default::default()
                };
                debug!("Sending message: {:#?}", msg_to_send);
                self.dispatch_to_teams(AppCmdEvent::SendMessage(msg_to_send))
                    .await;
                self.state.msg_input_textarea = TextArea::default();
                self.state.teams_store.mark_read(active_room);
            }
            None => warn!("Cannot send message, no room selected."),
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
        self.dispatch_to_teams(AppCmdEvent::GetAllRooms()).await;
    }

    pub async fn set_state_rooms_loaded(&mut self) {
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

    // indicate the completion of a pending teams task
    pub fn loaded(&mut self) {
        self.state.is_loading = false;
    }

    pub fn message_sent(&mut self) {
        trace!("Message was sent.");
    }

    pub async fn message_received(&mut self, msg: webex::Message) {
        // update room details, including title, adding room if needed
        if let Some(id) = &msg.room_id {
            self.dispatch_to_teams(AppCmdEvent::UpdateRoom(id.to_owned()))
                .await;
        }
        // store the message for that room id
        self.state.teams_store.add_message(msg);
    }

    pub fn room_updated(&mut self, room: Room) {
        self.state.teams_store.update_room(room);
    }

    pub fn show_log_window(&self) -> bool {
        self.state.show_logs
    }

    pub fn rooms_for_list_mode(&self, mode: &RoomsListMode) -> Vec<&Room> {
        let rooms = self.state.teams_store.rooms();
        match mode {
            RoomsListMode::All => (rooms).collect(),
            RoomsListMode::Recent => rooms
                .filter(|&room| {
                    self.state
                        .teams_store
                        .room_has_activity_since(Duration::hours(2), &room.id)
                })
                .collect(),
            RoomsListMode::Unread => rooms
                .filter(|&room| self.state.teams_store.room_has_unread(&room.id))
                .collect(),
        }
    }

    pub fn number_of_displayed_rooms(&self) -> usize {
        self.rooms_for_list_mode(&self.state.room_list_mode).len()
    }
}
