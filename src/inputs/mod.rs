// use self::key::Key;

pub mod events;
pub mod key;
pub mod patch;

pub enum InputEvent {
    /// An input event occurred.
    // Input(Key),
    Input(crossterm::event::KeyEvent),
    /// An tick event occurred.
    Tick,
}
