//! Panel showing log messages

use ratatui::{
    style::{Color, Style},
    widgets::BorderType,
};
use tui_logger::{TuiLoggerLevelOutput, TuiLoggerSmartWidget};

use crate::app::state::{ActivePane, AppState};

pub(crate) const LOG_BLOCK_PERCENTAGE: u16 = 40;

// Draw the logs panel
pub(crate) fn draw_logs<'a>(state: &AppState) -> TuiLoggerSmartWidget<'a> {
    // Highlight pane if active
    let border_style = match state.active_pane() {
        Some(ActivePane::Logs) => Style::default().fg(Color::Cyan),
        _ => Style::default(),
    };

    TuiLoggerSmartWidget::default()
        .style_error(Style::default().fg(Color::Red))
        .style_warn(Style::default().fg(Color::Yellow))
        .style_info(Style::default().fg(Color::Blue))
        .style_debug(Style::default().fg(Color::Green))
        .style_trace(Style::default().fg(Color::Gray))
        .output_separator(' ')
        .output_timestamp(Some("%H:%M:%S".to_string()))
        .output_level(Some(TuiLoggerLevelOutput::Long))
        .output_target(true)
        .output_file(true)
        .output_line(true)
        .state(&state.log_state)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
}
