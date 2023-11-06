// ui/logs.rs

//! Panel showing log messages

use ratatui::style::{Color, Style};
use ratatui::widgets::block::Block;
use ratatui::widgets::Borders;
use tui_logger::TuiLoggerWidget;

pub const LOG_BLOCK_HEIGHT: u16 = 15;

// Draw the logs panel
pub fn draw_logs<'a>() -> TuiLoggerWidget<'a> {
    TuiLoggerWidget::default()
        .style_error(Style::default().fg(Color::Red))
        .style_debug(Style::default().fg(Color::Green))
        .style_warn(Style::default().fg(Color::Yellow))
        .style_trace(Style::default().fg(Color::Gray))
        .style_info(Style::default().fg(Color::Blue))
        .block(
            Block::default()
                .title("Logs")
                .border_style(Style::default().fg(Color::White).bg(Color::Black))
                .borders(Borders::ALL),
        )
        .style(Style::default().fg(Color::White).bg(Color::Black))
}
