#![allow(unused_imports)]

use std::time::Duration;

use log::error;

use ratatui::backend::Backend;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::terminal::Frame;
use ratatui::terminal::Terminal;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::block::{Block, BorderType};
use ratatui::widgets::Wrap;
use ratatui::widgets::{Borders, Cell, Paragraph, Row, Table};
use tui_logger::TuiLoggerWidget;

#[allow(deprecated)]
use tui_textarea::TextArea;

use super::actions;
use super::actions::Actions;
use super::state::AppState;
use super::teams_store::RoomId;
use super::teams_store::TeamsStore;
use crate::app::App;

const TITLE_BLOCK_HEIGHT: u16 = 3;
const ROOM_MIN_HEIGHT: u16 = 8;
const MSG_INPUT_BLOCK_HEIGHT: u16 = 5;
const LOG_BLOCK_HEIGHT: u16 = 15;

const ROOMS_LIST_WIDTH: u16 = 32;
const ACTIVE_ROOM_MIN_WIDTH: u16 = 32;
const HELP_WIDTH: u16 = 32;

pub fn draw<B>(rect: &mut Frame<B>, app: &App)
where
    B: Backend,
{
    let size = rect.size();
    check_size(&size, app);

    let mut app_constraints = vec![
        Constraint::Length(TITLE_BLOCK_HEIGHT),
        Constraint::Min(ROOM_MIN_HEIGHT + MSG_INPUT_BLOCK_HEIGHT),
    ];
    if app.show_log_window() {
        app_constraints.push(Constraint::Length(LOG_BLOCK_HEIGHT));
    }

    // Vertical layout
    let app_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(app_constraints.as_ref())
        .split(size);

    // Title
    let title = draw_title();
    rect.render_widget(title, app_rows[0]);

    // Body: left panel, active room + message input, help
    let mut body_constraints = vec![
        Constraint::Length(ROOMS_LIST_WIDTH),
        Constraint::Min(ACTIVE_ROOM_MIN_WIDTH),
    ];
    if app.state.show_help {
        body_constraints.push(Constraint::Length(HELP_WIDTH));
    }

    let body_columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(body_constraints)
        .split(app_rows[1]);

    // add rooms list here
    let rooms_list = draw_rooms_list(app);
    rect.render_widget(rooms_list, body_columns[0]);

    // Room and message edit
    if let Some(active_room) = &app.state.active_room {
        let room_constraints = vec![
            Constraint::Min(ROOM_MIN_HEIGHT),
            Constraint::Length(MSG_INPUT_BLOCK_HEIGHT),
        ];
        let room_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(room_constraints)
            .split(body_columns[1]);

        let room_messages = draw_room_messages(active_room, &app.state.teams_store);
        rect.render_widget(room_messages, room_rows[0]);

        let msg_input = draw_msg_input(&app.state);
        rect.render_widget(msg_input.widget(), room_rows[1]);
    }

    // Help
    if app.state.show_help {
        let help = draw_help(app.actions());
        rect.render_widget(help, body_columns[2]);
    }

    // Logs
    if app.show_log_window() {
        let logs = draw_logs();
        rect.render_widget(logs, app_rows[2]);
    }
}

fn check_size(rect: &Rect, app: &App) {
    let mut min_width = ROOMS_LIST_WIDTH + ACTIVE_ROOM_MIN_WIDTH;
    if app.state.show_help {
        min_width += HELP_WIDTH
    };
    if rect.width < min_width {
        error!("Require width >= {}, (got {})", min_width, rect.width);
    }

    let mut min_height = TITLE_BLOCK_HEIGHT + ROOM_MIN_HEIGHT + MSG_INPUT_BLOCK_HEIGHT;
    if app.state.show_help {
        min_height += LOG_BLOCK_HEIGHT
    };
    if rect.height < min_height {
        error!("Require height >= {}, (got {})", min_height, rect.height);
    }
}

fn draw_title<'a>() -> Paragraph<'a> {
    Paragraph::new("Webex TUI")
        .style(Style::default().fg(Color::LightCyan))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::White))
                .border_type(BorderType::Plain),
        )
}

fn draw_rooms_list<'a>(app: &App) -> Table<'a> {
    let mut rows = vec![];
    for room in app.state.teams_store.rooms() {
        let row = Row::new(vec![Cell::from(Span::raw(room.title.to_owned()))]);
        rows.push(row);
    }
    Table::new(rows)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .title("Rooms"),
        )
        .widths(&[Constraint::Length(ROOMS_LIST_WIDTH)])
        .column_spacing(1)
}

fn draw_room_messages<'a>(room_id: &RoomId, store: &TeamsStore) -> Paragraph<'a> {
    let messages = store.messages_in_room(room_id);
    let mut text = vec![];
    for msg in messages.iter() {
        let mut line: Vec<Span> = Vec::new();
        if let Some(sender) = &msg.person_email {
            let sender_color = if store.is_me(&msg.person_id) {
                Color::Yellow
            } else {
                Color::Red
            };
            line.push(Span::styled(
                sender.clone(),
                Style::default().fg(sender_color),
            ));
            line.push(Span::raw(" > "));
        }
        if let Some(raw_text) = &msg.text {
            line.push(Span::raw(raw_text.clone()));
        }
        text.push(Line::from(line));
    }
    Paragraph::new(text)
        .block(
            Block::default()
                .title("Messages in room")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true })
}

fn draw_msg_input<'a>(state: &'a AppState<'a>) -> TextArea<'a> {
    let (title, borders_style) = if state.editing_mode {
        (
            Span::styled(
                "Type your message, Enter to send, Alt+Enter for new line, Esc to exit.",
                Style::default().fg(Color::Yellow),
            ),
            Style::default().fg(Color::Yellow),
        )
    } else {
        (
            Span::styled("Type m to enter message edit mode", Style::default()),
            Style::default(),
        )
    };
    let mut textarea = state.msg_input_textarea.clone();
    textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(borders_style)
            .title(title),
    );
    textarea.set_cursor_line_style(Style::default());
    textarea
}

fn draw_help(actions: &Actions) -> Table {
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
                .border_type(BorderType::Plain)
                .title("Help"),
        )
        .widths(&[Constraint::Length(11), Constraint::Min(20)])
        .column_spacing(1)
}

fn draw_logs<'a>() -> TuiLoggerWidget<'a> {
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
