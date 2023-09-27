use crate::app::actions::Actions;
use crate::app::state::AppState;
use crate::app::App;
// use crate::app::teams_store::{RoomId, TeamsStore};
// use webex::Room;

// use ratatui::backend::CrosstermBackend;
// use ratatui::terminal::Terminal;
// use ratatui::widgets::List;
// use ratatui::widgets::ListState;
// use ratatui::widgets::TableState;
use log::*;
use ratatui::backend::Backend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::terminal::Frame;
use ratatui::text::{Line, Span};
use ratatui::widgets::block::{Block, BorderType};
use ratatui::widgets::{Borders, Cell, Paragraph, Row, Table, Wrap};
use ratatui_textarea::TextArea;
use tui_logger::TuiLoggerWidget;

const TITLE_BLOCK_HEIGHT: u16 = 3;
const ROOM_MIN_HEIGHT: u16 = 8;
const MSG_INPUT_BLOCK_HEIGHT: u16 = 5;
const LOG_BLOCK_HEIGHT: u16 = 15;

const ROOMS_LIST_WIDTH: u16 = 32;
const ACTIVE_ROOM_MIN_WIDTH: u16 = 32;
const HELP_WIDTH: u16 = 32;

pub fn render<B>(rect: &mut Frame<B>, app: &App)
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
    let title = draw_title(app);
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

    // Rooms list
    let rooms_list = draw_rooms_list(app);
    let mut room_list_state = app.state.room_list_state.clone();
    rect.render_stateful_widget(rooms_list, body_columns[0], &mut room_list_state);

    // Room and message edit
    let room_constraints = vec![
        Constraint::Min(ROOM_MIN_HEIGHT),
        Constraint::Length(MSG_INPUT_BLOCK_HEIGHT),
    ];
    let room_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(room_constraints)
        .split(body_columns[1]);

    let room_messages = draw_room_messages(app);
    rect.render_widget(room_messages, room_rows[0]);

    let msg_input = draw_msg_input(&app.state);
    rect.render_widget(msg_input.widget(), room_rows[1]);

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
    // TODO: log only once if the size does not change
    let mut min_width = ROOMS_LIST_WIDTH + ACTIVE_ROOM_MIN_WIDTH;
    if app.state.show_help {
        min_width += HELP_WIDTH
    };
    if rect.width < min_width {
        warn!("Require width >= {}, (got {})", min_width, rect.width);
    }

    let mut min_height = TITLE_BLOCK_HEIGHT + ROOM_MIN_HEIGHT + MSG_INPUT_BLOCK_HEIGHT;
    if app.state.show_logs {
        min_height += LOG_BLOCK_HEIGHT
    };
    if rect.height < min_height {
        warn!("Require height >= {}, (got {})", min_height, rect.height);
    }
}

fn draw_title<'a>(app: &App) -> Paragraph<'a> {
    let title = match app.is_loading() {
        true => "webex-tui (loading)",
        false => "webex-tui",
    };
    Paragraph::new(title)
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
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .title("Rooms");
    let rooms_to_display = app.rooms_for_list_mode(&app.state.room_list_mode);
    let items: Vec<_> = rooms_to_display
        .iter()
        .map(|room| {
            let mut style = Style::default();
            if app.state.teams_store.room_has_unread(&room.id) {
                style = style.fg(Color::LightBlue).add_modifier(Modifier::BOLD);
            }
            Row::new(vec![Cell::from(Span::styled(room.title.to_owned(), style))])
        })
        .collect();
    Table::new(items)
        .block(block)
        .widths(&[Constraint::Length(ROOMS_LIST_WIDTH)])
        .column_spacing(1)
        .highlight_style(
            Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
}

fn draw_room_messages<'a>(app: &App) -> Paragraph<'a> {
    let mut text = vec![];
    if let Some(selected_room_id) = app.state.selected_room_id() {
        let messages = app.state.teams_store.messages_in_room(&selected_room_id);
        for msg in messages.iter() {
            let mut line: Vec<Span> = Vec::new();
            if let Some(sender) = &msg.person_email {
                let sender_color = if app.state.teams_store.is_me(&msg.person_id) {
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
            Span::styled("Press Enter with a selected room to type", Style::default()),
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
