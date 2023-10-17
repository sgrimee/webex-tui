pub const ROOMS_LIST_WIDTH: u16 = 32;

use crate::app::App;

use ratatui::layout::Constraint;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::block::{Block, BorderType};
use ratatui::widgets::{Borders, Cell, Row, Table};

// Draw the list of rooms as per selected filtering mode
pub fn draw_rooms_table<'a>(app: &App) -> Table<'a> {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .title(format!("Filter: {:?}", app.state.rooms_list.mode()));
    let items: Vec<_> = app
        .state
        .teams_store
        .rooms_filtered_by(app.state.rooms_list.mode(), app.state.active_room_id())
        .map(|room| {
            let mut style = Style::default();
            if app.state.teams_store.room_has_unread(&room.id) {
                style = style.fg(Color::LightBlue).add_modifier(Modifier::BOLD);
            }
            Row::new(vec![Cell::from(Span::styled(room.title.to_owned(), style))])
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
