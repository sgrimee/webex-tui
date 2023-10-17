pub const ACTIVE_ROOM_MIN_WIDTH: u16 = 30;
pub const MSG_INPUT_BLOCK_HEIGHT: u16 = 5;
pub const ROOM_MIN_HEIGHT: u16 = 8;
const MESSAGES_RIGHT_MARGIN: u16 = 1;
const MESSAGES_INDENT: &str = "  ";

use crate::app::state::AppState;
use crate::app::App;
use ratatui::prelude::Rect;
use webex::Message;

use chrono::{DateTime, Local, Utc};
use ratatui::layout::Constraint;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::block::{Block, BorderType};
use ratatui::widgets::{Borders, Cell, Row, Table};
use ratatui_textarea::TextArea;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use textwrap::fill;

// Assign a color/style to each message sender, spreading over the palette
// while ensuring each user always gets the same style for consistency
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

// Returns a human friendly view of the timestamp
// panics if the timestamp cannot be parsed
fn human_timestamp(datetime_str: &str) -> String {
    let datetime = DateTime::parse_from_rfc3339(datetime_str).unwrap();

    // Display more detail for further dates
    let now = Utc::now();
    let format = match now.signed_duration_since(datetime).num_days() {
        0 => "%H:%M",
        1..=7 => "%a, %H:%M",
        8..=365 => "%h %d, %H:%M",
        _ => "%v,%Y %H:%M",
    };

    let local_datetime = datetime.with_timezone(&Local);
    local_datetime.format(format).to_string()
}

// Return a row with the formatted message and the number of lines
fn row_for_message<'a>(msg: Message, width: u16) -> (Row<'a>, usize) {
    // One line for the author and timestamp
    let mut title_line = Line::default();
    if let Some(sender_email) = msg.person_email {
        title_line
            .spans
            .push(Span::styled(sender_email, style_for_user(&msg.person_id)));
    }
    // Add message timestamp
    title_line.spans.push(Span::from("  "));
    let mut stamp = String::new();
    if let Some(updated) = &msg.updated {
        stamp = format!("{} (edited)", human_timestamp(updated));
    } else if let Some(created) = &msg.created {
        stamp = human_timestamp(created);
    }
    title_line
        .spans
        .push(Span::styled(stamp, Style::new().gray()));

    // Message content
    let options = textwrap::Options::new((width - MESSAGES_RIGHT_MARGIN) as usize)
        .initial_indent(MESSAGES_INDENT)
        .subsequent_indent(MESSAGES_INDENT);
    let mut content = String::new();
    if msg.markdown.is_some() {
        // Detect markdown, but do not render it yet
        title_line.spans.push(Span::from("  MD"));
    }
    if let Some(raw_text) = msg.text {
        content = raw_text;
    }

    let mut text = Text::from(title_line);
    text.extend(Text::from(fill(&content, options)));
    text.extend(Text::from("\n"));

    let height = text.height();
    let cell = Cell::from(text);
    let row = Row::new(vec![cell]).height(height as u16);
    (row, height)
}

// Draw a table containing the formatted messages for the active room
// Also returns the number or rows in the table
pub fn draw_msg_table<'a>(app: &App, rect: &Rect) -> (Table<'a>, usize) {
    let mut title = "No selected room".to_string();
    let mut rows = Vec::<Row>::new();

    let mut _content_length = 0;
    if let Some(room) = app.state.active_room() {
        title = room.title.clone();
        rows = app
            .state
            .teams_store
            .messages_in_room(&room.id)
            .map(|msg| {
                let (row, height) = row_for_message(msg.clone(), rect.width - 2);
                _content_length += height;
                row
            })
            .collect();
    };
    let nb_rows = rows.len();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .title(title);

    (
        Table::new(rows)
            .block(block)
            .widths(&[Constraint::Percentage(100)])
            .column_spacing(1)
            .highlight_style(
                Style::default()
                    .bg(Color::Yellow)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            ),
        nb_rows,
    )
}

// Draw a text editor where the user can type a message
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
