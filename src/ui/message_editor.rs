// ui/message_editor.rs

//! Panel with a text editor used to type messages.

use ratatui::{
    style::{Color, Style},
    text::Span,
    widgets::{Block, BorderType, Borders},
};
use tui_textarea::TextArea;

use crate::app::state::{ActivePane, AppState};

pub const MSG_INPUT_BLOCK_HEIGHT: u16 = 5;

// Draws a text editor where the user can type a message.
pub fn draw_message_editor<'a>(state: &'a AppState<'a>) -> TextArea<'a> {
    // Update title when in editing mode
    let hint = Span::styled(
        " Enter: send, Ctrl-Enter: new line, Esc: cancel.",
        Style::default().fg(Color::Gray),
    );
    let title = if state.message_editor.is_editing() {
        match state.message_editor.respondee() {
            Some(respondee) => vec![
                Span::styled(
                    format!("Responding to {0}'s message.", respondee.author),
                    Style::default().fg(Color::Yellow),
                ),
                hint,
            ],
            None => vec![
                Span::styled("Type your new message.", Style::default().fg(Color::Yellow)),
                hint,
            ],
        }
    } else {
        vec![Span::styled(
            "Press Enter with a selected room to type",
            Style::default(),
        )]
    };

    // Highlight pane if active
    let border_style = match state.active_pane() {
        Some(ActivePane::Compose) => Style::default().fg(Color::Cyan),
        _ => Style::default(),
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
