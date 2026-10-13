//! Deterministic synthetic fixtures only; no I/O, logging or terminal.
use super::*;
use crate::frontend::{
    data::{OwnedMessageBody, PageCursor},
    threads::{ThreadPage, ThreadParent},
};

const ROOM: &str = "room/a?opaque";

fn msg(id: &str, parent: Option<&str>, created: Option<String>, revision: u64) -> MessageSnapshot {
    MessageSnapshot {
        id: MessageId::new(id),
        room_id: RoomId::new(ROOM),
        revision: Some(revision),
        parent_id: parent.map(MessageId::new),
        sender_id: None,
        sender_email: None,
        created_at: created,
        updated_at: None,
        status: MessageStatus::Active,
        body: Some(OwnedMessageBody::Forms {
            text: Some(format!("{id} r{revision}")),
            markdown: None,
            html: None,
        }),
        attachment_count: Some(1),
    }
}

fn at(minute: u32) -> Option<String> {
    Some(format!("2026-01-01T00:{minute:02}:00Z"))
}

fn deleted(
    id: &str,
    parent: Option<&str>,
    created: Option<String>,
    revision: u64,
) -> MessageSnapshot {
    let mut snapshot = msg(id, parent, created, revision);
    snapshot.status = MessageStatus::Deleted; // body deliberately left set
    snapshot
}

fn view() -> RoomMessageView {
    RoomMessageView::new(RoomId::new(ROOM))
}

fn ids(view: &RoomMessageView) -> Vec<String> {
    view.rows()
        .map(|row| match row {
            ViewRow::Message(m) => m.id.as_str().to_owned(),
            ViewRow::DeletedRoot(m) => format!("deleted:{}", m.id.as_str()),
            ViewRow::MissingRoot(id) => format!("missing:{}", id.as_str()),
        })
        .collect()
}

fn text(view: &RoomMessageView, id: &str) -> Option<String> {
    let snapshot = view.get(&MessageId::new(id))?;
    snapshot.presentation().body.text.map(str::to_owned)
}

fn page(messages: Vec<MessageSnapshot>) -> MessagePage {
    MessagePage {
        room_id: RoomId::new(ROOM),
        messages,
        next_cursor: Some(PageCursor::new("opaque-cursor")),
    }
}

fn state(view: &RoomMessageView) -> (Vec<String>, usize, usize) {
    (ids(view), view.messages.len(), view.tombstones.len())
}

#[test]
fn invalid_room_identity_or_revision_is_refused_without_mutation() {
    let mut v = view();
    v.upsert(&msg("a", None, at(1), 1)).unwrap();
    let before = state(&v);
    let mut foreign = msg("b", None, at(2), 1);
    foreign.room_id = RoomId::new("room/b");
    let mut legacy = msg("b", None, at(2), 1);
    legacy.revision = None;
    let cases = [
        (foreign, ViewError::Room),
        (msg("", None, at(2), 1), ViewError::Identity),
        (msg("bad\nid", None, at(2), 1), ViewError::Identity),
        (msg(&"x".repeat(1025), None, at(2), 1), ViewError::Identity),
        (msg("b", Some(""), at(2), 1), ViewError::Identity),
        (msg("b", Some("b"), at(2), 1), ViewError::Identity),
        (legacy, ViewError::Revision),
    ];
    for (snapshot, error) in cases {
        assert_eq!(v.upsert(&snapshot), Err(error));
        assert!(state(&v) == before);
    }
    assert_eq!(
        v.tombstone(&MessageId::new("bad\u{7}"), None),
        Err(ViewError::Identity)
    );
    assert!(state(&v) == before);
}

#[test]
fn revision_fences_equal_older_and_newer() {
    let mut v = view();
    assert_eq!(
        v.upsert(&msg("a", None, at(1), 2)),
        Ok(UpsertOutcome::Inserted)
    );
    assert_eq!(
        v.upsert(&msg("a", None, at(1), 2)),
        Ok(UpsertOutcome::Unchanged)
    );
    assert_eq!(
        v.upsert(&msg("a", None, at(1), 1)),
        Ok(UpsertOutcome::Stale)
    );
    assert_eq!(text(&v, "a").as_deref(), Some("a r2"));
    assert_eq!(
        v.upsert(&msg("a", None, at(1), 3)),
        Ok(UpsertOutcome::Revised)
    );
    assert_eq!(text(&v, "a").as_deref(), Some("a r3"));
    assert_eq!(
        v.upsert(&msg("a", None, at(1), 2)),
        Ok(UpsertOutcome::Stale)
    );
    assert_eq!(text(&v, "a").as_deref(), Some("a r3"));
    assert_eq!(v.len(), 1);
}

#[test]
fn duplicate_write_success_and_sse_echoes_are_idempotent() {
    let mut v = view();
    let canonical = msg("sent", None, at(5), 0);
    let outcomes: Vec<_> = (0..3).map(|_| v.upsert(&canonical).unwrap()).collect();
    assert_eq!(
        outcomes,
        [
            UpsertOutcome::Inserted,
            UpsertOutcome::Unchanged,
            UpsertOutcome::Unchanged
        ]
    );
    let outcome = v.apply_page(&page(vec![canonical.clone()])).unwrap();
    assert_eq!(outcome.ignored, 1);
    assert_eq!(ids(&v), ["sent"]);
}

#[test]
fn newer_revision_corrects_parent_and_older_cannot_revert_it() {
    let mut v = view();
    v.upsert(&msg("root-a", None, at(1), 0)).unwrap();
    v.upsert(&msg("root-b", None, at(2), 0)).unwrap();
    v.upsert(&msg("reply", Some("root-a"), at(3), 0)).unwrap();
    assert_eq!(ids(&v), ["root-a", "reply", "root-b"]);
    // Canonical daemon parent wins on a newer revision; no cached-parent repair.
    assert_eq!(
        v.upsert(&msg("reply", Some("root-b"), at(3), 1)),
        Ok(UpsertOutcome::Revised)
    );
    assert_eq!(ids(&v), ["root-a", "root-b", "reply"]);
    assert_eq!(
        v.upsert(&msg("reply", Some("root-a"), at(3), 0)),
        Ok(UpsertOutcome::Stale)
    );
    assert_eq!(ids(&v), ["root-a", "root-b", "reply"]);
    // A newer revision may also drop the parent entirely.
    v.upsert(&msg("reply", None, at(3), 2)).unwrap();
    assert_eq!(ids(&v), ["root-a", "root-b", "reply"]);
    assert!(v.get(&MessageId::new("reply")).unwrap().parent_id.is_none());
}

#[test]
fn tombstone_beats_late_success_stale_page_and_higher_active_revision() {
    let mut v = view();
    v.upsert(&msg("a", None, at(1), 4)).unwrap();
    assert_eq!(
        v.tombstone(&MessageId::new("a"), Some(2)),
        Ok(UpsertOutcome::Tombstoned)
    );
    let stored = v.get(&MessageId::new("a")).unwrap();
    assert_eq!(stored.status, MessageStatus::Deleted);
    assert!(stored.body.is_none());
    assert_eq!(stored.attachment_count, None);
    assert_eq!(stored.revision, Some(4)); // never lowered
    for late in [msg("a", None, at(1), 4), msg("a", None, at(1), 9)] {
        assert_eq!(v.upsert(&late), Ok(UpsertOutcome::Fenced));
    }
    let outcome = v.apply_page(&page(vec![msg("a", None, at(1), 3)])).unwrap();
    assert_eq!(outcome.ignored, 1);
    assert!(text(&v, "a").is_none());
    assert!(v.is_empty()); // deleted leaf is hidden
    assert_eq!(
        v.tombstone(&MessageId::new("a"), None),
        Ok(UpsertOutcome::Unchanged)
    );
}

#[test]
fn deleted_snapshot_purges_body_and_keeps_max_revision() {
    let mut v = view();
    v.upsert(&msg("a", None, at(1), 5)).unwrap();
    assert_eq!(
        v.upsert(&deleted("a", None, at(1), 3)),
        Ok(UpsertOutcome::Tombstoned)
    );
    let stored = v.get(&MessageId::new("a")).unwrap();
    assert!(stored.body.is_none() && stored.attachment_count.is_none());
    assert_eq!(stored.revision, Some(5));
    assert_eq!(
        v.upsert(&deleted("a", None, at(1), 5)),
        Ok(UpsertOutcome::Unchanged)
    );
    assert_eq!(
        v.upsert(&deleted("a", None, at(1), 6)),
        Ok(UpsertOutcome::Revised)
    );
    let stored = v.get(&MessageId::new("a")).unwrap();
    assert!(stored.body.is_none());
    assert_eq!(stored.revision, Some(6));
    // First sighting as a tombstone: fenced, never displayed, body purged.
    assert_eq!(
        v.upsert(&deleted("b", None, at(2), 1)),
        Ok(UpsertOutcome::Tombstoned)
    );
    assert!(v.get(&MessageId::new("b")).unwrap().body.is_none());
    assert_eq!(
        v.upsert(&msg("b", None, at(2), 2)),
        Ok(UpsertOutcome::Fenced)
    );
    assert!(v.is_empty());
}

#[test]
fn unknown_id_tombstone_fences_later_inserts_and_survives_resync_clear() {
    let mut v = view();
    assert_eq!(
        v.tombstone(&MessageId::new("ghost"), None),
        Ok(UpsertOutcome::Tombstoned)
    );
    assert!(v.get(&MessageId::new("ghost")).is_none()); // nothing fabricated
    assert!(v.is_empty());
    assert_eq!(
        v.upsert(&msg("ghost", None, at(1), 0)),
        Ok(UpsertOutcome::Fenced)
    );
    v.upsert(&msg("live", None, at(2), 0)).unwrap();
    assert_eq!(v.clear_messages(), 1);
    assert!(v.is_empty() && v.is_tombstoned(&MessageId::new("ghost")));
    assert_eq!(
        v.upsert(&msg("ghost", None, at(1), 1)),
        Ok(UpsertOutcome::Fenced)
    );
    assert_eq!(
        v.upsert(&msg("live", None, at(2), 0)),
        Ok(UpsertOutcome::Restored)
    );
}

#[test]
fn deleted_root_placeholder_only_while_replies_survive() {
    let mut v = view();
    v.upsert(&msg("root", None, at(1), 0)).unwrap();
    v.upsert(&msg("r1", Some("root"), at(2), 0)).unwrap();
    v.upsert(&msg("r2", Some("root"), at(3), 0)).unwrap();
    v.tombstone(&MessageId::new("root"), None).unwrap();
    assert_eq!(ids(&v), ["deleted:root", "r1", "r2"]);
    let Some(ViewRow::DeletedRoot(root)) = v.row(0) else {
        panic!("deleted root placeholder")
    };
    assert!(root.presentation().body.text.is_none());
    v.tombstone(&MessageId::new("r1"), None).unwrap();
    assert_eq!(ids(&v), ["deleted:root", "r2"]);
    v.upsert(&deleted("r2", Some("root"), at(3), 1)).unwrap();
    assert!(v.is_empty());
}

#[test]
fn reply_before_parent_shows_missing_placeholder_then_reanchors() {
    let mut v = view();
    v.upsert(&msg("other", None, at(0), 0)).unwrap();
    v.upsert(&msg("reply", Some("root"), at(5), 0)).unwrap();
    assert_eq!(ids(&v), ["other", "missing:root", "reply"]);
    assert!(matches!(v.row(1), Some(ViewRow::MissingRoot(id)) if id.as_str() == "root"));
    let selected = MessageId::new("reply");
    assert_eq!(v.index_of(&selected), Some(2));
    v.upsert(&msg("root", None, at(1), 0)).unwrap();
    assert_eq!(ids(&v), ["other", "root", "reply"]);
    assert_eq!(v.index_of(&selected), Some(2));
    assert_eq!(v.row(1).unwrap().id().as_str(), "root");
    // Orphan whose replies are all deleted shows nothing.
    v.upsert(&deleted("lonely", Some("gone"), at(9), 1))
        .unwrap();
    assert_eq!(ids(&v), ["other", "root", "reply"]);
}

#[test]
fn equal_and_absent_timestamps_order_deterministically_without_fabrication() {
    let mut v = view();
    for id in ["c", "a", "b"] {
        v.upsert(&msg(id, None, at(1), 0)).unwrap();
    }
    v.upsert(&msg("untimed-b", None, None, 0)).unwrap();
    v.upsert(&msg("untimed-a", None, None, 0)).unwrap();
    v.upsert(&msg("early", None, at(0), 0)).unwrap();
    v.upsert(&msg("late-reply", Some("a"), None, 0)).unwrap();
    assert_eq!(
        ids(&v),
        [
            "early",
            "a",
            "late-reply",
            "b",
            "c",
            "untimed-a",
            "untimed-b"
        ]
    );
    assert!(v
        .get(&MessageId::new("untimed-a"))
        .unwrap()
        .created_at
        .is_none());
    assert!(v
        .get(&MessageId::new("late-reply"))
        .unwrap()
        .created_at
        .is_none());
}

#[test]
fn pages_validate_atomically_and_count_outcomes() {
    let mut v = view();
    v.upsert(&msg("a", None, at(1), 1)).unwrap();
    let before = state(&v);
    let mut foreign = page(vec![msg("b", None, at(2), 0)]);
    foreign.room_id = RoomId::new("room/b");
    assert_eq!(v.apply_page(&foreign), Err(ViewError::Room));
    let mut legacy = msg("d", None, at(4), 0);
    legacy.revision = None;
    let invalid = page(vec![
        msg("b", None, at(2), 0),
        msg("c", None, at(3), 0),
        legacy,
    ]);
    assert_eq!(v.apply_page(&invalid), Err(ViewError::Revision));
    assert!(state(&v) == before);

    let outcome = v
        .apply_page(&page(vec![
            msg("b", None, at(2), 0),
            msg("a", None, at(1), 2),
            msg("a", None, at(1), 0),
            deleted("c", None, at(3), 1),
        ]))
        .unwrap();
    assert_eq!(
        outcome,
        PageOutcome {
            inserted: 1,
            restored: 0,
            revised: 1,
            tombstoned: 1,
            ignored: 1,
        }
    );
    assert_eq!(ids(&v), ["a", "b"]);
    assert_eq!(text(&v, "a").as_deref(), Some("a r2"));
}

#[test]
fn write_success_echo_after_history_page_is_one_row() {
    // Write success installs the canonical revision; its later SSE refetch and
    // the next history page carrying the same revision must not duplicate it.
    let mut v = view();
    let canonical = msg("mine", Some("root"), at(4), 0);
    v.upsert(&msg("root", None, at(1), 0)).unwrap();
    assert_eq!(v.upsert(&canonical), Ok(UpsertOutcome::Inserted));
    assert_eq!(v.upsert(&canonical), Ok(UpsertOutcome::Unchanged));
    v.apply_page(&page(vec![msg("root", None, at(1), 0), canonical]))
        .unwrap();
    assert_eq!(ids(&v), ["root", "mine"]);
}

fn thread(parent: ThreadParent, replies: Vec<MessageSnapshot>) -> ThreadPage {
    ThreadPage::new(
        &MessageId::new("root"),
        &RoomId::new(ROOM),
        parent,
        page(replies),
    )
    .unwrap()
}

#[test]
fn thread_pages_apply_parent_states_without_inferring_deletion() {
    let replies = || {
        vec![
            msg("r1", Some("root"), at(2), 0),
            msg("r2", Some("root"), at(3), 0),
        ]
    };
    let mut v = view();
    let outcome = v
        .apply_thread_page(&thread(
            ThreadParent::Active(msg("root", None, at(1), 0)),
            replies(),
        ))
        .unwrap();
    assert_eq!(outcome.inserted, 3);
    assert_eq!(ids(&v), ["root", "r1", "r2"]);

    // Missing/unavailable parent never deletes or hides a stored active root.
    for parent in [
        ThreadParent::Missing(MessageId::new("root")),
        ThreadParent::Unavailable(MessageId::new("root")),
    ] {
        let outcome = v.apply_thread_page(&thread(parent, replies())).unwrap();
        assert_eq!(outcome.ignored, 2);
        assert_eq!(ids(&v), ["root", "r1", "r2"]);
        assert!(!v.is_tombstoned(&MessageId::new("root")));
    }

    // Deleted parent applies a tombstone and keeps replies under a placeholder.
    let outcome = v
        .apply_thread_page(&thread(
            ThreadParent::Deleted(deleted("root", None, at(1), 1)),
            replies(),
        ))
        .unwrap();
    assert_eq!(outcome.tombstoned, 1);
    assert_eq!(ids(&v), ["deleted:root", "r1", "r2"]);

    // Unknown root in a fresh view: explicit missing placeholder.
    let mut fresh = view();
    fresh
        .apply_thread_page(&thread(
            ThreadParent::Unavailable(MessageId::new("root")),
            replies(),
        ))
        .unwrap();
    assert_eq!(ids(&fresh), ["missing:root", "r1", "r2"]);
}

#[test]
fn thread_pages_validate_atomically() {
    let mut v = view();
    v.upsert(&msg("root", None, at(1), 0)).unwrap();
    let before = state(&v);
    let mut legacy = msg("r2", Some("root"), at(3), 0);
    legacy.revision = None;
    let invalid = thread(
        ThreadParent::Active(msg("root", None, at(1), 5)),
        vec![msg("r1", Some("root"), at(2), 0), legacy],
    );
    assert_eq!(v.apply_thread_page(&invalid), Err(ViewError::Revision));
    assert!(state(&v) == before);
    assert_eq!(text(&v, "root").as_deref(), Some("root r0"));

    let mut other = view();
    let mut foreign = thread(ThreadParent::Missing(MessageId::new("root")), vec![]);
    foreign.replies.room_id = RoomId::new("room/b");
    assert_eq!(other.apply_thread_page(&foreign), Err(ViewError::Room));
    let bad_parent = ThreadPage {
        parent: ThreadParent::Missing(MessageId::new("bad\nid")),
        replies: page(vec![]),
    };
    assert_eq!(
        other.apply_thread_page(&bad_parent),
        Err(ViewError::Identity)
    );
    assert!(other.is_empty() && other.tombstones.is_empty());
}

#[test]
fn reinsert_after_resync_clear_is_restored_not_inserted() {
    let mut v = view();
    assert_eq!(
        v.upsert(&msg("a", None, at(1), 1)),
        Ok(UpsertOutcome::Inserted)
    );
    assert_eq!(v.clear_messages(), 1);
    assert!(v.is_empty());
    assert_eq!(
        v.upsert(&msg("a", None, at(1), 1)),
        Ok(UpsertOutcome::Restored)
    );
    assert_eq!(ids(&v), ["a"]);
    v.clear_messages();
    let outcome = v
        .apply_page(&page(vec![
            msg("a", None, at(1), 2),
            msg("b", None, at(2), 0),
        ]))
        .unwrap();
    assert_eq!(outcome.restored, 1);
    assert_eq!(outcome.inserted, 1);
    assert_eq!(ids(&v), ["a", "b"]);
}
