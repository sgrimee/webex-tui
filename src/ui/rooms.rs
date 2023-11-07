// ui/rooms.rs

//! Panel with a list of rooms

use crate::app::state::{ActivePane, AppState};

use ratatui::layout::Constraint;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::block::{Block, BorderType};
use ratatui::widgets::{Borders, Cell, Row, Table};

pub const ROOMS_LIST_WIDTH: u16 = 32;

/// Draws the list of rooms as per selected filtering mode.
pub fn draw_rooms_table<'a>(state: &AppState) -> Table<'a> {
    // highlight pane if it is active
    let border_style = match state.active_pane() {
        Some(ActivePane::Rooms) => Style::default().fg(Color::Cyan),
        _ => Style::default(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(border_style)
        .title(format!("Filter: {:?}", state.rooms_list.filter()));
    let items: Vec<_> = state
        .teams_store
        .rooms_filtered_by(
            state.rooms_list.filter(),
            state.rooms_list.active_room_id().cloned(),
        )
        .map(|room| {
            let mut style = Style::default();
            if state.teams_store.room_has_unread(&room.id) {
                style = style.fg(Color::LightBlue).add_modifier(Modifier::BOLD);
            }
            Row::new(vec![Cell::from(Span::styled(
                room.title.clone().unwrap_or_default(),
                style,
            ))])
        })
        .collect();
    Table::new(items)
        .block(block)
        .widths(&[Constraint::Length(ROOMS_LIST_WIDTH)])
        .column_spacing(1)
        .highlight_style(
            Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
}
