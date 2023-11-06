// ui/mod.rs

//! ratatui user interface

use log::*;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::terminal::Frame;
use ratatui::widgets::{Scrollbar, ScrollbarOrientation};

mod editor;
mod help;
mod logs;
mod messages;
mod rooms;
mod title;

use crate::app::state::AppState;
use editor::{draw_msg_input, MSG_INPUT_BLOCK_HEIGHT};
use help::{draw_help, HELP_WIDTH};
use logs::{draw_logs, LOG_BLOCK_HEIGHT};
use messages::{draw_msg_table, ACTIVE_ROOM_MIN_WIDTH, ROOM_MIN_HEIGHT};
use rooms::{draw_rooms_table, ROOMS_LIST_WIDTH};
use title::{draw_title, TITLE_BLOCK_HEIGHT};

/// Render all blocks.
pub fn render(rect: &mut Frame, state: &mut AppState) {
    let size = rect.size();
    check_size(&size, state);

    let mut app_constraints = vec![
        Constraint::Length(TITLE_BLOCK_HEIGHT),
        Constraint::Min(ROOM_MIN_HEIGHT + MSG_INPUT_BLOCK_HEIGHT),
    ];
    if state.show_logs {
        app_constraints.push(Constraint::Length(LOG_BLOCK_HEIGHT));
    }

    // Vertical layout
    let app_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(app_constraints.as_ref())
        .split(size);

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
    let (msg_table, nb_messages) = draw_msg_table(state, &messages_area);
    state.messages_list.set_nb_messages(nb_messages);
    // state.messages_list.set_nb_lines(nb_lines);
    rect.render_stateful_widget(
        msg_table,
        messages_area,
        state.messages_list.table_state_mut(),
    );
    // rect.render_stateful_widget(
    //     Scrollbar::default()
    //         .orientation(ScrollbarOrientation::VerticalRight)
    //         .begin_symbol(Some("↑"))
    //         .end_symbol(Some("↓")),
    //     messages_area,
    //     state.messages_list.scroll_state_mut(),
    // );

    // Message input
    let msg_input = draw_msg_input(state);
    rect.render_widget(msg_input.widget(), room_rows[1]);

    // Help
    if state.show_help {
        let help = draw_help(&state.actions);
        rect.render_widget(help, body_columns[2]);
    }

    // Logs
    if state.show_logs {
        let logs = draw_logs();
        rect.render_widget(logs, app_rows[2]);
    }
}

/// Logs warnings when terminal size constraints are not respected.
fn check_size(rect: &Rect, state: &AppState) {
    // TODO: log only once if the size does not change
    let mut min_width = ROOMS_LIST_WIDTH + ACTIVE_ROOM_MIN_WIDTH;
    if state.show_help {
        min_width += HELP_WIDTH
    };
    if rect.width < min_width {
        warn!("Require width >= {}, (got {})", min_width, rect.width);
    }

    let mut min_height = TITLE_BLOCK_HEIGHT + ROOM_MIN_HEIGHT + MSG_INPUT_BLOCK_HEIGHT;
    if state.show_logs {
        min_height += LOG_BLOCK_HEIGHT
    };
    if rect.height < min_height {
        warn!("Require height >= {}, (got {})", min_height, rect.height);
    }
}
