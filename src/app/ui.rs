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

use super::actions::Actions;
use super::state::AppState;
use super::teams_store::TeamsStore;
use crate::app::App;

const TITLE_BLOCK_HEIGHT: u16 = 3;
const BODY_BLOCK_HEIGHT_MIN: u16 = 5;
const MSG_INPUT_BLOCK_HEIGHT: u16 = 5;
const LOG_BLOCK_HEIGHT: u16 = 10;

pub fn draw<B>(rect: &mut Frame<B>, app: &App)
where
    B: Backend,
{
    let size = rect.size();
    check_size(&size);

    let mut constraints = vec![
        Constraint::Length(TITLE_BLOCK_HEIGHT),
        Constraint::Min(BODY_BLOCK_HEIGHT_MIN),
        Constraint::Length(MSG_INPUT_BLOCK_HEIGHT),
    ];
    if app.show_log_window() {
        constraints.push(Constraint::Length(LOG_BLOCK_HEIGHT));
    }

    // Vertical layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints.as_ref())
        .split(size);

    // Title
    let title = draw_title();
    rect.render_widget(title, chunks[0]);

    // Body & Help
    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(32)].as_ref())
        .split(chunks[1]);

    if let Some(active_room) = &app.state.active_room {
        let msg_output = draw_msg_output(&active_room, &app.state.teams_store);
        rect.render_widget(msg_output, body_chunks[0]);

        let msg_input = draw_msg_input(&app.state);
        rect.render_widget(msg_input.widget(), chunks[2]);
    }

    let help = draw_help(app.actions());
    rect.render_widget(help, body_chunks[1]);

    // Logs
    if app.show_log_window() {
        let logs = draw_logs();
        rect.render_widget(logs, chunks[3]);
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

fn check_size(rect: &Rect) {
    if rect.width < 52 {
        error!("Require width >= 52, (got {})", rect.width);
    }
    let min_height =
        TITLE_BLOCK_HEIGHT + BODY_BLOCK_HEIGHT_MIN + MSG_INPUT_BLOCK_HEIGHT + LOG_BLOCK_HEIGHT;

    if rect.height < min_height {
        error!("Require height >= {}, (got {})", min_height, rect.height);
    }
}

fn draw_msg_output<'a>(room_id: &str, store: &TeamsStore) -> Paragraph<'a> {
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
