//! Some helper function to style items

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::app::cache::room_and_team_title::RoomAndTeamTitle;

/// Returns a formatted ratatui line with the room title and team name if any.
pub(crate) fn line_for_room_and_team_title<'a>(ratt: RoomAndTeamTitle, unread: bool) -> Line<'a> {
    let room_style = if unread {
        Style::default()
            .fg(Color::LightBlue)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let mut line = Line::default();
    line.spans
        .push(Span::styled(ratt.room_title.clone(), room_style));
    if let Some(team_name) = ratt.team_name.clone() {
        line.spans.push(Span::styled(
            format!(" ({})", team_name),
            Style::default().fg(Color::LightCyan),
        ));
    }
    line
}
