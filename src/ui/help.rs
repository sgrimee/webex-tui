// ui/help.rs

//! Panel with contextual help

use ratatui::layout::Constraint;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::block::{Block, BorderType};
use ratatui::widgets::{Borders, Cell, Row, Table};

use crate::app::actions::Actions;

const HELP_KEY_WIDTH: u16 = 14;
const HELP_ACTION_WIDTH: u16 = 20;
pub const HELP_WIDTH: u16 = HELP_KEY_WIDTH + HELP_ACTION_WIDTH;

/// Draw the help panel as a `Table` containing available keys and
/// their associated `Action`
/// Argument `actions`: list of actions currently available
pub fn draw_help(actions: &Actions) -> Table {
    let key_style = Style::default().fg(Color::LightCyan);
    let help_style = Style::default().fg(Color::Gray);

    let mut rows = vec![];
    for action in actions.actions().iter() {
        let mut first = true;
        for key in action.keys() {
            let help = if first {
                first = false;
                action.to_string()
            } else {
                String::from("")
            };
            let row = Row::new(vec![
                Cell::from(Span::styled(key.to_string(), key_style)),
                Cell::from(Span::styled(help, help_style)),
            ]);
            rows.push(row);
        }
    }

    Table::new(rows)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("Help"),
        )
        .widths(&[
            Constraint::Length(HELP_KEY_WIDTH),
            Constraint::Min(HELP_ACTION_WIDTH),
        ])
        .column_spacing(1)
}
