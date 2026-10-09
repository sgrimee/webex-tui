//! Pure lifecycle projection and scoped view-refresh hints.
//!
//! This reducer owns no selection, cache, network, queue, write or read-receipt
//! state. Refresh hints are not commands: the future host must coalesce/bound
//! work, fence snapshot/live races and preserve request generation independently.

use super::{
    data::RoomId,
    events::{
        ChangeScope, ConnectionLayer, ConnectionState, LifecycleEvent, MembershipChange,
        MessageChange, MessageKey, ResyncReason, SyncProgress,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RefreshTarget {
    Status,
    Rooms,
    Room(RoomId),
    Message {
        key: MessageKey,
        /// Preserve the invalidation watermark without inventing canonical data.
        min_revision: Option<u64>,
    },
    /// Reconcile every already-loaded ID/tombstone in this scope, not just the
    /// newest page. This does not request a whole-account message download.
    LoadedMessages(ChangeScope),
}

#[derive(Default, PartialEq, Eq)]
pub(crate) struct ProjectionEffects {
    pub(crate) refresh: Vec<RefreshTarget>,
    /// Only an explicit committed message-delete event permits immediate body
    /// redaction. Room reconciliation must fetch canonical tombstones instead.
    pub(crate) redact: Vec<MessageKey>,
    pub(crate) membership: Option<MembershipChange>,
    pub(crate) resync: Option<(ChangeScope, ResyncReason)>,
}

impl ProjectionEffects {
    /// Scope hint, not permission to change focus or suppress another room's
    /// cache updates. Global refresh affects any current room.
    pub(crate) fn affects_room(&self, room_id: &RoomId) -> bool {
        self.refresh.iter().any(|target| match target {
            RefreshTarget::Status => false,
            RefreshTarget::Rooms | RefreshTarget::LoadedMessages(ChangeScope::WholeMirror) => true,
            RefreshTarget::Room(id) | RefreshTarget::LoadedMessages(ChangeScope::Room(id)) => {
                id == room_id
            }
            RefreshTarget::Message { key, .. } => &key.room_id == room_id,
        })
    }

    fn resync(scope: ChangeScope, reason: ResyncReason) -> Self {
        let mut refresh = vec![RefreshTarget::LoadedMessages(scope.clone())];
        match &scope {
            ChangeScope::WholeMirror => {
                refresh.push(RefreshTarget::Status);
                refresh.push(RefreshTarget::Rooms);
            }
            ChangeScope::Room(room_id) => refresh.push(RefreshTarget::Room(room_id.clone())),
        }
        Self {
            refresh,
            resync: Some((scope, reason)),
            ..Default::default()
        }
    }
}

#[derive(Default)]
pub(crate) struct LifecycleProjection {
    pub(crate) local_daemon: ConnectionState,
    pub(crate) upstream_webex: ConnectionState,
    pub(crate) sync: SyncProgress,
}

impl LifecycleProjection {
    pub(crate) fn apply(&mut self, event: LifecycleEvent) -> ProjectionEffects {
        match event {
            LifecycleEvent::Connection { layer, state } => {
                let connection = match layer {
                    ConnectionLayer::LocalDaemon => &mut self.local_daemon,
                    ConnectionLayer::UpstreamWebex => &mut self.upstream_webex,
                };
                let previous = *connection;
                *connection = state;
                if state == ConnectionState::Connected
                    && previous != ConnectionState::Connected
                    && (layer == ConnectionLayer::LocalDaemon
                        || previous != ConnectionState::Unknown)
                {
                    ProjectionEffects::resync(
                        ChangeScope::WholeMirror,
                        ResyncReason::StreamReconnect,
                    )
                } else {
                    ProjectionEffects::default()
                }
            }
            LifecycleEvent::Sync(update) => {
                macro_rules! merge_count {
                    ($field:ident) => {
                        if update.$field.is_some() {
                            self.sync.$field = update.$field;
                        }
                    };
                }
                merge_count!(rooms);
                merge_count!(revisions);
                merge_count!(initial_pending);
                merge_count!(gaps);
                merge_count!(failures);
                // Progress counters aren't content invalidations. B7 supplies
                // explicit room/global invalidations for committed mirror data.
                ProjectionEffects::default()
            }
            LifecycleEvent::RoomInvalidated(room_id) => ProjectionEffects {
                refresh: vec![RefreshTarget::Room(room_id)],
                ..Default::default()
            },
            LifecycleEvent::MessageInvalidated {
                key,
                change,
                revision,
            } => {
                let redact = if change == MessageChange::Deleted {
                    vec![key.clone()]
                } else {
                    Vec::new()
                };
                ProjectionEffects {
                    refresh: vec![
                        RefreshTarget::Message {
                            key: key.clone(),
                            min_revision: revision,
                        },
                        RefreshTarget::Room(key.room_id),
                    ],
                    redact,
                    ..Default::default()
                }
            }
            LifecycleEvent::MembershipInvalidated(change) => ProjectionEffects {
                refresh: vec![
                    RefreshTarget::Rooms,
                    RefreshTarget::Room(change.room_id.clone()),
                ],
                membership: Some(change),
                ..Default::default()
            },
            LifecycleEvent::ResyncRequired { scope, reason } => {
                if reason == ResyncReason::DaemonRestart {
                    // Even counts cannot be carried across an instance boundary
                    // as if they were fresh. Connection state is learned separately.
                    self.sync = SyncProgress::default();
                }
                ProjectionEffects::resync(scope, reason)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::{data::MessageId, requests::RoomViewRequests};

    fn room() -> RoomId {
        RoomId::new("opaque/room?not-a-path")
    }
    fn key() -> MessageKey {
        MessageKey {
            room_id: room(),
            message_id: MessageId::new("opaque/message?not-a-path"),
        }
    }
    fn connection(layer: ConnectionLayer, state: ConnectionState) -> LifecycleEvent {
        LifecycleEvent::Connection { layer, state }
    }

    #[test]
    fn local_and_upstream_connection_states_are_independent() {
        let mut projection = LifecycleProjection::default();
        projection.apply(connection(
            ConnectionLayer::LocalDaemon,
            ConnectionState::Connected,
        ));
        projection.apply(connection(
            ConnectionLayer::UpstreamWebex,
            ConnectionState::Disconnected,
        ));
        assert_eq!(projection.local_daemon, ConnectionState::Connected);
        assert_eq!(projection.upstream_webex, ConnectionState::Disconnected);
        projection.apply(connection(
            ConnectionLayer::UpstreamWebex,
            ConnectionState::AuthRequired,
        ));
        assert_eq!(projection.local_daemon, ConnectionState::Connected);
        assert_eq!(projection.upstream_webex, ConnectionState::AuthRequired);
    }

    #[test]
    fn daemon_loss_does_not_invent_upstream_logout_or_clear_available_counts() {
        let mut projection = LifecycleProjection::default();
        projection.apply(connection(
            ConnectionLayer::UpstreamWebex,
            ConnectionState::Connected,
        ));
        projection.apply(LifecycleEvent::Sync(SyncProgress {
            rooms: Some(12),
            ..Default::default()
        }));
        let effects = projection.apply(connection(
            ConnectionLayer::LocalDaemon,
            ConnectionState::Disconnected,
        ));
        assert_eq!(projection.upstream_webex, ConnectionState::Connected);
        assert_eq!(projection.sync.rooms, Some(12));
        assert!(effects.refresh.is_empty());
    }

    #[test]
    fn sparse_progress_preserves_known_counts_and_explicit_zero() {
        let mut projection = LifecycleProjection::default();
        projection.apply(LifecycleEvent::Sync(SyncProgress {
            rooms: Some(12),
            gaps: Some(2),
            ..Default::default()
        }));
        let effects = projection.apply(LifecycleEvent::Sync(SyncProgress {
            gaps: Some(0),
            initial_pending: Some(1),
            ..Default::default()
        }));
        assert_eq!(projection.sync.rooms, Some(12));
        assert_eq!(projection.sync.gaps, Some(0));
        assert_eq!(projection.sync.initial_pending, Some(1));
        assert_eq!(projection.sync.revisions, None);
        assert!(effects.refresh.is_empty());
    }

    #[test]
    fn room_invalidation_is_correlated_by_opaque_id() {
        let effects = LifecycleProjection::default().apply(LifecycleEvent::RoomInvalidated(room()));
        assert_eq!(effects.refresh, [RefreshTarget::Room(room())]);
        assert!(effects.affects_room(&room()));
        assert!(!effects.affects_room(&RoomId::new("other-room")));
    }

    #[test]
    fn create_and_edit_refresh_one_canonical_message_without_redaction() {
        for change in [MessageChange::Created, MessageChange::Updated] {
            let effects =
                LifecycleProjection::default().apply(LifecycleEvent::MessageInvalidated {
                    key: key(),
                    change,
                    revision: Some(4),
                });
            assert_eq!(
                effects.refresh,
                [
                    RefreshTarget::Message {
                        key: key(),
                        min_revision: Some(4)
                    },
                    RefreshTarget::Room(room())
                ]
            );
            assert!(effects.redact.is_empty());
            assert!(!effects.affects_room(&RoomId::new("other-room")));
        }
    }

    #[test]
    fn delete_redacts_only_the_confirmed_message_not_its_room_or_replies() {
        let effects = LifecycleProjection::default().apply(LifecycleEvent::MessageInvalidated {
            key: key(),
            change: MessageChange::Deleted,
            revision: None,
        });
        assert_eq!(effects.redact, [key()]);
        assert_eq!(
            effects.refresh,
            [
                RefreshTarget::Message {
                    key: key(),
                    min_revision: None
                },
                RefreshTarget::Room(room())
            ]
        );
        assert!(effects.resync.is_none());
    }

    #[test]
    fn membership_unknown_does_not_become_departure() {
        let effects = LifecycleProjection::default().apply(LifecycleEvent::MembershipInvalidated(
            MembershipChange {
                room_id: room(),
                is_member: None,
            },
        ));
        assert_eq!(effects.membership.unwrap().is_member, None);
        assert_eq!(
            effects.refresh,
            [RefreshTarget::Rooms, RefreshTarget::Room(room())]
        );
    }

    #[test]
    fn membership_preserves_confirmed_join_or_departure_without_selection_mutation() {
        for is_member in [true, false] {
            let change = MembershipChange {
                room_id: room(),
                is_member: Some(is_member),
            };
            let effects = LifecycleProjection::default()
                .apply(LifecycleEvent::MembershipInvalidated(change.clone()));
            assert_eq!(effects.membership, Some(change));
            assert!(effects.redact.is_empty());
        }
    }

    #[test]
    fn room_resync_targets_loaded_history_not_only_recent_page() {
        let effects = LifecycleProjection::default().apply(LifecycleEvent::ResyncRequired {
            scope: ChangeScope::Room(room()),
            reason: ResyncReason::Reconciliation,
        });
        assert_eq!(
            effects.refresh,
            [
                RefreshTarget::LoadedMessages(ChangeScope::Room(room())),
                RefreshTarget::Room(room())
            ]
        );
        assert_eq!(
            effects.resync,
            Some((ChangeScope::Room(room()), ResyncReason::Reconciliation))
        );
        assert!(!effects.affects_room(&RoomId::new("other-room")));
        assert!(effects.redact.is_empty());
    }

    #[test]
    fn global_lag_resync_includes_status_rooms_and_all_loaded_ids() {
        let effects = LifecycleProjection::default().apply(LifecycleEvent::ResyncRequired {
            scope: ChangeScope::WholeMirror,
            reason: ResyncReason::EventLag,
        });
        assert_eq!(
            effects.refresh,
            [
                RefreshTarget::LoadedMessages(ChangeScope::WholeMirror),
                RefreshTarget::Status,
                RefreshTarget::Rooms
            ]
        );
        assert!(effects.affects_room(&room()));
        assert!(effects.affects_room(&RoomId::new("other-room")));
    }

    #[test]
    fn daemon_restart_clears_stale_counts_but_does_not_guess_connection_state() {
        let mut projection = LifecycleProjection::default();
        projection.sync.rooms = Some(12);
        projection.local_daemon = ConnectionState::Connected;
        let effects = projection.apply(LifecycleEvent::ResyncRequired {
            scope: ChangeScope::WholeMirror,
            reason: ResyncReason::DaemonRestart,
        });
        assert_eq!(projection.sync, SyncProgress::default());
        assert_eq!(projection.local_daemon, ConnectionState::Connected);
        assert_eq!(
            effects.resync,
            Some((ChangeScope::WholeMirror, ResyncReason::DaemonRestart))
        );
    }

    #[test]
    fn reconnect_resyncs_but_duplicate_connected_status_does_not_refetch() {
        let mut projection = LifecycleProjection::default();
        projection.apply(connection(
            ConnectionLayer::LocalDaemon,
            ConnectionState::Disconnected,
        ));
        let reconnect = projection.apply(connection(
            ConnectionLayer::LocalDaemon,
            ConnectionState::Connected,
        ));
        assert_eq!(
            reconnect.resync,
            Some((ChangeScope::WholeMirror, ResyncReason::StreamReconnect))
        );
        let repeated = projection.apply(connection(
            ConnectionLayer::LocalDaemon,
            ConnectionState::Connected,
        ));
        assert!(repeated.refresh.is_empty());
        projection.apply(connection(
            ConnectionLayer::UpstreamWebex,
            ConnectionState::Disconnected,
        ));
        let upstream = projection.apply(connection(
            ConnectionLayer::UpstreamWebex,
            ConnectionState::Connected,
        ));
        assert_eq!(
            upstream.resync,
            Some((ChangeScope::WholeMirror, ResyncReason::StreamReconnect))
        );
    }

    #[test]
    fn lifecycle_events_do_not_change_existing_room_request_generation() {
        let mut requests = RoomViewRequests::default();
        requests.select_room(Some(room().as_str().into())).unwrap();
        let context = requests.start_read().unwrap().unwrap();
        let mut projection = LifecycleProjection::default();
        projection.apply(LifecycleEvent::ResyncRequired {
            scope: ChangeScope::WholeMirror,
            reason: ResyncReason::SnapshotBoundary,
        });
        projection.apply(LifecycleEvent::MessageInvalidated {
            key: key(),
            change: MessageChange::Updated,
            revision: Some(4),
        });
        assert!(requests.is_current(&context));
        // Neither events nor their hints can construct/discard/retry a write.
    }
}
