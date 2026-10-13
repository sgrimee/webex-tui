//! Neutral message composition panel.
use crate::app::state::{ActivePane, AppState};
use ratatui::{
    style::Style,
    text::Span,
    widgets::{Block, BorderType, Borders},
};
use tui_textarea::TextArea;
pub(crate) const MSG_INPUT_BLOCK_HEIGHT: u16 = 5;
/// Like the message list: the cached display name, then the sender email.
/// The account's own message says "your".
fn label(state: &AppState, message: &crate::frontend::data::MessageSnapshot) -> String {
    let sender = message.sender_id.as_ref().map(|id| id.as_str().to_owned());
    if sender.is_some() && state.cache.is_me(&sender) {
        return "your".into();
    }
    let name = sender
        .as_deref()
        .and_then(|id| state.cache.persons.get(id))
        .and_then(|person| person.display_name.clone());
    let who = name
        .or_else(|| message.sender_email.clone())
        .unwrap_or_else(|| "unknown sender".into());
    format!("{who}'s")
}
/// With the bridge, a reply/edit is sent to its message's own room even after
/// a room switch, so name that room when it is not the active one.
fn target_room_note(state: &AppState, message: &crate::frontend::data::MessageSnapshot) -> String {
    if state.bridge.is_none() {
        return String::new();
    }
    let room = message.room_id.as_str();
    if state.rooms_list.active_room_id().map(String::as_str) == Some(room) {
        return String::new();
    }
    let title = state
        .cache
        .room_and_team_title(&room.to_owned())
        .map(|title| title.room_title)
        .unwrap_or_else(|_| "another room".into());
    // Bridge room titles are sanitized when projected into the cache.
    format!(" (in {title})")
}

pub(crate) fn draw_message_editor<'a>(state: &'a AppState<'a>) -> TextArea<'a> {
    let hint = Span::styled(
        " Enter: send, Alt-Enter: new line, Esc: cancel.",
        Style::default().fg(state.theme.roles.hint()),
    );
    let title = if state.message_editor.is_composing() {
        if let Some(message) = state.message_editor.response_to() {
            vec![
                Span::styled(
                    format!(
                        "Responding to {} message{}.",
                        label(state, message),
                        target_room_note(state, message)
                    ),
                    Style::default().fg(state.theme.roles.compose_status()),
                ),
                hint,
            ]
        } else if let Some(message) = state.message_editor.editing_of() {
            vec![
                Span::styled(
                    format!(
                        "Editing {} message{}.",
                        label(state, message),
                        target_room_note(state, message)
                    ),
                    Style::default().fg(state.theme.roles.compose_status()),
                ),
                hint,
            ]
        } else {
            vec![
                Span::styled(
                    "Type your new message.",
                    Style::default().fg(state.theme.roles.compose_status()),
                ),
                hint,
            ]
        }
    } else {
        vec![Span::styled(
            "Press Enter with a selected room to type",
            Style::default(),
        )]
    };
    let border = match state.active_pane() {
        Some(ActivePane::Compose) => Style::default().fg(state.theme.roles.border_active()),
        _ => Style::default().fg(state.theme.roles.border()),
    };
    let mut textarea = state.message_editor.textarea().clone();
    textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .border_type(BorderType::Rounded)
            .title(title),
    );
    textarea.set_cursor_line_style(Style::default());
    textarea
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::data::{MessageId, MessageStatus, RoomId};
    #[test]
    fn missing_sender_has_neutral_label() {
        let m = crate::frontend::data::MessageSnapshot {
            id: MessageId::new("m"),
            room_id: RoomId::new("r"),
            revision: None,
            parent_id: None,
            sender_id: None,
            sender_email: None,
            created_at: None,
            updated_at: None,
            status: MessageStatus::Deleted,
            body: None,
            attachment_count: None,
        };
        assert_eq!(label(&AppState::default(), &m), "unknown sender's");
    }

    #[test]
    fn own_and_known_senders_are_named_like_the_message_list() {
        use crate::frontend::data::{PersonId, PersonSnapshot};
        let mut state = AppState::default();
        state.cache.set_me(PersonSnapshot {
            id: PersonId::new("me"),
            display_name: Some("Synthetic Me".into()),
            emails: None,
        });
        state.cache.persons.insert(PersonSnapshot {
            id: PersonId::new("other"),
            display_name: Some("Synthetic Other".into()),
            emails: None,
        });
        let message = |sender: &str| crate::frontend::data::MessageSnapshot {
            id: MessageId::new("m"),
            room_id: RoomId::new("r"),
            revision: Some(1),
            parent_id: None,
            sender_id: Some(PersonId::new(sender)),
            sender_email: Some("synthetic@example.invalid".into()),
            created_at: None,
            updated_at: None,
            status: MessageStatus::Active,
            body: None,
            attachment_count: None,
        };
        assert_eq!(label(&state, &message("me")), "your");
        assert_eq!(label(&state, &message("other")), "Synthetic Other's");
        assert_eq!(
            label(&state, &message("unknown")),
            "synthetic@example.invalid's"
        );
    }

    #[test]
    fn a_reply_target_in_another_room_is_named_only_with_the_bridge() {
        let mut state = AppState::default();
        let mut m = crate::frontend::data::MessageSnapshot {
            id: MessageId::new("m"),
            room_id: RoomId::new("other"),
            revision: Some(1),
            parent_id: None,
            sender_id: None,
            sender_email: None,
            created_at: None,
            updated_at: None,
            status: MessageStatus::Active,
            body: None,
            attachment_count: None,
        };
        state.rooms_list.set_active_room_id(Some("active".into()));
        assert_eq!(target_room_note(&state, &m), "", "legacy: no note");
        state.bridge = Some(crate::app::state::BridgeView::default());
        assert_eq!(target_room_note(&state, &m), " (in another room)");
        m.room_id = RoomId::new("active");
        assert_eq!(target_room_note(&state, &m), "");
    }
}
