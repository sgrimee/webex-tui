#![allow(unused_imports)]

use std::time::Duration;

use log::error;

use ratatui::backend::Backend;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::terminal::Frame;
use ratatui::terminal::Terminal;
use ratatui::text::Span;
use ratatui::widgets::block::{Block, BorderType};
use ratatui::widgets::{Borders, Cell, Paragraph, Row, Table};
use tui_logger::TuiLoggerWidget;

#[allow(deprecated)]
use tui_textarea::TextArea;

use super::actions::Actions;
use super::state::AppState;
use crate::app::App;

const TITLE_BLOCK_HEIGHT: u16 = 3;
const BODY_BLOCK_HEIGHT_MIN: u16 = 5;
const MSG_INPUT_BLOCK_HEUGHT: u16 = 5;
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
        Constraint::Length(MSG_INPUT_BLOCK_HEUGHT),
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

    let msg_output = draw_msg_output(app.msg_output_textarea.clone());
    rect.render_widget(msg_output.widget(), body_chunks[0]);

    let help = draw_help(app.actions());
    rect.render_widget(help, body_chunks[1]);

    let msg_input = draw_msg_input(app.state(), app.msg_input_textarea.clone());
    rect.render_widget(msg_input.widget(), chunks[2]);

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
        TITLE_BLOCK_HEIGHT + BODY_BLOCK_HEIGHT_MIN + MSG_INPUT_BLOCK_HEUGHT + LOG_BLOCK_HEIGHT;

    if rect.height < min_height {
        error!("Require height >= {}, (got {})", min_height, rect.height);
    }
}

fn draw_msg_output<'a>(mut textarea: TextArea<'a>) -> TextArea<'a> {
    textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .title("Received messages"),
    );
    textarea
}

fn draw_msg_input<'a>(state: &AppState, mut textarea: TextArea<'a>) -> TextArea<'a> {
    let title = if state.is_editing() {
        Span::styled(
            "Type your message, Esc to exit, Alt+Enter to send and exit.",
            Style::default().fg(Color::Yellow),
        )
    } else {
        Span::styled("Type e to enter message edit mode", Style::default())
    };
    textarea.set_block(Block::default().borders(Borders::ALL).title(title));
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
