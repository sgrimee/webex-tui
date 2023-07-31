pub mod actions;
pub mod state;
pub mod teams_store;
pub mod ui;

use self::{actions::Actions, state::AppState};
use crate::app::actions::Action;

use crate::inputs::key::Key;
use crate::inputs::patch::input_from_key_event;
use crate::teams::app_handler::AppCmdEvent;
use crossterm::event::KeyEvent;
use log::{debug, error, warn};

use tui_textarea::TextArea;
use webex::Person;

#[derive(Debug, PartialEq, Eq)]
pub enum AppReturn {
    Exit,
    Continue,
}

pub struct App<'a> {
    app_to_teams_tx: tokio::sync::mpsc::Sender<AppCmdEvent>,
    state: AppState<'a>,
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
                Action::SendMessage => {
                    self.send_message_buffer().await;
                    AppReturn::Continue
                }
                Action::ToggleLogs => {
                    self.state.show_logs = !self.state.show_logs;
                    AppReturn::Continue
                }
            }
        } else {
            warn!("No action accociated to {}", key);
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
            _ => {
                _ = self
                    .state
                    .msg_input_textarea
                    .input(input_from_key_event(key_event))
            }
        }
        AppReturn::Continue
    }

    pub async fn send_message_buffer(&mut self) {
        if self.state.msg_input_textarea.is_empty() {
            debug!("Won't send and empty message");
            return;
        };
        match &self.state.active_room {
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

    pub fn initialized(&mut self) {
        // Update contextual actions
        self.state.actions = vec![
            Action::Quit,
            Action::EditMessage,
            Action::SendMessage,
            Action::ToggleLogs,
        ]
        .into();
        self.state.active_room = Some(
            "Y2lzY29zcGFyazovL3VzL1JPT00vOTA1ZjJjOTAtMjdiZS0xMWVlLWJlY2YtMzNhZGYyOWQzODFj"
                .to_string(), // bla
                              // "Y2lzY29zcGFyazovL3VzL1JPT00vYmY4Mzk3NjYtY2NkMy0zMDdhLWFmMzctNWJhYWRjODNkNmQ3", // Raph
        );
    }

    // indicate the completion of a pending teams task
    pub fn loaded(&mut self) {
        self.state.is_loading = false;
    }

    pub fn message_sent(&mut self) {
        debug!("Message was sent.");
    }

    pub fn message_received(&mut self, msg: webex::Message) {
        self.state.teams_store.add_message(msg)
    }

    pub fn show_log_window(&self) -> bool {
        self.state.show_logs
    }
}
