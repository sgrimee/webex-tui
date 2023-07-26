pub mod handler;
mod webex_client;
use webex::types::MessageOut;

// For this dummy application we only need two IO event
#[derive(Debug, Clone)]
pub enum IoEvent {
    Initialize, // Launch to initialize the application
    SendMessage(MessageOut),
}
