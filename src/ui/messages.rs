// ui/messages.rs

//! A panel displaying the messages in a `Room` as a table.

const CONTENT_INDENT_REPLY: &str = "  |   ";
const CONTENT_INDENT: &str = "  ";
const CONTENT_RIGHT_MARGIN: u16 = 2;
const TITLE_INDENT_REPLY: &str = "  | ";
const TITLE_INDENT: &str = "";
pub(crate) const ACTIVE_ROOM_MIN_WIDTH: u16 = 30;
pub(crate) const ROOM_MIN_HEIGHT: u16 = 8;

use crate::app::state::{ActivePane, AppState};
use crate::frontend::models::{DisplayBody, MessagePresentation};
use base64::Engine;
use html2text::from_read;
use ratatui::prelude::Rect;

use chrono::{DateTime, Local, Utc};
use ratatui::layout::Constraint;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::block::{Block, BorderType};
use ratatui::widgets::{Borders, Cell, Row, Table};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use textwrap::fill;

use super::style::line_for_room_and_team_title;

/// Assigns a color/style to each message sender, spreading over the palette
/// while ensuring each user always gets the same style for consistency.
fn style_for_user(id: Option<&str>, user_colors: &[Color]) -> Style {
    match id {
        Some(id) => {
            let upper = user_colors.len() as u64;
            let index = hash_string_to_number(id, upper) as usize;
            Style::default().fg(user_colors[index])
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

fn trim_newline(s: &mut String) {
    if s.ends_with('\n') {
        s.pop();
        if s.ends_with('\r') {
            s.pop();
        }
    }
}

/// Returns a row with the formatted message and the number of lines.
fn row_for_message<'a>(
    state: &AppState,
    msg: MessagePresentation<'_>,
    width: u16,
) -> (Row<'a>, usize) {
    // Offset messages that are part of a conversation
    let (title_indent, content_indent) = match msg.parent_id {
        None => (TITLE_INDENT, CONTENT_INDENT),
        Some(_) => (TITLE_INDENT_REPLY, CONTENT_INDENT_REPLY),
    };

    // One line for the author and timestamp
    let mut title_line = Line::default();
    title_line.spans.push(title_indent.into());

    // Prefer the cached display name, then the projected sender email.
    let person_opt = msg.sender_id.and_then(|id| state.cache.persons.get(id));
    let sender = match (person_opt, msg.sender_email) {
        (Some(person), _) => person.display_name.clone(),
        (None, Some(email)) => email.to_owned(),
        _ => String::from("Unknown"),
    };
    title_line.spans.push(Span::styled(
        sender,
        style_for_user(msg.sender_id, &state.theme.user_colors()),
    ));

    // Add message timestamp
    title_line.spans.push(Span::from("  "));
    let mut stamp = String::new();
    if let Some(updated) = msg.updated_at {
        stamp = format!("{} (edited)", human_timestamp(updated));
    } else if let Some(created) = msg.created_at {
        stamp = human_timestamp(created);
    }
    title_line.spans.push(Span::styled(
        stamp,
        Style::default().fg(state.theme.roles.msg_timestamp()),
    ));

    // Add message id
    if state.debug {
        add_uuid_to_line(msg.id, &mut title_line);
    }

    // Message content
    let text_width = (width - CONTENT_RIGHT_MARGIN) as usize;
    let options = textwrap::Options::new(text_width)
        .initial_indent(content_indent)
        .subsequent_indent(content_indent);
    let mut content = match msg.body.display() {
        DisplayBody::Missing => String::from("No content"),
        DisplayBody::Html(html) => {
            if state.debug {
                title_line.spans.push(Span::from(" (HTML)"));
            }
            from_read(html.as_bytes(), text_width)
                .unwrap_or_else(|_| String::from("Failed to parse HTML"))
        }
        DisplayBody::Markdown(markdown) => {
            if state.debug {
                title_line.spans.push(Span::from("  (MD)"));
            }
            markdown.to_owned()
        }
        DisplayBody::Text(text) => text.to_owned(),
    };
    trim_newline(&mut content);

    let mut text = Text::default();
    // One empty line, with a conversation marker if applicable
    text.extend(Text::from(format!("{content_indent}\n")));
    text.extend(Text::from(title_line));
    text.extend(Text::from(fill(&content, options)));

    // Indicate the presence of attachments, without exposing attachment URLs.
    if let Some(count) = msg.attachment_count {
        text.extend(Text::from(format!(
            "-- {} attachment{}",
            count,
            if count > 1 { "s" } else { "" }
        )));
    }

    let height = text.height();
    let cell = Cell::from(text);
    let row = Row::new(vec![cell]).height(height as u16);
    (row, height)
}

/// Adds the decoded message uuid to the line.
fn add_uuid_to_line(id: Option<&str>, line: &mut Line<'_>) {
    if let Some(id) = id
        .and_then(|id| {
            base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(id)
                .ok()
        })
        .and_then(|dec| String::from_utf8(dec).ok())
        .map(|id| id.split('/').nth(4).unwrap().to_string())
    {
        line.spans.push(Span::styled(
            format!(" [{id}]"),
            Style::default().fg(Color::DarkGray),
        ))
    }
}

/// Draws a table containing the formatted messages for the active room.
/// Also returns the number or messages(rows) in the table and the number of text lines.
pub(crate) fn draw_msg_table<'a>(state: &AppState, rect: &Rect) -> (Table<'a>, usize, usize) {
    let mut title_line = Line::from("No selected room");
    let mut rows = Vec::<Row>::new();

    let mut nb_lines = 0;
    if let Some(room) = state.active_room() {
        // get the formatted title for the room
        let ratt = state
            .cache
            .room_and_team_title(&room.id)
            .unwrap_or_default();
        title_line = line_for_room_and_team_title(
            ratt,
            room.unread,
            state.theme.roles.room_unread(),
            state.theme.roles.room_team(),
        );

        // add the room id to the title if debug is enabled
        if state.debug {
            add_uuid_to_line(Some(room.id.as_str()), &mut title_line);
        }

        // get the formatted messages for the room
        rows = state
            .cache
            .messages_in_room(&room.id)
            .map(|msg| {
                let (row, height) =
                    row_for_message(state, MessagePresentation::from(msg), rect.width - 2);
                nb_lines += height;
                row
            })
            .collect();
    };
    let nb_rows = rows.len();

    // Highlight pane if active
    let border_style = match state.active_pane() {
        Some(ActivePane::Messages) => Style::default().fg(state.theme.roles.border_active()),
        _ => Style::default().fg(state.theme.roles.border()),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(title_line);

    (
        Table::new(rows, &[Constraint::Percentage(100)])
            .block(block)
            .column_spacing(1)
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        nb_rows,
        nb_lines,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::models::MessageBody;
    use ratatui::{buffer::Buffer, widgets::Widget};

    fn rendered_row(message: MessagePresentation<'_>) -> String {
        let state = AppState::default();
        let (row, height) = row_for_message(&state, message, 60);
        let area = Rect::new(0, 0, 60, height as u16);
        let mut buffer = Buffer::empty(area);
        Table::new([row], [Constraint::Percentage(100)]).render(area, &mut buffer);
        buffer.content.iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn neutral_row_renders_text_sender_and_attachment_count() {
        let text = rendered_row(MessagePresentation {
            sender_email: Some("synthetic@example.invalid"),
            body: MessageBody {
                text: Some("Plain message"),
                ..Default::default()
            },
            attachment_count: Some(2),
            ..Default::default()
        });
        assert!(text.contains("synthetic@example.invalid"));
        assert!(text.contains("Plain message"));
        assert!(text.contains("-- 2 attachments"));
    }

    #[test]
    fn neutral_row_preserves_html_display_precedence_and_reply_indent() {
        let text = rendered_row(MessagePresentation {
            parent_id: Some("parent-a"),
            body: MessageBody {
                text: Some("alternate plain body"),
                markdown: Some("alternate Markdown body"),
                html: Some("<p>Displayed HTML body</p>"),
            },
            ..Default::default()
        });
        assert!(text.contains("Displayed HTML body"));
        assert!(text.contains(CONTENT_INDENT_REPLY));
        assert!(!text.contains("<p>"));
        assert!(!text.contains("alternate"));
    }

    #[test]
    fn neutral_row_distinguishes_no_body_and_known_zero_attachments() {
        let text = rendered_row(MessagePresentation {
            attachment_count: Some(0),
            ..Default::default()
        });
        assert!(text.contains("Unknown"));
        assert!(text.contains("No content"));
        assert!(text.contains("-- 0 attachment"));
    }
}
