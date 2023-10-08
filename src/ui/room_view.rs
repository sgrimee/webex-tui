pub const ACTIVE_ROOM_MIN_WIDTH: u16 = 30;
pub const MSG_INPUT_BLOCK_HEIGHT: u16 = 5;
pub const ROOM_MIN_HEIGHT: u16 = 8;

use crate::app::state::AppState;
use crate::app::App;
use webex::Message;

use ratatui::layout::Constraint;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::block::{Block, BorderType};
use ratatui::widgets::{Borders, Row, Table};
use ratatui_textarea::TextArea;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn style_for_user(id: &Option<String>) -> Style {
    let colors = [
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
    ];
    match id {
        Some(id) => {
            let upper = colors.len() as u64;
            let index = hash_string_to_number(id, upper) as usize;
            Style::default().fg(colors[index])
        }
        None => Style::default().add_modifier(Modifier::REVERSED),
    }
}

// Hash a string to a number in [0, upper[
fn hash_string_to_number(s: &str, upper: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    let hash = hasher.finish();
    hash % upper
}

fn rows_for_message(msg: Message) -> Vec<Row<'static>> {
    let mut rows: Vec<Row> = Vec::new();
    if let Some(sender_email) = &msg.person_email {
        let row = Row::new(vec![Span::styled(
            sender_email.clone(),
            style_for_user(&msg.person_id),
        )]);
        rows.push(row);
    }
    if let Some(raw_text) = &msg.text {
        rows.push(Row::new(vec![Span::raw(raw_text.clone())]).height(3));
    }
    rows
}

pub fn draw_room_messages<'a>(app: &'a App) -> Table<'a> {
    let mut title = "No selected room".to_string();
    let mut rows = Vec::<Row>::new();
    if let Some(room) = &app.state.active_room() {
        title = room.title.clone();
        rows = app
            .state
            .teams_store
            .messages_in_room(&room.id)
            .flat_map(|msg| rows_for_message(msg.clone()))
            .collect();
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .title(title);

    Table::new(rows)
        .block(block)
        .widths(&[Constraint::Percentage(100)])
        .column_spacing(1)
        .highlight_style(
            Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
}

pub fn draw_msg_input<'a>(state: &'a AppState<'a>) -> TextArea<'a> {
    let (title, borders_style) = if state.editing_mode {
        (
            Span::styled(
                "Type your message, Enter to send, Alt+Enter for new line, Esc to exit.",
                Style::default().fg(Color::Yellow),
            ),
            Style::default().fg(Color::Yellow),
        )
    } else {
        (
            Span::styled("Press Enter with a selected room to type", Style::default()),
            Style::default(),
        )
    };
    let mut textarea = state.msg_input_textarea.clone();
    textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(borders_style)
            .title(title),
    );
    textarea.set_cursor_line_style(Style::default());
    textarea
}
