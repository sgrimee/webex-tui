//! Neutral message composition panel.
use crate::app::state::{ActivePane, AppState};
use ratatui::{
    style::Style,
    text::Span,
    widgets::{Block, BorderType, Borders},
};
use tui_textarea::TextArea;
pub(crate) const MSG_INPUT_BLOCK_HEIGHT: u16 = 5;
fn label(message: &crate::frontend::data::MessageSnapshot) -> String {
    message
        .sender_email
        .clone()
        .unwrap_or_else(|| "unknown sender".into())
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
                    format!("Responding to {}'s message.", label(message)),
                    Style::default().fg(state.theme.roles.compose_status()),
                ),
                hint,
            ]
        } else if let Some(message) = state.message_editor.editing_of() {
            vec![
                Span::styled(
                    format!("Editing {}'s message.", label(message)),
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
        assert_eq!(label(&m), "unknown sender");
    }
}
