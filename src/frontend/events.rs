//! Typed metadata-only lifecycle/change inputs for a future bridge adapter.
//!
//! This is not an SSE parser or wire protocol. The adapter must validate event
//! names, resource IDs and scope before constructing these inputs. Remote bodies,
//! URLs, credentials and user-controlled actions have no representation here.

use super::data::{MessageId, RoomId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ConnectionState {
    #[default]
    Unknown,
    Connecting,
    Connected,
    Disconnected,
    AuthRequired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnectionLayer {
    LocalDaemon,
    UpstreamWebex,
}

/// Sparse metadata: an absent count means no new information, not zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SyncProgress {
    pub(crate) rooms: Option<u64>,
    pub(crate) revisions: Option<u64>,
    pub(crate) initial_pending: Option<u64>,
    pub(crate) gaps: Option<u64>,
    pub(crate) failures: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChangeScope {
    WholeMirror,
    Room(RoomId),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MessageKey {
    pub(crate) room_id: RoomId,
    pub(crate) message_id: MessageId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MessageChange {
    Created,
    Updated,
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResyncReason {
    EventLag,
    DaemonRestart,
    StreamReconnect,
    SnapshotBoundary,
    Sync,
    Reconciliation,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MembershipChange {
    pub(crate) room_id: RoomId,
    /// None is invalidation only, not evidence of a join or departure.
    pub(crate) is_member: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LifecycleEvent {
    Connection {
        layer: ConnectionLayer,
        state: ConnectionState,
    },
    Sync(SyncProgress),
    RoomInvalidated(RoomId),
    MessageInvalidated {
        key: MessageKey,
        change: MessageChange,
        /// An invalidation watermark only; never invent a canonical message.
        revision: Option<u64>,
    },
    MembershipInvalidated(MembershipChange),
    ResyncRequired {
        scope: ChangeScope,
        reason: ResyncReason,
    },
}
