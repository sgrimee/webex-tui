//! Golden full-frame render tests: `ui::render` into a `TestBackend`, then
//! every cell's symbol and style compared against `src/ui/golden/*.txt`.
//! They pin the rendering across ratatui/widget upgrades. Synthetic data
//! only. Timestamps are more than a year old (fixed format) and masked, so the
//! goldens do not depend on the time zone or today's date.
//!
//! Regenerate after an intended change, and review the diff:
//! `UPDATE_GOLDEN=1 cargo test ui::render_tests`.

use super::render;
use crate::app::state::{ActivePane, AppState};
use crate::frontend::data::{
    MessageId, MessageSnapshot, MessageStatus, OwnedMessageBody, PersonId, PersonSnapshot, RoomId,
    RoomKind, RoomSnapshot, TeamId, TeamSnapshot,
};
use crate::theme::models::Theme;
use ratatui::{backend::TestBackend, buffer::Buffer, style::Color, Terminal};
use std::{fmt::Write as _, path::PathBuf};

const ROOM: &str = "room-a";

fn room(
    id: &str,
    title: &str,
    kind: RoomKind,
    team: Option<&str>,
) -> crate::app::cache::room::Room {
    crate::app::cache::room::Room::from_bridge_snapshot(&RoomSnapshot {
        id: RoomId::new(id),
        title: Some(title.into()),
        kind,
        team_id: team.map(TeamId::new),
        last_activity: Some("2020-03-04T05:06:07Z".into()),
        is_member: true,
        is_locked: None,
    })
    .unwrap()
}

fn message(
    id: &str,
    parent: Option<&str>,
    sender: &str,
    minute: u32,
    body: OwnedMessageBody,
) -> MessageSnapshot {
    MessageSnapshot {
        id: MessageId::new(id),
        room_id: RoomId::new(ROOM),
        revision: Some(1),
        parent_id: parent.map(MessageId::new),
        sender_id: Some(PersonId::new(sender)),
        sender_email: Some(format!("{sender}@example.invalid")),
        created_at: Some(format!("2020-03-04T05:{minute:02}:00Z")),
        updated_at: None,
        status: MessageStatus::Active,
        body: Some(body),
        attachment_count: None,
    }
}

fn text(value: &str) -> OwnedMessageBody {
    OwnedMessageBody::Forms {
        text: Some(value.into()),
        markdown: None,
        html: None,
    }
}

/// Rooms in a team and alone, an unread one, people, and a thread with a
/// deleted root, a reply, Markdown/HTML, a long wrapped line and an edit.
fn populated() -> AppState<'static> {
    let mut state = AppState::default();
    state.cache.teams.add(TeamSnapshot {
        id: TeamId::new("team-1"),
        name: Some("Synthetic Team".into()),
    });
    for r in [
        room(ROOM, "Alpha room", RoomKind::Group, Some("team-1")),
        room("room-b", "Beta direct", RoomKind::Direct, None),
        room(
            "room-c",
            "Gamma room with a rather long title",
            RoomKind::Group,
            None,
        ),
    ] {
        state.cache.rooms.update_with_room(&r);
    }
    state.cache.rooms.mark_unread(&"room-b".to_owned());
    state.cache.set_me(PersonSnapshot {
        id: PersonId::new("me"),
        display_name: Some("Synthetic Me".into()),
        emails: None,
    });
    state.cache.persons.insert(PersonSnapshot {
        id: PersonId::new("alice"),
        display_name: Some("Alice Example".into()),
        emails: None,
    });
    let room_id: String = ROOM.into();
    let view = state.cache.canonical_view_mut(&room_id);
    view.upsert(&message("m1", None, "alice", 1, text("hello there")))
        .unwrap();
    view.upsert(&message(
        "m2",
        Some("m1"),
        "me",
        2,
        OwnedMessageBody::Forms {
            text: Some("bold and code".into()),
            markdown: Some("**bold** and `code`".into()),
            html: Some("<p><strong>bold</strong> and <code>code</code></p>".into()),
        },
    ))
    .unwrap();
    let mut edited = message(
        "m3",
        None,
        "alice",
        3,
        text("a long line that has to wrap across several rows of the message table because it is wider than the panel"),
    );
    edited.updated_at = Some("2020-03-04T05:04:00Z".into());
    view.upsert(&edited).unwrap();
    view.upsert(&message("m4", None, "me", 5, text("to be deleted")))
        .unwrap();
    view.upsert(&message(
        "m5",
        Some("m4"),
        "alice",
        6,
        text("reply under a deleted root"),
    ))
    .unwrap();
    view.tombstone(&RoomId::new(ROOM), &MessageId::new("m4"), None)
        .unwrap();
    state.rooms_list.set_active_room_id(Some(room_id));
    state.update_room_selection_with_active_room();
    state
        .messages_list
        .set_nb_messages(state.num_messages_active_room());
    state
}

/// Each cell's symbol, then style runs per row. Colors use their enum names;
/// modifiers their bits, so the format does not depend on ratatui's Debug.
fn serialize(buffer: &Buffer, masks: &[String]) -> String {
    let area = buffer.area;
    let mut out = String::new();
    let mut rows = Vec::new();
    for y in 0..area.height {
        let mut line = String::new();
        for x in 0..area.width {
            line.push_str(buffer[(x, y)].symbol());
        }
        rows.push(line);
    }
    for row in &mut rows {
        for mask in masks {
            if !mask.is_empty() {
                *row = row.replace(mask.as_str(), &"T".repeat(mask.chars().count()));
            }
        }
        let _ = writeln!(out, "|{row}|");
    }
    out.push_str("--- styles (row: x0-x1 fg bg underline modifier-bits)\n");
    let color = |c: Color| format!("{c:?}");
    for y in 0..area.height {
        let mut x = 0;
        while x < area.width {
            let cell = &buffer[(x, y)];
            let key = (cell.fg, cell.bg, cell.underline_color, cell.modifier.bits());
            let start = x;
            while x < area.width {
                let next = &buffer[(x, y)];
                if (next.fg, next.bg, next.underline_color, next.modifier.bits()) != key {
                    break;
                }
                x += 1;
            }
            let _ = writeln!(
                out,
                "{y}: {start}-{} {} {} {} {:#06x}",
                x - 1,
                color(key.0),
                color(key.1),
                color(key.2),
                key.3
            );
        }
    }
    out
}

/// The timestamps as `human_timestamp` formats them in this time zone.
fn timestamp_masks() -> Vec<String> {
    (0..=6)
        .map(|minute| super::messages::human_timestamp(&format!("2020-03-04T05:{minute:02}:00Z")))
        .collect()
}

fn golden(name: &str, width: u16, height: u16, state: &mut AppState) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let frame = terminal.draw(|frame| render(frame, state)).unwrap();
    let mut actual = serialize(frame.buffer, &timestamp_masks());
    let cursor = terminal.get_cursor_position().ok();
    let _ = writeln!(actual, "--- cursor {cursor:?}");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/ui/golden")
        .join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden {}: run with UPDATE_GOLDEN=1",
            path.display()
        )
    });
    if expected != actual {
        let diff: Vec<String> = expected
            .lines()
            .zip(actual.lines())
            .enumerate()
            .filter(|(_, (e, a))| e != a)
            .take(12)
            .map(|(i, (e, a))| format!("line {}:\n  expected {e}\n  actual   {a}", i + 1))
            .collect();
        panic!(
            "{name} render changed ({} vs {} lines):\n{}",
            expected.lines().count(),
            actual.lines().count(),
            diff.join("\n")
        );
    }
}

#[test]
fn empty_state_renders_the_default_layout() {
    golden("empty_100x30", 100, 30, &mut AppState::default());
}

#[test]
fn populated_rooms_messages_and_help() {
    let mut state = populated();
    state.set_active_pane(Some(ActivePane::Rooms));
    golden("populated_120x36", 120, 36, &mut state);
}

#[test]
fn selected_message_and_composing_reply() {
    let mut state = populated();
    state.show_help = false;
    state.messages_list.select_index(1);
    state.set_active_pane(Some(ActivePane::Compose));
    state.message_editor.set_is_composing(true);
    for key in "draft".chars() {
        state.message_editor.input(tui_textarea::Input {
            key: tui_textarea::Key::Char(key),
            ..Default::default()
        });
    }
    state.message_editor.insert_newline();
    state.message_editor.input(tui_textarea::Input {
        key: tui_textarea::Key::Char('x'),
        ..Default::default()
    });
    golden("compose_100x30", 100, 30, &mut state);
}

#[test]
fn narrow_without_rooms_scrolls_with_a_scrollbar() {
    let mut state = populated();
    state.show_rooms = false;
    state.show_help = false;
    state.set_active_pane(Some(ActivePane::Messages));
    state.messages_list.select_index(4);
    golden("narrow_50x16", 50, 16, &mut state);
}

#[test]
fn logs_panel_and_dracula_theme() {
    let mut state = populated();
    state.theme = serde_yaml::from_str::<Theme>(include_str!("../../themes/dracula.yml")).unwrap();
    state.show_logs = true;
    state.set_active_pane(Some(ActivePane::Logs));
    golden("logs_dracula_110x40", 110, 40, &mut state);
}

/// The draft editor scrolls: a long line (cursor at its end) scrolls
/// horizontally, and a draft taller than the editor scrolls vertically.
#[test]
fn a_long_multiline_draft_scrolls_in_the_editor() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut state = populated();
    state.show_help = false;
    state.set_active_pane(Some(ActivePane::Compose));
    state.message_editor.set_is_composing(true);
    state.message_editor.reset_with_text(format!(
        "first\nsecond\nthird\nfourth\n{}",
        "long-draft-line ".repeat(8)
    ));
    for code in [
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::End,
    ] {
        state
            .message_editor
            .input(KeyEvent::new(code, KeyModifiers::NONE));
    }
    golden("long_draft_80x24", 80, 24, &mut state);
}
