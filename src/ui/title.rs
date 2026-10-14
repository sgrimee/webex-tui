// ui/title.rs

//! Panel with application title.

use ratatui::layout::Alignment;
use ratatui::style::Style;
use ratatui::widgets::{Block, BorderType};
use ratatui::widgets::{Borders, Paragraph};

use crate::app::state::AppState;

pub(crate) const TITLE_BLOCK_HEIGHT: u16 = 3;

/// Draws the application title panel.
pub(crate) fn draw_title<'a>(state: &AppState) -> Paragraph<'a> {
    Paragraph::new(title_text(state))
        .style(Style::default().fg(state.theme.roles.title()))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(state.theme.roles.border()))
                .border_type(BorderType::Rounded),
        )
}

/// The title, followed by the bridge status line when the bridge is used.
fn title_text(state: &AppState) -> String {
    let title = match state.is_loading {
        true => "webex-tui (loading)",
        false => "webex-tui",
    };
    match &state.bridge {
        Some(bridge) if !bridge.line.is_empty() => format!("{title} · {}", bridge.line),
        _ => title.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::BridgeView;

    #[test]
    fn the_bridge_status_line_follows_the_title_only_when_present() {
        let mut state = AppState::default();
        assert_eq!(title_text(&state), "webex-tui");
        state.bridge = Some(BridgeView {
            line: "bridge online · Webex connected".into(),
            review_open: false,
        });
        assert_eq!(
            title_text(&state),
            "webex-tui · bridge online · Webex connected"
        );
    }
}
