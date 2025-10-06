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
    // highlight pane if it is active or in search mode
    let border_style = match state.active_pane() {
        Some(ActivePane::Rooms) | Some(ActivePane::Search) => Style::default().fg(Color::Cyan),
        _ => Style::default(),
    };
    
    // Build title based on current mode
    let title = if let Some(query) = state.rooms_list.search_query() {
        format!("Search: {}", query)
    } else {
        format!("Filter: {:?}", state.rooms_list.filter())
    };
    
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title({
            let selected_count = state.rooms_list.selected_room_count();
            if selected_count > 0 {
                format!("Filter: {:?} ({} selected)", state.rooms_list.filter(), selected_count)
            } else {
                title
            }
        });
    let items: Vec<_> = state
        .visible_rooms()
        .map(|room| {
            let ratt = state
                .cache
                .room_and_team_title(&room.id)
                .unwrap_or_default();
            let line = line_for_room_and_team_title(ratt, room.unread);
            
            // Add selection indicator
            let selection_indicator = if state.rooms_list.is_room_selected(&room.id) {
                "☑ "
            } else {
                "☐ "
            };
            
            let cell_content = format!("{}{}", selection_indicator, line);
            Row::new(vec![Cell::from(cell_content)])
        })
        .collect();
    Table::new(items, &[Constraint::Length(ROOMS_LIST_WIDTH)])
        .block(block)
        .row_highlight_style(
            Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
}
