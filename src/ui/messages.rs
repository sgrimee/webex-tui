// ui/messages.rs

//! A panel displaying the messages in a `Room` as a table.

const CONTENT_INDENT_REPLY: &str = "  |   ";
const CONTENT_INDENT: &str = "  ";
const CONTENT_RIGHT_MARGIN: u16 = 2;
const TITLE_INDENT_REPLY: &str = "  | ";
const TITLE_INDENT: &str = "";
pub const ACTIVE_ROOM_MIN_WIDTH: u16 = 30;
pub const ROOM_MIN_HEIGHT: u16 = 8;

use crate::app::state::{ActivePane, AppState};
use ratatui::prelude::Rect;
use webex::Message;

use chrono::{DateTime, Local, Utc};
use ratatui::layout::Constraint;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::block::{Block, BorderType};
use ratatui::widgets::{Borders, Cell, Row, Table};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use textwrap::fill;

/// Assigns a color/style to each message sender, spreading over the palette
/// while ensuring each user always gets the same style for consistency.
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

/// Hash a string to a number in [0, upper[
fn hash_string_to_number(s: &str, upper: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    let hash = hasher.finish();
    hash % upper
}

/// Returns a human friendly view of the timestamp.
/// Panics if the timestamp cannot be parsed.
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

/// Returns a row with the formatted message and the number of lines.
fn row_for_message<'a>(msg: Message, width: u16) -> (Row<'a>, usize) {
    // Offset messages that are part of a conversation
    let (title_indent, content_indent) = match msg.parent_id {
        None => (TITLE_INDENT, CONTENT_INDENT),
        Some(_) => (TITLE_INDENT_REPLY, CONTENT_INDENT_REPLY),
    };

    // One line for the author and timestamp
    let mut title_line = Line::default();
    title_line.spans.push(title_indent.into());
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
    let options = textwrap::Options::new((width - CONTENT_RIGHT_MARGIN) as usize)
        .initial_indent(content_indent)
        .subsequent_indent(content_indent);
    let mut content = String::new();
    if msg.markdown.is_some() {
        // Detect markdown, but do not render it yet
        title_line.spans.push(Span::from("  MD"));
    }
    if let Some(raw_text) = msg.text {
        content = raw_text;
    }

    let mut text = Text::default();
    // One empty line, with a conversation marker if applicable
    text.extend(Text::from(format!("{content_indent}\n")));
    text.extend(Text::from(title_line));
    text.extend(Text::from(fill(&content, options)));

    // Indicate the presence of attachments
    if let Some(files) = msg.files {
        text.extend(Text::from(format!(
            "-- {} attachment{}",
            files.len(),
            if files.len() > 1 { "s" } else { "" }
        )));
    }

    let height = text.height();
    let cell = Cell::from(text);
    let row = Row::new(vec![cell]).height(height as u16);
    (row, height)
}

/// Draws a table containing the formatted messages for the active room.
/// Also returns the number or messages(rows) in the table and the number of text lines.
pub fn draw_msg_table<'a>(state: &AppState, rect: &Rect) -> (Table<'a>, usize, usize) {
    let mut title = "No selected room".to_string();
    let mut rows = Vec::<Row>::new();

    let mut nb_lines = 0;
    if let Some(room) = state.active_room() {
        title = room.title.clone().unwrap_or(String::from("Untitled room"));
        rows = state
            .teams_store
            .messages_in_room(&room.id)
            .map(|msg| {
                let (row, height) = row_for_message(msg.clone(), rect.width - 2);
                nb_lines += height;
                row
            })
            .collect();
    };
    let nb_rows = rows.len();

    // Highlight pane if active
    let border_style = match state.active_pane() {
        Some(ActivePane::Messages) => Style::default().fg(Color::Cyan),
        _ => Style::default(),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(title);

    (
        Table::new(rows)
            .block(block)
            .widths(&[Constraint::Percentage(100)])
            .column_spacing(1)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        nb_rows,
        nb_lines,
    )
}
