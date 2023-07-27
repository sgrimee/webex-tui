pub mod app_handler;
mod auth;
mod client;
pub mod store;
pub mod webex_handler;

use log::error;
use webex::{Event, Webex};

use self::client::get_webex_client;

pub struct Teams {
    client: Webex,
    // event_stream: WebexEventStream,
    rx: tokio::sync::mpsc::Receiver<Event>,
    _tx: tokio::sync::mpsc::Sender<Event>,
}

impl Teams {
    /// Get authenticated webex client and spawn thread to watch for events
    pub async fn new() -> Self {
        let client = get_webex_client().await;
        let mut event_stream = client.event_stream().await.expect("event stream");

        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let event_tx = tx.clone();

        tokio::spawn(async move {
            while let Ok(event) = event_stream.next().await {
                // debug!("Webex event in events thread: {:?}", event);
                // pass on the message, to be retrieved by calling the 'next' function
                if let Err(err) = event_tx.send(event).await {
                    error!("Oops!, {}", err);
                }
            }
        });

        Self {
            client,
            rx,
            _tx: tx,
        }
    }

    pub async fn next_event(&mut self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
}
