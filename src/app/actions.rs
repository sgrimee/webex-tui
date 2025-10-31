//! Actions the user can trigger on the main `App`.

use enum_iterator::{all, Sequence};
use std::collections::HashMap;
use std::fmt::{self, Display};

use crate::inputs::key::Key;

/// All possible user actions.
/// Not all actions are available in all contexts.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Sequence)]
pub(crate) enum Action {
    ComposeNewMessage,
    DeleteMessage,
    DumpRoomContentToFile,
    EditSelectedMessage,
    EndComposeMessage,
    LogExitPageMode,
    LogFocusSelectedTarget,
    LogIncreaseCapturedOneLevel,
    LogIncreaseShownOneLevel,
    LogPageDown,
    LogPageUp,
    LogReduceCapturedOneLevel,
    LogReduceShownOneLevel,
    LogSelectNextTarget,
    LogSelectPreviousTarget,
    LogToggleFilteredTargets,
    LogToggleTargetSelector,
    MarkRead,
    NextMessage,
    NextPane,
    NextRoom,
    NextRoomFilter,
    PreviousMessage,
    PreviousPane,
    PreviousRoom,
    PreviousRoomFilter,
    Quit,
    RespondMessage,
    SendMessage,
    StartRoomSearch,
    EndRoomSearch,
    ToggleDebug,
    ToggleHelp,
    ToggleLogs,
    ToggleRooms,
    ToggleRoomSelection,
    SelectAllVisibleRooms,
    ClearRoomSelections,
    DeleteSelectedRooms,
    UnselectMessage,
    CopyMessage,
}

impl Action {
    /// Return a slice with the key(s) associated to the action.
    pub(crate) fn keys(&self) -> &[Key] {
        match self {
            Action::DeleteMessage => &[Key::Char('d')],
            Action::DumpRoomContentToFile => &[Key::Char('D')],
            Action::ComposeNewMessage => &[Key::Enter],
            Action::EditSelectedMessage => &[Key::Char('e')],
            Action::EndComposeMessage => &[Key::Esc],
            Action::LogExitPageMode => &[Key::Esc],
            Action::LogFocusSelectedTarget => &[Key::Char('f')],
            Action::LogIncreaseCapturedOneLevel => &[Key::Char('+')],
            Action::LogIncreaseShownOneLevel => &[Key::Right],
            Action::LogPageDown => &[Key::PageDown],
            Action::LogPageUp => &[Key::PageUp],
            Action::LogReduceCapturedOneLevel => &[Key::Char('-')],
            Action::LogReduceShownOneLevel => &[Key::Left],
            Action::LogSelectNextTarget => &[Key::Down],
            Action::LogSelectPreviousTarget => &[Key::Up],
            Action::LogToggleFilteredTargets => &[Key::Char(' ')],
            Action::LogToggleTargetSelector => &[Key::Char('h')],
            Action::MarkRead => &[Key::Char('r')],
            Action::NextMessage => &[Key::Down],
            Action::NextPane => &[Key::Tab],
            Action::NextRoom => &[Key::Down],
            Action::NextRoomFilter => &[Key::Right],
            Action::PreviousMessage => &[Key::Up],
            Action::PreviousPane => &[Key::ShiftTab],
            Action::PreviousRoom => &[Key::Up],
            Action::PreviousRoomFilter => &[Key::Left],
            Action::Quit => &[Key::Ctrl('c'), Key::Char('q')],
            Action::RespondMessage => &[Key::Char('r')],
            Action::SendMessage => &[],
            Action::StartRoomSearch => &[Key::Char('/'), Key::Ctrl('f')],
            Action::EndRoomSearch => &[Key::Esc],
            Action::ToggleDebug => &[Key::Char('t')],
            Action::ToggleHelp => &[Key::Char('?')],
            Action::ToggleLogs => &[Key::Char('l')],
            Action::ToggleRooms => &[Key::Char('R')],
            Action::ToggleRoomSelection => &[Key::Char(' ')],
            Action::SelectAllVisibleRooms => &[Key::Char('A')],
            Action::ClearRoomSelections => &[Key::Char('C')],
            Action::DeleteSelectedRooms => &[Key::Char('X')],
            Action::UnselectMessage => &[Key::Esc],
            Action::CopyMessage => &[Key::Char('y')],
        }
    }
}

/// User friendly short description of the action
impl Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let str = match self {
            Action::DeleteMessage => "Delete selected",
            Action::DumpRoomContentToFile => "Dump room content to file",
            Action::ComposeNewMessage => "New message",
            Action::EditSelectedMessage => "Edit selected",
            Action::EndComposeMessage => "End editing",
            Action::LogExitPageMode => "Exit page mode",
            Action::LogFocusSelectedTarget => "Focus selected",
            Action::LogIncreaseCapturedOneLevel => "Increase captured",
            Action::LogIncreaseShownOneLevel => "Increase shown",
            Action::LogPageDown => "Page down",
            Action::LogPageUp => "Page up",
            Action::LogReduceCapturedOneLevel => "Reduce captured",
            Action::LogReduceShownOneLevel => "Reduce shown",
            Action::LogSelectNextTarget => "Select next",
            Action::LogSelectPreviousTarget => "Select previous",
            Action::LogToggleFilteredTargets => "Toggle filtered",
            Action::LogToggleTargetSelector => "Toggle selector",
            Action::MarkRead => "Mark read (locally)",
            Action::NextMessage => "Next message",
            Action::NextPane => "Next pane",
            Action::NextRoom => "Next room",
            Action::NextRoomFilter => "Next room filter",
            Action::PreviousMessage => "Previous message",
            Action::PreviousPane => "Previous pane",
            Action::PreviousRoom => "Previous room",
            Action::PreviousRoomFilter => "Previous room filter",
            Action::Quit => "Quit",
            Action::RespondMessage => "Respond to message",
            Action::SendMessage => "Send message",
            Action::StartRoomSearch => "Search rooms",
            Action::EndRoomSearch => "End search",
            Action::ToggleDebug => "Toggle debug",
            Action::ToggleHelp => "Toggle help",
            Action::ToggleLogs => "Toggle logs",
            Action::ToggleRooms => "Toggle rooms panel",
            Action::ToggleRoomSelection => "Toggle room selection",
            Action::SelectAllVisibleRooms => "Select all visible rooms",
            Action::ClearRoomSelections => "Clear room selections",
            Action::DeleteSelectedRooms => "Delete selected rooms",
            Action::UnselectMessage => "Unselect message",
            Action::CopyMessage => "Copy message",
        };
        write!(f, "{}", str)
    }
}

/// Vec of actions.
/// Can be used to enumerate the actions available in a
/// given context.
/// In a context, a key must map to at most one action.
#[derive(Default, Debug, Clone)]
pub(crate) struct Actions(Vec<Action>);

impl Actions {
    /// Given a key, find the corresponding action
    pub(crate) fn find(&self, key: Key) -> Option<Action> {
        all::<Action>()
            .filter(|action| self.0.contains(action))
            .find(|action| action.keys().contains(&key))
    }

    pub(crate) fn actions(&self) -> &[Action] {
        self.0.as_slice()
    }
}

impl From<Vec<Action>> for Actions {
    /// Builds contextual actions
    ///
    /// # Panics
    ///
    /// If two actions have same key
    fn from(actions: Vec<Action>) -> Self {
        // Check key unicity
        let mut map: HashMap<Key, Vec<Action>> = HashMap::new();
        for action in actions.iter() {
            for key in action.keys().iter() {
                match map.get_mut(key) {
                    Some(vec) => vec.push(*action),
                    None => {
                        map.insert(*key, vec![*action]);
                    }
                }
            }
        }
        let errors = map
            .iter()
            .filter(|(_, actions)| actions.len() > 1) // at least two actions share same shortcut
            .map(|(key, actions)| {
                let actions = actions
                    .iter()
                    .map(Action::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Conflict key {} with actions {}", key, actions)
            })
            .collect::<Vec<_>>();
        if !errors.is_empty() {
            panic!("{}", errors.join("; "))
        }

        // Ok, we can create contextual actions
        Self(actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_find_action_by_key() {
        let actions: Actions = vec![Action::Quit, Action::SendMessage].into();
        let result = actions.find(Key::Char('q'));
        assert_eq!(result, Some(Action::Quit));
    }

    #[test]
    fn should_find_action_by_key_not_found() {
        let actions: Actions = vec![Action::Quit, Action::SendMessage].into();
        let result = actions.find(Key::Alt('w'));
        assert_eq!(result, None);
    }

    #[test]
    fn should_create_actions_from_vec() {
        let _actions: Actions =
            vec![Action::Quit, Action::ComposeNewMessage, Action::SendMessage].into();
    }

    #[test]
    #[should_panic]
    fn should_panic_when_create_actions_conflict_key() {
        let _actions: Actions = vec![Action::Quit, Action::Quit].into();
    }
}
