// ui/mod.rs

//! ratatui user interface

use log::*;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation};
use ratatui::Frame;

mod help;
mod logs;
mod message_editor;
mod messages;
mod rooms;
mod style;
mod title;

use crate::app::state::AppState;
use help::{draw_help, HELP_WIDTH};
use logs::{draw_logs, LOG_BLOCK_PERCENTAGE};
use message_editor::{draw_message_editor, MSG_INPUT_BLOCK_HEIGHT};
use messages::{draw_msg_table, ACTIVE_ROOM_MIN_WIDTH, ROOM_MIN_HEIGHT};
use rooms::{draw_rooms_table, ROOMS_LIST_WIDTH};
use title::{draw_title, TITLE_BLOCK_HEIGHT};

/// Render all blocks.
pub(crate) fn render(rect: &mut Frame, state: &mut AppState) {
    let area = rect.area();
    // Check size constraints when the size changes
    if area != state.last_frame_size {
        check_size(&area, state);
        state.last_frame_size = area;
    }

    let mut app_constraints = vec![
        Constraint::Length(TITLE_BLOCK_HEIGHT),
        Constraint::Min(ROOM_MIN_HEIGHT + MSG_INPUT_BLOCK_HEIGHT),
    ];
    if state.show_logs {
        app_constraints.push(Constraint::Percentage(LOG_BLOCK_PERCENTAGE));
    }

    // Vertical layout
    let app_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(app_constraints)
        .split(area);

    // Title
    let title = draw_title(state);
    rect.render_widget(title, app_rows[0]);

    // Body: left panel, active room + message input, help
    let mut body_constraints = vec![
        Constraint::Length(ROOMS_LIST_WIDTH),
        Constraint::Min(ACTIVE_ROOM_MIN_WIDTH),
    ];
    if state.show_help {
        body_constraints.push(Constraint::Length(HELP_WIDTH));
    }

    let body_columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(body_constraints)
        .split(app_rows[1]);

    // Rooms list
    let rooms_table = draw_rooms_table(state);
    let room_table_state = state.rooms_list.table_state_mut();
    rect.render_stateful_widget(rooms_table, body_columns[0], room_table_state);

    // Room and message edit
    let room_constraints = vec![
        Constraint::Min(ROOM_MIN_HEIGHT),
        Constraint::Length(MSG_INPUT_BLOCK_HEIGHT),
    ];
    let room_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(room_constraints)
        .split(body_columns[1]);

    // Messages list
    let messages_area = room_rows[0];
    let (msg_table, nb_messages, nb_lines) = draw_msg_table(state, &messages_area);
    state.messages_list.set_nb_messages(nb_messages);
    rect.render_stateful_widget(
        msg_table,
        messages_area,
        state.messages_list.table_state_mut(),
    );
    // Display scrollbar
    state.messages_list.set_nb_lines(nb_lines);
    state.messages_list.scroll_to_selection();
    rect.render_stateful_widget(
        Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓")),
        messages_area,
        state.messages_list.scroll_state_mut(),
    );

    // Help
    if state.show_help {
        let help = draw_help(&state.actions);
        rect.render_widget(help, body_columns[2]);
    }

    // Logs
    if state.show_logs {
        let logs = draw_logs(state);
        rect.render_widget(logs, app_rows[2]);
    }

    // Message input
    let editor = draw_message_editor(state);
    rect.render_widget(&editor, room_rows[1]);
}

/// Logs warnings when terminal size constraints are not respected.
fn check_size(rect: &Rect, state: &AppState) {
    let mut min_width = ROOMS_LIST_WIDTH + ACTIVE_ROOM_MIN_WIDTH;
    if state.show_help {
        min_width += HELP_WIDTH
    };
    if rect.width < min_width {
        warn!("Require width >= {}, (got {})", min_width, rect.width);
    }

    let min_height = TITLE_BLOCK_HEIGHT + ROOM_MIN_HEIGHT + MSG_INPUT_BLOCK_HEIGHT;
    if rect.height < min_height {
        warn!("Require height >= {}, (got {})", min_height, rect.height);
    }
}
