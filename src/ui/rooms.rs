//! Panel with a list of rooms

use crate::app::state::{ActivePane, AppState};

use ratatui::layout::Constraint;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::block::{Block, BorderType};
use ratatui::widgets::{Borders, Cell, Row, Table};

use super::style::line_for_room_and_team_title;

pub(crate) const ROOMS_LIST_WIDTH: u16 = 32;

/// Draws the list of rooms as per selected filtering mode.
pub(crate) fn draw_rooms_table<'a>(state: &AppState) -> Table<'a> {
    // highlight pane if it is active or in search mode
    let border_style = match (state.active_pane(), state.rooms_list.search_state()) {
        (Some(ActivePane::Rooms), _)
        | (_, super::super::app::rooms_list::SearchState::Entering) => {
            Style::default().fg(state.theme.roles.border_active())
        }
        _ => Style::default().fg(state.theme.roles.border()),
    };

    // Build title based on current mode
    let title = match (
        state.rooms_list.search_query(),
        state.rooms_list.search_state(),
    ) {
        (Some(query), super::super::app::rooms_list::SearchState::Entering) => {
            format!("Search: {query}")
        }
        (Some(query), super::super::app::rooms_list::SearchState::Filtering) => {
            let selected_count = state.rooms_list.selected_room_count();
            if selected_count > 0 {
                format!("Filtered: {query} ({selected_count} selected)")
            } else {
                format!("Filtered: {query}")
            }
        }
        _ => {
            let selected_count = state.rooms_list.selected_room_count();
            if selected_count > 0 {
                format!(
                    "Filter: {:?} ({} selected)",
                    state.rooms_list.filter(),
                    selected_count
                )
            } else {
                format!("Filter: {:?}", state.rooms_list.filter())
            }
        }
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title({
            let selected_count = state.rooms_list.selected_room_count();
            if selected_count > 0 {
                format!(
                    "Filter: {:?} ({} selected)",
                    state.rooms_list.filter(),
                    selected_count
                )
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
            let line = line_for_room_and_team_title(
                ratt,
                room.unread,
                state.theme.roles.room_unread(),
                state.theme.roles.room_team(),
            );

            // Add selection indicator
            let selection_indicator = if state.rooms_list.is_room_selected(&room.id) {
                "☑ "
            } else {
                "☐ "
            };

            let cell_content = format!("{selection_indicator}{line}");
            Row::new(vec![Cell::from(cell_content)])
        })
        .collect();
    Table::new(items, &[Constraint::Length(ROOMS_LIST_WIDTH)])
        .block(block)
        .row_highlight_style(
            Style::default()
                .bg(state.theme.roles.selection_bg())
                .fg(state.theme.roles.selection_fg())
                .add_modifier(Modifier::BOLD),
        )
}
