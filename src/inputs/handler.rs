// inputs/handler.rs

//! Event handler that wraps crossterm input and tick event.
//!
//! Terminal input is read on a dedicated OS thread (crossterm's `poll`/`read`
//! block), never on a tokio worker. A poll or read error ends that thread and
//! closes the channel, so `next` returns `None` and the UI quits instead of
//! freezing.

use log::*;
use ratatui::crossterm::event::{Event as CrosstermEvent, KeyEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub(crate) enum Event {
    /// An input event occurred.
    Input(KeyEvent),
    /// An tick event occurred.
    Tick,
}

/// Blocking terminal event source (crossterm in production).
pub(crate) trait TerminalSource: Send + 'static {
    fn poll(&mut self, timeout: Duration) -> std::io::Result<bool>;
    fn read(&mut self) -> std::io::Result<CrosstermEvent>;
}

pub(crate) struct Crossterm;

impl TerminalSource for Crossterm {
    fn poll(&mut self, timeout: Duration) -> std::io::Result<bool> {
        ratatui::crossterm::event::poll(timeout)
    }
    fn read(&mut self) -> std::io::Result<CrosstermEvent> {
        ratatui::crossterm::event::read()
    }
}

/// Event handler that wraps crossterm input and tick event, delivered to a
/// common `Receiver`.
pub(crate) struct EventHandler {
    rx: tokio::sync::mpsc::Receiver<Event>,
    // To stop the loop
    stop_capture: Arc<AtomicBool>,
}

impl EventHandler {
    /// Constructs an new instance of `Events` with the default config
    /// and given `tick_rate`.
    pub(crate) fn new(tick_rate: Duration) -> Self {
        Self::with_source(Crossterm, tick_rate)
    }

    pub(crate) fn with_source(mut source: impl TerminalSource, tick_rate: Duration) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let stop_capture = Arc::new(AtomicBool::new(false));
        let event_stop_capture = stop_capture.clone();
        // The only sender lives in this thread: when it ends, `next` sees the
        // channel closed.
        std::thread::spawn(move || loop {
            // poll for tick rate duration, if no event, sent tick event.
            match source.poll(tick_rate) {
                Ok(true) => match source.read() {
                    Ok(CrosstermEvent::Key(key_event)) => {
                        if tx.blocking_send(Event::Input(key_event)).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(err) => {
                        error!("Terminal input failed, closing the UI: {err}");
                        break;
                    }
                },
                Ok(false) => {}
                Err(err) => {
                    error!("Terminal input failed, closing the UI: {err}");
                    break;
                }
            }
            if tx.blocking_send(Event::Tick).is_err() {
                break;
            }
            if event_stop_capture.load(Ordering::Relaxed) {
                break;
            }
        });

        EventHandler { rx, stop_capture }
    }

    /// The next event; `None` once terminal input has failed or stopped.
    pub(crate) async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }

    /// Close
    pub(crate) fn close(&mut self) {
        self.stop_capture.store(true, Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    use std::collections::VecDeque;

    /// Scripted source: each entry answers one `poll` (and its `read`).
    struct Scripted(VecDeque<std::io::Result<Option<CrosstermEvent>>>);

    impl TerminalSource for Scripted {
        fn poll(&mut self, _: Duration) -> std::io::Result<bool> {
            match self.0.front() {
                Some(Ok(Some(_))) => Ok(true),
                Some(Ok(None)) => {
                    self.0.pop_front();
                    Ok(false)
                }
                Some(Err(_)) => Err(self.0.pop_front().unwrap().unwrap_err()),
                None => Err(std::io::Error::other("script exhausted")),
            }
        }
        fn read(&mut self) -> std::io::Result<CrosstermEvent> {
            Ok(self.0.pop_front().unwrap().unwrap().unwrap())
        }
    }

    fn key() -> CrosstermEvent {
        CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
    }

    #[tokio::test]
    async fn a_key_and_ticks_arrive_then_an_input_error_closes_the_stream() {
        let script = VecDeque::from([
            Ok(Some(key())),
            Ok(None),
            Err(std::io::Error::other("tty gone")),
        ]);
        let mut events = EventHandler::with_source(Scripted(script), Duration::ZERO);
        assert!(matches!(events.next().await, Some(Event::Input(_))));
        assert!(matches!(events.next().await, Some(Event::Tick)));
        assert!(matches!(events.next().await, Some(Event::Tick)));
        // poll failed: the reader thread ended and the channel closed.
        assert!(events.next().await.is_none());
    }

    #[tokio::test]
    async fn a_read_error_also_closes_the_stream() {
        struct ReadFails;
        impl TerminalSource for ReadFails {
            fn poll(&mut self, _: Duration) -> std::io::Result<bool> {
                Ok(true)
            }
            fn read(&mut self) -> std::io::Result<CrosstermEvent> {
                Err(std::io::Error::other("read failed"))
            }
        }
        let mut events = EventHandler::with_source(ReadFails, Duration::ZERO);
        assert!(events.next().await.is_none());
    }
}
