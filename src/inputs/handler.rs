// inputs/handler.rs

//! Event handler that wraps crossterm input and tick event.

use log::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub enum Event {
    /// An input event occurred.
    // Input(Key),
    Input(crossterm::event::KeyEvent),
    /// An tick event occurred.
    Tick,
}

/// Event handler that wraps crossterm input and tick event.
/// Each event type is handled in its own thread and returned to a common `Receiver`
pub struct EventHandler {
    rx: tokio::sync::mpsc::Receiver<Event>,
    // Need to be kept around to prevent disposing the sender side.
    _tx: tokio::sync::mpsc::Sender<Event>,
    // To stop the loop
    stop_capture: Arc<AtomicBool>,
}

impl EventHandler {
    /// Constructs an new instance of `Events` with the default config
    /// and given `tick_rate`.
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let stop_capture = Arc::new(AtomicBool::new(false));

        let event_tx = tx.clone();
        let event_stop_capture = stop_capture.clone();
        tokio::spawn(async move {
            loop {
                // poll for tick rate duration, if no event, sent tick event.
                if crossterm::event::poll(tick_rate).unwrap() {
                    if let crossterm::event::Event::Key(key_event) =
                        crossterm::event::read().unwrap()
                    {
                        if let Err(err) = event_tx.send(Event::Input(key_event)).await {
                            error!("Could not send terminal event to main thread!, {}", err);
                        }
                    }
                }
                if let Err(err) = event_tx.send(Event::Tick).await {
                    error!("Could not send tick to main thread!, {}", err);
                }
                if event_stop_capture.load(Ordering::Relaxed) {
                    break;
                }
            }
        });

        EventHandler {
            rx,
            _tx: tx,
            stop_capture,
        }
    }

    /// Attempts to read an event.
    pub async fn next(&mut self) -> Event {
        self.rx.recv().await.unwrap_or(Event::Tick)
    }

    /// Close
    pub fn close(&mut self) {
        self.stop_capture.store(true, Ordering::Relaxed)
    }
}
