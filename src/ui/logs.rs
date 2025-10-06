//! Panel showing log messages

use crate::app::state::{ActivePane, AppState};
use ratatui::prelude::*;
use ratatui::widgets::*;
use tui_logger::{TuiLoggerLevelOutput, TuiLoggerSmartWidget};

pub(crate) const LOG_BLOCK_PERCENTAGE: u16 = 40;

// Draw the logs panel
pub(crate) fn draw_logs<'a>(state: &AppState) -> TuiLoggerSmartWidget<'a> {
    // Highlight pane if active
    let border_style = match state.active_pane() {
        Some(ActivePane::Logs) => Style::default().fg(state.theme.roles.border_active()),
        _ => Style::default().fg(state.theme.roles.border()),
    };

    TuiLoggerSmartWidget::default()
        .style_error(Style::default().fg(state.theme.roles.log_error()))
        .style_warn(Style::default().fg(state.theme.roles.log_warn()))
        .style_info(Style::default().fg(state.theme.roles.log_info()))
        .style_debug(Style::default().fg(state.theme.roles.log_debug()))
        .style_trace(Style::default().fg(state.theme.roles.log_trace()))
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
