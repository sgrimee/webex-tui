use enum_iterator::{all, Sequence};
use std::collections::HashMap;
use std::fmt::{self, Display};

use crate::inputs::key::Key;

/// We define all available user actions
#[derive(Debug, Clone, Copy, Eq, PartialEq, Sequence)]
pub enum Action {
    ArrowDown,
    ArrowUp,
    EditMessage,
    EndEditMessage,
    MarkRead,
    NextRoomsListMode,
    PreviousRoomsListMode,
    Quit,
    SendMessage,
    ToggleHelp,
    ToggleLogs,
}

impl Action {
    /// List of key associated to action
    pub fn keys(&self) -> &[Key] {
        match self {
            Action::ArrowDown => &[Key::Down],
            Action::ArrowUp => &[Key::Up],
            Action::EditMessage => &[Key::Enter],
            Action::EndEditMessage => &[Key::Esc],
            Action::MarkRead => &[Key::Char('r')],
            Action::NextRoomsListMode => &[Key::Right],
            Action::PreviousRoomsListMode => &[Key::Left],
            Action::Quit => &[Key::Ctrl('c'), Key::Char('q')],
            Action::SendMessage => &[],
            Action::ToggleHelp => &[Key::Char('h')],
            Action::ToggleLogs => &[Key::Char('l')],
        }
    }
}

/// User friendly short description of action
impl Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let str = match self {
            Action::ArrowDown => "Next room",
            Action::ArrowUp => "Previous room",
            Action::EditMessage => "Edit message",
            Action::EndEditMessage => "End editing message",
            Action::MarkRead => "Mark read (locally)",
            Action::NextRoomsListMode => "Next room filter",
            Action::PreviousRoomsListMode => "Previous room filter",
            Action::Quit => "Quit",
            Action::SendMessage => "Send message",
            Action::ToggleHelp => "Toggle help",
            Action::ToggleLogs => "Toggle logs",
        };
        write!(f, "{}", str)
    }
}

#[derive(Default, Debug, Clone)]
pub struct Actions(Vec<Action>);

impl Actions {
    /// Given a key, find the corresponding action
    pub fn find(&self, key: Key) -> Option<Action> {
        all::<Action>()
            .filter(|action| self.0.contains(action))
            .find(|action| action.keys().contains(&key))
    }

    pub fn actions(&self) -> &[Action] {
        self.0.as_slice()
    }
}

impl From<Vec<Action>> for Actions {
    /// Build contextual action
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
        let _actions: Actions = vec![Action::Quit, Action::EditMessage, Action::SendMessage].into();
    }

    #[test]
    #[should_panic]
    fn should_panic_when_create_actions_conflict_key() {
        let _actions: Actions = vec![Action::Quit, Action::Quit].into();
    }
}
