// ui/editor.rs

//! Panel with a text editor used to type messages.

use ratatui::{
    style::{Color, Style},
    text::Span,
    widgets::{Block, Borders},
};
use tui_textarea::TextArea;

use crate::app::state::{ActivePane, AppState};

pub const MSG_INPUT_BLOCK_HEIGHT: u16 = 5;

// Draws a text editor where the user can type a message.
pub fn draw_msg_input<'a>(state: &'a AppState<'a>) -> TextArea<'a> {
    // Update title when in editing mode
    let title = if state.editing_mode {
        Span::styled(
            "Type your message, Enter to send, Alt+Enter for new line, Esc to exit.",
            Style::default().fg(Color::Yellow),
        )
    } else {
        Span::styled("Press Enter with a selected room to type", Style::default())
    };

    // Highlight pane if active
    let border_style = match state.active_pane() {
        Some(ActivePane::Compose) => Style::default().fg(Color::Cyan),
        _ => Style::default(),
    };

    let mut textarea = state.msg_input_textarea.clone();
    textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title),
    );
    textarea.set_cursor_line_style(Style::default());
    textarea
}
