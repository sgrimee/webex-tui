use self::actions::Actions;
use self::state::AppState;
use crate::app::actions::Action;
use crate::inputs::key::Key;
use crate::inputs::patch::input_from_key_event;
use crate::teams::IoEvent;
use crossterm::event::KeyEvent;
use log::{debug, error, warn};
use tui_textarea::TextArea;
// use webex::Webex;

pub mod actions;
pub mod state;
pub mod ui;

#[derive(Debug, PartialEq, Eq)]
pub enum AppReturn {
    Exit,
    Continue,
}

/// The main application, containing the state
pub struct App<'a> {
    /// We could dispatch an IO event
    io_tx: tokio::sync::mpsc::Sender<IoEvent>,
    /// Contextual actions
    actions: Actions,
    /// State
    is_loading: bool,
    state: AppState,
    msg_input_textarea: TextArea<'a>,
}

impl App<'_> {
    pub fn new(io_tx: tokio::sync::mpsc::Sender<IoEvent>) -> Self {
        let actions = vec![Action::Quit].into();
        let is_loading = false;
        let state = AppState::default();
        let msg_input_textarea = TextArea::default();

        Self {
            io_tx,
            actions,
            is_loading,
            state,
            msg_input_textarea,
        }
    }

    /// Handle a user action (non-editing mode)
    pub async fn do_action(&mut self, key: crate::inputs::key::Key) -> AppReturn {
        if let Some(action) = self.actions.find(key) {
            debug!("Run action [{:?}]", action);
            match action {
                Action::Quit => AppReturn::Exit,
                // IncrementDelay and DecrementDelay is handled in the UI thread
                Action::IncrementDelay => {
                    self.state.increment_delay();
                    AppReturn::Continue
                }
                // Note, that we clamp the duration, so we stay >= 0
                Action::DecrementDelay => {
                    self.state.decrement_delay();
                    AppReturn::Continue
                }
                Action::EditMessage => {
                    self.state.set_editing(true);
                    AppReturn::Continue
                }
                Action::SendMessage => {
                    self.send_message_buffer().await;
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
            Key::Esc => self.state.set_editing(false),
            Key::AltEnter => self.send_message_buffer().await,
            _ => {
                _ = self
                    .msg_input_textarea
                    .input(input_from_key_event(key_event))
            }
        }
        AppReturn::Continue
    }

    pub async fn send_message_buffer(&mut self) {
        self.state.set_editing(false);
        let lines = self.msg_input_textarea.lines();
        let msg_to_send = webex::types::MessageOut {
            // to_person_email: Some("rawouter@cisco.com".to_string()),
            room_id: Some(
                "Y2lzY29zcGFyazovL3VzL1JPT00vOTA1ZjJjOTAtMjdiZS0xMWVlLWJlY2YtMzNhZGYyOWQzODFj"
                    .to_string(),
            ),
            text: Some(lines.join("\n")),
            ..Default::default()
        };
        debug!("Sending message: {:#?}", msg_to_send);
        self.dispatch(IoEvent::SendMessage(msg_to_send)).await;
        self.msg_input_textarea = TextArea::default();
    }

    /// We could update the app or dispatch event on tick
    pub async fn update_on_tick(&mut self) -> AppReturn {
        // here we just increment a counter
        self.state.incr_tick();
        AppReturn::Continue
    }

    /// Send a network event to the IO thread
    pub async fn dispatch(&mut self, action: IoEvent) {
        // `is_loading` will be set to false again after the async action has finished in io/handler.rs
        self.is_loading = true;
        if let Err(e) = self.io_tx.send(action).await {
            self.is_loading = false;
            error!("Error from dispatch {}", e);
        };
    }

    pub fn actions(&self) -> &Actions {
        &self.actions
    }
    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn is_loading(&self) -> bool {
        self.is_loading
    }

    pub fn initialized(&mut self) {
        // Update contextual actions
        self.actions = vec![
            Action::Quit,
            Action::IncrementDelay,
            Action::DecrementDelay,
            Action::EditMessage,
            Action::SendMessage,
        ]
        .into();
        self.state = AppState::initialized()
    }

    // indicate the completion of a pending IO(thread) task
    pub fn loaded(&mut self) {
        self.is_loading = false;
    }

    pub fn message_sent(&mut self) {
        debug!("Message was sent.");
    }
}
