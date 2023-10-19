use ratatui::layout::Alignment;
use ratatui::style::{Color, Style};
use ratatui::widgets::block::{Block, BorderType};
use ratatui::widgets::{Borders, Paragraph};

use crate::app::App;

pub const TITLE_BLOCK_HEIGHT: u16 = 3;

pub fn draw_title<'a>(app: &App) -> Paragraph<'a> {
    let title = match app.state.is_loading() {
        true => "webex-tui (loading)",
        false => "webex-tui",
    };
    Paragraph::new(title)
        .style(Style::default().fg(Color::LightCyan))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::White))
                .border_type(BorderType::Plain),
        )
}
