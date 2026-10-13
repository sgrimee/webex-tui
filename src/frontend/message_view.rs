//! Canonical, disposable per-room message view for the bridge-backed frontend.
//!
//! Dormant: nothing in App/main installs bridge data here yet, and the legacy
//! `app::cache` (with its legacy-only parent repair) is untouched until cutover.
//!
//! Rules, all enforced before any mutation so a refused input changes nothing:
//! - Every snapshot must belong to this room, carry valid opaque IDs and a
//!   canonical daemon revision. Legacy (`revision: None`) data is refused.
//! - Revisions are opaque, strictly increasing per message within one mirror
//!   instance (bridge mirror-wide counter): only compare them; never assume 0
//!   means created or +1 steps. Use `updated_at` for "edited".
//! - Messages are keyed by ID. An older revision never replaces a newer one;
//!   an equal revision is an idempotent duplicate (API result, write echo or
//!   SSE refetch). A newer revision replaces the stored snapshot wholesale,
//!   including its parent: canonical parentage comes from the daemon, and the
//!   old "keep the cached parent" repair rule is deliberately not carried over.
//! - Tombstones are terminal. Deleting purges body and attachment metadata
//!   immediately, fences the ID against any later active snapshot (late write
//!   success, stale page or SSE refetch) and never decreases the revision.
//! - Timestamps are never fabricated. Ordering parses RFC 3339 to UTC (mixed
//!   precision/offsets compare chronologically); untimed or unparsable
//!   messages order after timed ones, deterministically by ID, via an internal
//!   sort key that is never displayed.
//!
//! Grouping reuses the bridge's UI-neutral `webex_bridge::threads::group`, a
//! pure function over IDs/timestamps (no DTO, transport or credential). On top:
//! a missing root yields an explicit placeholder row; a deleted root with
//! surviving replies yields a deleted placeholder row; deleted leaves and
//! deleted roots without visible replies are hidden. Rows are addressed by ID
//! so selection can be re-anchored after reordering. No body is logged and no
//! body-bearing type implements Debug/Display.

use super::{
    data::{MessageId, MessagePage, MessageSnapshot, MessageStatus, RoomId},
    threads::{ThreadPage, ThreadParent},
    write_rules::valid_opaque_id,
};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use webex_bridge::threads::{group, ThreadMessage};

/// Result of applying one canonical snapshot. No outcome is an activity or
/// unread signal by itself: history and thread pages are never activity, and
/// unread must come only from SSE `message.created` compared with a read
/// watermark. `Inserted` vs `Restored` only says whether this view has seen
/// the ID before (across resync clears).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UpsertOutcome {
    /// First active snapshot this view has ever held for this ID.
    Inserted,
    /// Active snapshot for an ID previously held and cleared by a resync.
    Restored,
    /// Newer active revision replaced the stored snapshot.
    Revised,
    /// Message is now (or was first seen as) a tombstone.
    Tombstoned,
    /// Same revision already stored, or tombstone already applied.
    Unchanged,
    /// Older revision than stored; ignored.
    Stale,
    /// Active snapshot for a tombstoned ID; ignored, never resurrected.
    Fenced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ViewError {
    /// Snapshot or page belongs to another room.
    Room,
    /// Invalid opaque message/parent ID, or a message naming itself as parent.
    Identity,
    /// Missing canonical revision (legacy/uncorrelated data).
    Revision,
}

/// Counts for an atomically validated page application.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PageOutcome {
    pub(crate) inserted: usize,
    pub(crate) restored: usize,
    pub(crate) revised: usize,
    pub(crate) tombstoned: usize,
    pub(crate) ignored: usize,
}

/// One displayed row, in grouped display order.
pub(crate) enum ViewRow<'a> {
    Message(&'a MessageSnapshot),
    /// Root known deleted but with visible replies; body is already purged.
    DeletedRoot(&'a MessageSnapshot),
    /// Replies whose root is not in this view (unloaded, missing or unavailable).
    MissingRoot(&'a MessageId),
}

impl ViewRow<'_> {
    pub(crate) fn id(&self) -> &MessageId {
        match self {
            Self::Message(message) | Self::DeletedRoot(message) => &message.id,
            Self::MissingRoot(id) => id,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
enum RowKey {
    Message(MessageId),
    DeletedRoot(MessageId),
    MissingRoot(MessageId),
}

pub(crate) struct RoomMessageView {
    room_id: RoomId,
    messages: HashMap<MessageId, MessageSnapshot>,
    /// Deleted IDs, including ones never loaded, so no late active snapshot
    /// can resurrect them. Survives `clear_messages` for room resync.
    tombstones: HashSet<MessageId>,
    /// Every ID ever held active; also survives `clear_messages`, so a
    /// re-fetch after resync is `Restored`, not `Inserted`.
    seen: HashSet<MessageId>,
    rows: Vec<RowKey>,
}

impl RoomMessageView {
    pub(crate) fn new(room_id: RoomId) -> Self {
        Self {
            room_id,
            messages: HashMap::new(),
            tombstones: HashSet::new(),
            seen: HashSet::new(),
            rows: Vec::new(),
        }
    }

    pub(crate) fn room_id(&self) -> &RoomId {
        &self.room_id
    }

    /// Stored canonical snapshot (active or tombstone), displayed or not.
    pub(crate) fn get(&self, id: &MessageId) -> Option<&MessageSnapshot> {
        self.messages.get(id)
    }

    pub(crate) fn is_tombstoned(&self, id: &MessageId) -> bool {
        self.tombstones.contains(id)
    }

    pub(crate) fn rows(&self) -> impl Iterator<Item = ViewRow<'_>> {
        self.rows.iter().map(|key| self.resolve(key))
    }

    pub(crate) fn row(&self, index: usize) -> Option<ViewRow<'_>> {
        self.rows.get(index).map(|key| self.resolve(key))
    }

    /// Re-anchor selection by ID after any reorder.
    pub(crate) fn index_of(&self, id: &MessageId) -> Option<usize> {
        self.rows.iter().position(|key| match key {
            RowKey::Message(row) | RowKey::DeletedRoot(row) | RowKey::MissingRoot(row) => row == id,
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(crate) fn upsert(
        &mut self,
        snapshot: &MessageSnapshot,
    ) -> Result<UpsertOutcome, ViewError> {
        self.validate(snapshot)?;
        let outcome = self.apply(snapshot);
        if changed(outcome) {
            self.rebuild();
        }
        Ok(outcome)
    }

    /// ID-only tombstone (confirmed delete or metadata-only delete event).
    /// An unknown ID is still fenced; nothing is fabricated for display.
    pub(crate) fn tombstone(
        &mut self,
        id: &MessageId,
        revision: Option<u64>,
    ) -> Result<UpsertOutcome, ViewError> {
        if !valid_opaque_id(id.as_str()) {
            return Err(ViewError::Identity);
        }
        let outcome = self.apply_tombstone(id, revision);
        if changed(outcome) {
            self.rebuild();
        }
        Ok(outcome)
    }

    /// Validate the whole page first; a single invalid item refuses all of it.
    /// Cursors/completeness are owned by the caller's history tracker.
    pub(crate) fn apply_page(&mut self, page: &MessagePage) -> Result<PageOutcome, ViewError> {
        if page.room_id != self.room_id {
            return Err(ViewError::Room);
        }
        self.apply_batch(&page.messages)
    }

    /// Bounded children read. An active or deleted parent snapshot is applied
    /// with its replies; a missing/unavailable parent is never inferred to be
    /// deleted and never removes a stored root — grouping shows a placeholder
    /// only when the root is genuinely absent from this view.
    pub(crate) fn apply_thread_page(
        &mut self,
        page: &ThreadPage,
    ) -> Result<PageOutcome, ViewError> {
        if page.replies.room_id != self.room_id {
            return Err(ViewError::Room);
        }
        let parent = match &page.parent {
            // A thread parent is a root: a parent that is itself a reply
            // would create nested threads, which Webex does not have.
            ThreadParent::Active(message) | ThreadParent::Deleted(message) => {
                if message.parent_id.is_some() {
                    return Err(ViewError::Identity);
                }
                Some(message)
            }
            ThreadParent::Missing(id) | ThreadParent::Unavailable(id) => {
                if !valid_opaque_id(id.as_str()) {
                    return Err(ViewError::Identity);
                }
                None
            }
        };
        // Cheap re-check of the ThreadPage invariant: every reply names this
        // exact parent and none is the parent itself.
        let parent_id = page.parent.id();
        if page
            .replies
            .messages
            .iter()
            .any(|reply| reply.id == *parent_id || reply.parent_id.as_ref() != Some(parent_id))
        {
            return Err(ViewError::Identity);
        }
        let batch: Vec<&MessageSnapshot> =
            parent.into_iter().chain(&page.replies.messages).collect();
        self.apply_batch(batch)
    }

    fn apply_batch<'a>(
        &mut self,
        snapshots: impl IntoIterator<Item = &'a MessageSnapshot> + Clone,
    ) -> Result<PageOutcome, ViewError> {
        for snapshot in snapshots.clone() {
            self.validate(snapshot)?;
        }
        let mut outcome = PageOutcome::default();
        for snapshot in snapshots {
            match self.apply(snapshot) {
                UpsertOutcome::Inserted => outcome.inserted += 1,
                UpsertOutcome::Restored => outcome.restored += 1,
                UpsertOutcome::Revised => outcome.revised += 1,
                UpsertOutcome::Tombstoned => outcome.tombstoned += 1,
                UpsertOutcome::Unchanged | UpsertOutcome::Stale | UpsertOutcome::Fenced => {
                    outcome.ignored += 1
                }
            }
        }
        if outcome.inserted + outcome.restored + outcome.revised + outcome.tombstoned > 0 {
            self.rebuild();
        }
        Ok(outcome)
    }

    /// Drop loaded messages for a room resync; tombstone fences and the seen
    /// set are retained. Revisions are only comparable within one daemon
    /// mirror instance: on ANY continuity instance change the App must use a
    /// fresh view (or clear and drop pre-clear in-flight pages) before
    /// applying new data.
    pub(crate) fn clear_messages(&mut self) -> usize {
        let count = self.messages.len();
        self.messages.clear();
        self.rows.clear();
        count
    }

    fn validate(&self, snapshot: &MessageSnapshot) -> Result<(), ViewError> {
        if snapshot.room_id != self.room_id {
            return Err(ViewError::Room);
        }
        if !valid_opaque_id(snapshot.id.as_str())
            || snapshot
                .parent_id
                .as_ref()
                .is_some_and(|parent| !valid_opaque_id(parent.as_str()) || *parent == snapshot.id)
        {
            return Err(ViewError::Identity);
        }
        if snapshot.revision.is_none() {
            return Err(ViewError::Revision);
        }
        Ok(())
    }

    /// Pre-validated. Never partially mutates: each branch is one assignment.
    fn apply(&mut self, snapshot: &MessageSnapshot) -> UpsertOutcome {
        if snapshot.status == MessageStatus::Deleted {
            return self.apply_deleted_snapshot(snapshot);
        }
        if self.tombstones.contains(&snapshot.id) {
            return UpsertOutcome::Fenced;
        }
        match self.messages.get(&snapshot.id) {
            None => {
                self.messages.insert(snapshot.id.clone(), snapshot.clone());
                if self.seen.insert(snapshot.id.clone()) {
                    UpsertOutcome::Inserted
                } else {
                    UpsertOutcome::Restored
                }
            }
            Some(stored) => match snapshot.revision.cmp(&stored.revision) {
                std::cmp::Ordering::Less => UpsertOutcome::Stale,
                std::cmp::Ordering::Equal => UpsertOutcome::Unchanged,
                std::cmp::Ordering::Greater => {
                    self.messages.insert(snapshot.id.clone(), snapshot.clone());
                    UpsertOutcome::Revised
                }
            },
        }
    }

    fn apply_deleted_snapshot(&mut self, snapshot: &MessageSnapshot) -> UpsertOutcome {
        let mut tombstone = purged(snapshot.clone());
        match self.messages.get(&snapshot.id) {
            Some(stored) if stored.status == MessageStatus::Deleted => {
                if snapshot.revision > stored.revision {
                    // Later tombstone metadata; still deleted, no body.
                    self.messages.insert(snapshot.id.clone(), tombstone);
                    return UpsertOutcome::Revised;
                }
                UpsertOutcome::Unchanged
            }
            Some(stored) => {
                // Tombstone wins over any active revision; never lower it.
                tombstone.revision = tombstone.revision.max(stored.revision);
                self.tombstones.insert(snapshot.id.clone());
                self.messages.insert(snapshot.id.clone(), tombstone);
                UpsertOutcome::Tombstoned
            }
            None => {
                // Retain identity/parent so replies can show a deleted root.
                // The map changed even for an already-fenced (ID-only) delete,
                // so rows must rebuild: a missing-root placeholder becomes a
                // deleted-root placeholder. Tombstoned is never unread.
                self.tombstones.insert(snapshot.id.clone());
                self.messages.insert(snapshot.id.clone(), tombstone);
                UpsertOutcome::Tombstoned
            }
        }
    }

    fn apply_tombstone(&mut self, id: &MessageId, revision: Option<u64>) -> UpsertOutcome {
        let newly = self.tombstones.insert(id.clone());
        match self.messages.get(id) {
            Some(stored) if stored.status == MessageStatus::Deleted => {
                if newly {
                    UpsertOutcome::Tombstoned
                } else {
                    UpsertOutcome::Unchanged
                }
            }
            Some(stored) => {
                let mut tombstone = purged(stored.clone());
                tombstone.revision = tombstone.revision.max(revision);
                self.messages.insert(id.clone(), tombstone);
                UpsertOutcome::Tombstoned
            }
            None if newly => UpsertOutcome::Tombstoned,
            None => UpsertOutcome::Unchanged,
        }
    }

    fn resolve<'a>(&'a self, key: &'a RowKey) -> ViewRow<'a> {
        match key {
            RowKey::Message(id) => ViewRow::Message(&self.messages[id]),
            RowKey::DeletedRoot(id) => ViewRow::DeletedRoot(&self.messages[id]),
            RowKey::MissingRoot(id) => ViewRow::MissingRoot(id),
        }
    }

    fn rebuild(&mut self) {
        let threads = group(self.messages.values().map(|message| {
            ThreadMessage {
                id: message.id.as_str().to_owned(),
                parent_id: self
                    .display_parent(message)
                    .map(|id| id.as_str().to_owned()),
                created_at: sort_key(message.created_at.as_deref()),
            }
        }));
        let mut rows = Vec::with_capacity(self.messages.len());
        for thread in threads {
            let root = MessageId::new(thread.root_id);
            let replies: Vec<MessageId> = thread
                .messages
                .into_iter()
                .map(|message| MessageId::new(message.id))
                .filter(|id| *id != root && self.active(id))
                .collect();
            let root_row = match self.messages.get(&root) {
                Some(message) if message.status == MessageStatus::Active => {
                    Some(RowKey::Message(root.clone()))
                }
                Some(_) if !replies.is_empty() => Some(RowKey::DeletedRoot(root.clone())),
                Some(_) => None,
                None if !replies.is_empty() => Some(RowKey::MissingRoot(root.clone())),
                None => None,
            };
            rows.extend(root_row);
            rows.extend(replies.into_iter().map(RowKey::Message));
        }
        self.rows = rows;
    }

    /// Display grouping parent. Webex threads are one level deep, but stored
    /// data could still be malformed: a nested reply collapses to its top-most
    /// loaded ancestor (or the first absent one, shown as a missing root), and
    /// any member of a parent cycle is shown as its own root. Every message
    /// therefore appears in exactly one row. Canonical parent IDs are untouched.
    fn display_parent<'a>(&'a self, message: &'a MessageSnapshot) -> Option<&'a MessageId> {
        let mut parent = message.parent_id.as_ref()?;
        let mut visited = HashSet::from([&message.id]);
        loop {
            let Some(ancestor) = self.messages.get(parent) else {
                return Some(parent); // absent: missing-root group
            };
            let Some(next) = ancestor.parent_id.as_ref() else {
                return Some(parent); // top-most loaded root
            };
            if !visited.insert(parent) || visited.contains(next) {
                return None; // cycle: show as its own root
            }
            parent = next;
        }
    }

    fn active(&self, id: &MessageId) -> bool {
        self.messages
            .get(id)
            .is_some_and(|message| message.status == MessageStatus::Active)
    }
}

fn changed(outcome: UpsertOutcome) -> bool {
    matches!(
        outcome,
        UpsertOutcome::Inserted
            | UpsertOutcome::Restored
            | UpsertOutcome::Revised
            | UpsertOutcome::Tombstoned
    )
}

fn purged(mut snapshot: MessageSnapshot) -> MessageSnapshot {
    snapshot.status = MessageStatus::Deleted;
    snapshot.body = None;
    snapshot.attachment_count = None;
    snapshot
}

/// Internal ordering only, never stored or displayed as a timestamp. RFC 3339
/// strings are parsed and normalised to UTC with fixed nanosecond precision,
/// so `.500Z`, `Z` and `+00:00` forms compare chronologically. Timed messages
/// (prefix 0) order before untimed or unparsable ones (prefix 1); ties are
/// broken by ID inside `group`.
fn sort_key(created_at: Option<&str>) -> String {
    match created_at.and_then(|value| DateTime::parse_from_rfc3339(value).ok()) {
        Some(timestamp) => format!(
            "0{}",
            timestamp
                .with_timezone(&Utc)
                .format("%Y-%m-%dT%H:%M:%S%.9f")
        ),
        None => "1".into(),
    }
}

#[cfg(test)]
mod tests;
