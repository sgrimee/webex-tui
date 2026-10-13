//! Dormant App-side glue between the bridge runtime and the presentation
//! cache. Not constructed by `main` or App; driven only by fake-backed tests
//! until the separately approved startup slice.
//!
//! Owns the App-side endpoints (presentation pump, command mailbox, runtime
//! control) and an App read tracker, and reduces drained runtime events into
//! `AppState`'s cache using canonical `RoomMessageView`s (§7.2):
//! - snapshot install → reset every loaded view (a snapshot may come from a
//!   new daemon instance whose revision counter restarted), replace identity
//!   and the room list, ack the install and wake the runtime, then request the
//!   newest page of every loaded room;
//! - per-room read generations: a page/message/thread result issued before a
//!   room reset is dropped; pump `Stale` results are settled and re-requested
//!   once if still relevant; `Duplicate(existing)` re-keys to the existing
//!   request; `TimedOut`/other unavailability is settled and recorded per
//!   room, never retried automatically;
//! - whole-mirror reconcile → Rooms plus the newest page of every loaded room
//!   (message-scoped hints are subsumed upstream);
//! - unread only from SSE `message.created`: the created message is fetched
//!   and its room marked unread only if it is not from the account and the
//!   room is not the active room. Pages, resyncs and edits are never activity;
//! - redactions apply immediately as tombstones to loaded views;
//! - a closed pump marks the glue closed and forgets outstanding reads.
//!
//! No writes (Step 5D), no logging of content, no capability access.

use super::{
    cache::room::{Room, RoomId},
    state::AppState,
};
use crate::{
    bridge::{
        commands::{CommandMailbox, ReadAdmission},
        presentation::{PresentationEndpoint, PumpEvent},
        read_dispatch::{ReadTerminal, ReadUnavailable},
        runtime::RuntimeControl,
        session::SessionPhase,
        snapshot::BridgeSnapshot,
    },
    frontend::{
        data::{self, MessageId, MessagePage, PageCursor, PersonId, RoomSnapshot},
        envelopes::{
            FrontendIntent, FrontendResult, PageRequest, ReadOutcome, RequestId, RequestSequence,
        },
        events::{ChangeScope, MessageKey},
        lifecycle::{LifecycleProjection, ProjectionEffects, RefreshTarget},
        message_view::RoomMessageView,
    },
};
use std::collections::{HashMap, HashSet, VecDeque};

/// Why a read was issued; carries the room generation it was issued under.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Purpose {
    Rooms,
    Room(RoomId),
    Person(String),
    NewestPage {
        room: RoomId,
        generation: u64,
    },
    OlderPage {
        room: RoomId,
        generation: u64,
    },
    Message {
        room: RoomId,
        generation: u64,
        unread_check: bool,
    },
    Thread {
        room: RoomId,
        generation: u64,
    },
}

impl Purpose {
    fn room(&self) -> Option<&RoomId> {
        match self {
            Self::NewestPage { room, .. }
            | Self::OlderPage { room, .. }
            | Self::Message { room, .. }
            | Self::Thread { room, .. } => Some(room),
            _ => None,
        }
    }
    fn generation(&self) -> Option<u64> {
        match self {
            Self::NewestPage { generation, .. }
            | Self::OlderPage { generation, .. }
            | Self::Message { generation, .. }
            | Self::Thread { generation, .. } => Some(*generation),
            _ => None,
        }
    }
}

/// Metadata-only glue status for a future status line.
#[derive(Default)]
pub(crate) struct GlueStatus {
    pub(crate) phase: Option<SessionPhase>,
    pub(crate) projection: LifecycleProjection,
    /// The runtime/pump is gone: displayed data is stale.
    pub(crate) closed: bool,
    /// Rooms whose last page read timed out or was rejected (no auto retry).
    pub(crate) failed_rooms: HashSet<RoomId>,
}

pub(crate) struct BridgeGlue {
    endpoint: PresentationEndpoint<BridgeSnapshot>,
    mailbox: CommandMailbox,
    control: RuntimeControl,
    sequence: RequestSequence,
    pending: HashMap<RequestId, (FrontendIntent, Purpose)>,
    /// Bounded by distinct intents: re-submitted when read credits free up.
    deferred: VecDeque<(FrontendIntent, Purpose)>,
    generations: HashMap<RoomId, u64>,
    /// Oldest-page cursor per loaded room for explicit "load older".
    cursors: HashMap<RoomId, Option<PageCursor>>,
    page_size: u32,
    status: GlueStatus,
}

impl BridgeGlue {
    pub(crate) fn new(
        endpoint: PresentationEndpoint<BridgeSnapshot>,
        mailbox: CommandMailbox,
        control: RuntimeControl,
        page_size: u32,
    ) -> Self {
        Self {
            endpoint,
            mailbox,
            control,
            sequence: RequestSequence::default(),
            pending: HashMap::new(),
            deferred: VecDeque::new(),
            generations: HashMap::new(),
            cursors: HashMap::new(),
            // The UI API accepts 1..=100; clamp rather than refuse.
            page_size: page_size.clamp(1, 100),
            status: GlueStatus::default(),
        }
    }

    pub(crate) fn status(&self) -> &GlueStatus {
        &self.status
    }

    pub(crate) fn outstanding_reads(&self) -> usize {
        self.pending.len() + self.deferred.len()
    }

    /// Explicit user navigation: load the newest page of a room once.
    pub(crate) fn open_room(&mut self, state: &mut AppState, room: &RoomId) {
        if state.cache.canonical_view(room).is_some() {
            return;
        }
        if state.cache.canonical_view_mut(room).is_some() {
            self.request_newest(room);
        }
    }

    /// Explicit user action: load the page before the oldest loaded one.
    pub(crate) fn load_older(&mut self, room: &RoomId) {
        let Some(Some(cursor)) = self.cursors.get(room).cloned() else {
            return; // nothing loaded yet, or history exhausted
        };
        let generation = self.generation(room);
        self.request(
            FrontendIntent::ListMessagesInRoom {
                room_id: data::RoomId::new(room.as_str()),
                page: PageRequest {
                    before: Some(cursor),
                    max: self.page_size,
                },
            },
            Purpose::OlderPage {
                room: room.clone(),
                generation,
            },
        );
    }

    /// Explicit user action: load one page of a thread's replies.
    pub(crate) fn load_thread(&mut self, room: &RoomId, parent: &MessageId) {
        let generation = self.generation(room);
        self.request(
            FrontendIntent::UpdateChildrenMessages {
                parent_id: parent.clone(),
                room_id: data::RoomId::new(room.as_str()),
                page: PageRequest {
                    before: None,
                    max: self.page_size,
                },
            },
            Purpose::Thread {
                room: room.clone(),
                generation,
            },
        );
    }

    /// Reduce up to `max` drained runtime events into `state`. Returns the
    /// number of events processed.
    pub(crate) fn drain(&mut self, state: &mut AppState, max: usize) -> usize {
        let events = self.endpoint.drain(max);
        let count = events.len();
        for event in events {
            match event {
                PumpEvent::Phase(phase) => self.status.phase = Some(phase),
                PumpEvent::Lifecycle {
                    projection,
                    effects,
                    ..
                } => {
                    self.status.projection = projection;
                    self.apply_effects(state, effects);
                }
                PumpEvent::Snapshot { install, snapshot } => {
                    self.install_snapshot(state, snapshot);
                    if self.endpoint.ack(install) {
                        self.control.wake();
                    }
                    for room in state.cache.canonical_rooms() {
                        self.request_newest(&room);
                    }
                }
                PumpEvent::Read { terminal, .. } => self.apply_read(state, terminal),
            }
        }
        if self.endpoint.is_closed() {
            self.status.closed = true;
            self.pending.clear();
            self.deferred.clear();
        } else {
            self.flush_deferred();
        }
        count
    }

    fn generation(&self, room: &RoomId) -> u64 {
        self.generations.get(room).copied().unwrap_or(0)
    }

    fn install_snapshot(&mut self, state: &mut AppState, snapshot: BridgeSnapshot) {
        for room in state.cache.canonical_rooms() {
            if let Some(view) = state.cache.canonical_view_mut(&room) {
                view.reset_for_new_instance();
            }
            *self.generations.entry(room.clone()).or_insert(0) += 1;
            self.cursors.remove(&room);
        }
        if let Some(account) = snapshot.account {
            state.cache.set_me(account);
        }
        self.replace_rooms(state, &snapshot.rooms);
    }

    fn replace_rooms(&mut self, state: &mut AppState, rooms: &[RoomSnapshot]) {
        let listed: HashSet<&str> = rooms.iter().map(|room| room.id.as_str()).collect();
        let stale: Vec<RoomId> = state
            .cache
            .rooms
            .sorted_rooms()
            .iter()
            .filter(|room| !listed.contains(room.id.as_str()))
            .map(|room| room.id.clone())
            .collect();
        for room in stale {
            state.cache.remove_room(&room);
        }
        for snapshot in rooms {
            if let Ok(room) = Room::from_bridge_snapshot(snapshot) {
                state.cache.rooms.update_with_room(&room);
            }
        }
    }

    fn apply_effects(&mut self, state: &mut AppState, effects: ProjectionEffects) {
        for key in &effects.redact {
            let room = key.room_id.as_str().to_owned();
            if let Some(view) = loaded_view(state, &room) {
                let _ = view.tombstone(&key.room_id, &key.message_id, None);
            }
        }
        if let Some(membership) = &effects.membership {
            if membership.is_member == Some(false) {
                state
                    .cache
                    .remove_room(&membership.room_id.as_str().to_owned());
            }
        }
        for key in &effects.created {
            self.request_created(state, key);
        }
        for target in effects.refresh {
            match target {
                RefreshTarget::Status => {}
                RefreshTarget::Rooms => self.request(FrontendIntent::ListAllRooms, Purpose::Rooms),
                RefreshTarget::Room(id) => self.request(
                    FrontendIntent::UpdateRoom(id.clone()),
                    Purpose::Room(id.as_str().to_owned()),
                ),
                RefreshTarget::Message { key, .. } => {
                    let room = key.room_id.as_str().to_owned();
                    if state.cache.canonical_view(&room).is_some() {
                        self.request_message(&room, &key.message_id, false);
                    }
                }
                RefreshTarget::LoadedMessages(ChangeScope::Room(id)) => {
                    let room = id.as_str().to_owned();
                    if state.cache.canonical_view(&room).is_some() {
                        self.request_newest(&room);
                    }
                }
                // Covers messages created in unloaded rooms only via Rooms and
                // the unread path; loaded rooms refetch their newest page.
                RefreshTarget::LoadedMessages(ChangeScope::WholeMirror) => {
                    for room in state.cache.canonical_rooms() {
                        self.request_newest(&room);
                    }
                }
            }
        }
    }

    fn request_created(&mut self, state: &AppState, key: &MessageKey) {
        let room = key.room_id.as_str().to_owned();
        let active = state.is_active_room(&room);
        let loaded = state.cache.canonical_view(&room).is_some();
        if !active || loaded {
            // Inactive rooms need the sender for the unread decision; loaded
            // rooms need the canonical message itself.
            self.request_message(&room, &key.message_id, !active);
        }
    }

    fn request_newest(&mut self, room: &RoomId) {
        let generation = self.generation(room);
        self.request(
            FrontendIntent::ListMessagesInRoom {
                room_id: data::RoomId::new(room.as_str()),
                page: PageRequest {
                    before: None,
                    max: self.page_size,
                },
            },
            Purpose::NewestPage {
                room: room.clone(),
                generation,
            },
        );
    }

    fn request_message(&mut self, room: &RoomId, id: &MessageId, unread_check: bool) {
        let generation = self.generation(room);
        self.request(
            FrontendIntent::UpdateMessage {
                message_id: id.clone(),
                room_id: data::RoomId::new(room.as_str()),
            },
            Purpose::Message {
                room: room.clone(),
                generation,
                unread_check,
            },
        );
    }

    fn request(&mut self, intent: FrontendIntent, purpose: Purpose) {
        let Ok(request) = self.sequence.begin(intent.clone(), None) else {
            return;
        };
        let id = request.context.id;
        match self.mailbox.try_read(request) {
            ReadAdmission::Accepted => {
                self.pending.insert(id, (intent, purpose));
                self.control.wake();
            }
            // One terminal answers both: keep the newest purpose (and room
            // generation) on the existing request.
            ReadAdmission::Duplicate(existing) => {
                if let Some(entry) = self.pending.get_mut(&existing.id) {
                    entry.1 = merge_purpose(&entry.1, purpose);
                }
            }
            ReadAdmission::Full => {
                if !self.deferred.iter().any(|(queued, _)| *queued == intent) {
                    self.deferred.push_back((intent, purpose));
                }
            }
            ReadAdmission::Closed => self.status.closed = true,
            ReadAdmission::InvalidRequest => {}
        }
    }

    fn flush_deferred(&mut self) {
        let deferred = std::mem::take(&mut self.deferred);
        for (intent, purpose) in deferred {
            self.request(intent, purpose);
        }
    }

    fn apply_read(&mut self, state: &mut AppState, terminal: ReadTerminal) {
        let Some((intent, purpose)) = self.pending.remove(&terminal.context().id) else {
            return; // not ours, or already settled
        };
        let envelope = match terminal {
            ReadTerminal::Result(envelope) => envelope,
            ReadTerminal::Unavailable { reason, .. } => {
                match reason {
                    // From before a newer snapshot: re-request once if still
                    // relevant; the fresh request is current by construction.
                    ReadUnavailable::Stale if self.relevant(state, &purpose) => {
                        self.request(intent, self.refreshed(purpose));
                    }
                    _ => {
                        if let Some(room) = page_room(&purpose) {
                            self.status.failed_rooms.insert(room.clone());
                        }
                    }
                }
                return;
            }
        };
        // A result issued before a room reset must never apply after it.
        if let (Some(room), Some(generation)) = (purpose.room(), purpose.generation()) {
            if generation != self.generation(room) {
                return;
            }
        }
        match (purpose, envelope.result) {
            (Purpose::Rooms, FrontendResult::ListAllRooms(outcome)) => {
                if let Some(rooms) = read_value(outcome) {
                    self.replace_rooms(state, &rooms);
                }
            }
            (Purpose::Room(_), FrontendResult::UpdateRoom(outcome)) => {
                if let Some(snapshot) = read_value(outcome) {
                    if let Ok(room) = Room::from_bridge_snapshot(&snapshot) {
                        state.cache.rooms.update_with_room(&room);
                    }
                }
            }
            (Purpose::Person(_), FrontendResult::UpdatePerson(outcome)) => {
                if let Some(person) = read_value(outcome) {
                    state.cache.persons.insert(person);
                }
            }
            (
                purpose @ (Purpose::NewestPage { .. } | Purpose::OlderPage { .. }),
                FrontendResult::ListMessagesInRoom(outcome),
            ) => {
                let room = page_room(&purpose).cloned().expect("page purpose");
                match read_value(outcome) {
                    Some(page) => {
                        self.status.failed_rooms.remove(&room);
                        self.apply_page(state, &room, &page, purpose);
                    }
                    None => {
                        self.status.failed_rooms.insert(room);
                    }
                }
            }
            (
                Purpose::Message {
                    room, unread_check, ..
                },
                FrontendResult::UpdateMessage(outcome),
            ) => {
                if let Some(message) = read_value(outcome) {
                    if let Some(view) = loaded_view(state, &room) {
                        let _ = view.upsert(&message);
                    }
                    let from_me = state
                        .cache
                        .is_me(&message.sender_id.as_ref().map(|id| id.as_str().to_owned()));
                    if unread_check
                        && !from_me
                        && message.status == data::MessageStatus::Active
                        && !state.is_active_room(&room)
                    {
                        state.cache.rooms.mark_unread(&room);
                    }
                }
            }
            (Purpose::Thread { room, .. }, FrontendResult::UpdateChildrenMessages(outcome)) => {
                if let Some(page) = read_value(outcome) {
                    if let Some(view) = loaded_view(state, &room) {
                        let _ = view.apply_thread_page(&page);
                    }
                }
            }
            _ => {} // operation mismatch cannot happen for our own requests
        }
    }

    fn apply_page(
        &mut self,
        state: &mut AppState,
        room: &RoomId,
        page: &MessagePage,
        purpose: Purpose,
    ) {
        let Some(view) = loaded_view(state, room) else {
            return;
        };
        if view.apply_page(page).is_err() {
            self.status.failed_rooms.insert(room.clone());
            return;
        }
        let senders: Vec<PersonId> = page
            .messages
            .iter()
            .filter_map(|message| message.sender_id.clone())
            .collect();
        // The oldest loaded cursor moves only with explicit older pages, or is
        // initialised by the first newest page.
        let first = !self.cursors.contains_key(room);
        if first || matches!(purpose, Purpose::OlderPage { .. }) {
            self.cursors.insert(room.clone(), page.next_cursor.clone());
        }
        for sender in senders {
            let id = sender.as_str().to_owned();
            if !state.cache.persons.exists_or_requested(&id) {
                state.cache.persons.add_requested(&id);
                self.request(FrontendIntent::UpdatePerson(sender), Purpose::Person(id));
            }
        }
    }

    fn relevant(&self, state: &AppState, purpose: &Purpose) -> bool {
        match purpose.room() {
            Some(room) => state.cache.canonical_view(room).is_some(),
            None => true,
        }
    }

    /// The same purpose under the room's current generation.
    fn refreshed(&self, purpose: Purpose) -> Purpose {
        match purpose {
            Purpose::NewestPage { room, .. } => Purpose::NewestPage {
                generation: self.generation(&room),
                room,
            },
            Purpose::OlderPage { room, .. } => Purpose::OlderPage {
                generation: self.generation(&room),
                room,
            },
            Purpose::Message {
                room, unread_check, ..
            } => Purpose::Message {
                generation: self.generation(&room),
                room,
                unread_check,
            },
            Purpose::Thread { room, .. } => Purpose::Thread {
                generation: self.generation(&room),
                room,
            },
            other => other,
        }
    }
}

fn loaded_view<'a>(state: &'a mut AppState, room: &RoomId) -> Option<&'a mut RoomMessageView> {
    state.cache.canonical_view(room)?;
    state.cache.canonical_view_mut(room)
}

fn page_room(purpose: &Purpose) -> Option<&RoomId> {
    match purpose {
        Purpose::NewestPage { room, .. } | Purpose::OlderPage { room, .. } => Some(room),
        _ => None,
    }
}

/// A duplicate keeps the newest generation and the stronger unread intent.
fn merge_purpose(existing: &Purpose, newer: Purpose) -> Purpose {
    match (existing, newer) {
        (
            Purpose::Message {
                unread_check: old, ..
            },
            Purpose::Message {
                room,
                generation,
                unread_check,
            },
        ) => Purpose::Message {
            room,
            generation,
            unread_check: *old || unread_check,
        },
        (_, newer) => newer,
    }
}

/// Success and partial reads both carry honest data; rejection carries none.
fn read_value<T>(outcome: ReadOutcome<T>) -> Option<T> {
    match outcome {
        ReadOutcome::Success(value) | ReadOutcome::Partial { value, .. } => Some(value),
        ReadOutcome::Rejected(_) => None,
    }
}

#[cfg(test)]
mod tests;
