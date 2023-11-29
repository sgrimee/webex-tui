//! Panel with a list of rooms

use crate::app::state::{ActivePane, AppState};

use ratatui::layout::Constraint;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::block::{Block, BorderType};
use ratatui::widgets::{Borders, Cell, Row, Table};

use super::style::line_for_room_and_team_title;

pub(crate) const ROOMS_LIST_WIDTH: u16 = 32;

/// Draws the list of rooms as per selected filtering mode.
pub(crate) fn draw_rooms_table<'a>(state: &AppState) -> Table<'a> {
    // highlight pane if it is active
    let border_style = match state.active_pane() {
        Some(ActivePane::Rooms) => Style::default().fg(Color::Cyan),
        _ => Style::default(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(format!("Filter: {:?}", state.rooms_list.filter()));
    let items: Vec<_> = state
        .cache
        .rooms_info
        .rooms_filtered_by(state.rooms_list.filter())
        .map(|room| {
            let ratt = state
                .cache
                .room_and_team_title(&room.id)
                .unwrap_or_default();
            let line = line_for_room_and_team_title(ratt, room.unread);
            Row::new(vec![Cell::from(line)])
        })
        .collect();
    Table::new(items)
        .block(block)
        .widths(&[Constraint::Length(ROOMS_LIST_WIDTH)])
        .highlight_style(
            Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
}
