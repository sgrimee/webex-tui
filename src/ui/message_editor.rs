// ui/message_editor.rs

//! Panel with a text editor used to type messages.

use ratatui::{
    style::Style,
    text::Span,
    widgets::{Block, BorderType, Borders},
};
use tui_textarea::TextArea;

use crate::app::state::{ActivePane, AppState};

pub(crate) const MSG_INPUT_BLOCK_HEIGHT: u16 = 5;

// Draws a text editor where the user can type a message.
pub(crate) fn draw_message_editor<'a>(state: &'a AppState<'a>) -> TextArea<'a> {
    // Update title when in editing mode
    let hint = Span::styled(
        " Enter: send, Alt-Enter: new line, Esc: cancel.",
        Style::default().fg(state.theme.roles.hint()),
    );
    let title = if state.message_editor.is_composing() {
        if let Some(orig_msg) = state.message_editor.response_to() {
            // Responding to a message
            vec![
                Span::styled(
                    format!(
                        "Responding to {0}'s message.",
                        orig_msg
                            .person_email
                            .clone()
                            .unwrap_or("unknown sender".to_string())
                    ),
                    Style::default().fg(state.theme.roles.compose_status()),
                ),
                hint,
            ]
        } else if let Some(orig_msg) = state.message_editor.editing_of() {
            // Editing a message
            vec![
                Span::styled(
                    format!(
                        "Editing {0}'s message.",
                        orig_msg
                            .person_email
                            .clone()
                            .unwrap_or("unknown sender".to_string())
                    ),
                    Style::default().fg(state.theme.roles.compose_status()),
                ),
                hint,
            ]
        } else {
            // Composing a new message
            vec![
                Span::styled(
                    "Type your new message.",
                    Style::default().fg(state.theme.roles.compose_status()),
                ),
                hint,
            ]
        }
    } else {
        // Not composing
        vec![Span::styled(
            "Press Enter with a selected room to type",
            Style::default(),
        )]
    };

    // Highlight pane if active
    let border_style = match state.active_pane() {
        Some(ActivePane::Compose) => Style::default().fg(state.theme.roles.border_active()),
        _ => Style::default().fg(state.theme.roles.border()),
    };

    let mut textarea = state.message_editor.textarea().clone();
    textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .border_type(BorderType::Rounded)
            .title(title),
    );
    textarea.set_cursor_line_style(Style::default());
    textarea
}
