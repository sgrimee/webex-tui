//! Compile-only B7 payload and B8 UI continuity/control projection.
//! Whole-frame projection validates markers before exposing ordinary changes;
//! it does not subscribe, maintain a runtime fence or recreate expired sessions.

use super::projection::{connection, valid_id, ProjectionError};
use crate::frontend::{
    continuity::{ContinuityBoundary, DaemonInstanceId, LifecycleControl, UiLifecycleFrame},
    data::{MessageId, RoomId},
    events::{
        ChangeScope, ConnectionLayer, LifecycleEvent, MembershipChange, MessageChange, MessageKey,
        ResyncReason, SyncProgress,
    },
};
use serde_json::Value;
use webex_bridge::{
    client::events::{BridgeEvent, CONTINUITY_READY_EVENT},
    dto,
};

fn string<'a>(data: &'a Value, name: &str) -> Result<&'a str, ProjectionError> {
    data.get(name)
        .and_then(Value::as_str)
        .ok_or(ProjectionError::Metadata)
}

fn room(data: &Value) -> Result<RoomId, ProjectionError> {
    let id = string(data, "roomId")?;
    valid_id(id)?;
    Ok(RoomId::new(id))
}

fn count(data: &Value, name: &str) -> Result<Option<u64>, ProjectionError> {
    match data.get(name) {
        None => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or(ProjectionError::Metadata),
    }
}

// Payload-only helper stays private: UI callers must retain continuity/controls.
fn project_event(event: &BridgeEvent) -> Result<Option<LifecycleEvent>, ProjectionError> {
    let data = &event.data;
    let projected = match event.name.as_str() {
        "connection.status" => LifecycleEvent::Connection {
            layer: ConnectionLayer::UpstreamWebex,
            state: connection(string(data, "connection")?)?,
        },
        "sync.progress" if data.get("roomId").is_some() => LifecycleEvent::ResyncRequired {
            scope: ChangeScope::Room(room(data)?),
            reason: ResyncReason::Sync,
        },
        "sync.progress" => LifecycleEvent::Sync(SyncProgress {
            rooms: count(data, "rooms")?,
            revisions: count(data, "revisions")?,
            // "initial" is a completed-pass count, not initial_pending.
            initial_pending: count(data, "initialPending")?,
            gaps: count(data, "gaps")?,
            failures: count(data, "failed")?,
        }),
        "room.updated" => LifecycleEvent::RoomInvalidated(room(data)?),
        "room.resync" => LifecycleEvent::ResyncRequired {
            scope: ChangeScope::Room(room(data)?),
            reason: match string(data, "reason")? {
                "sync" => ResyncReason::Sync,
                "reconciled" => ResyncReason::Reconciliation,
                _ => ResyncReason::Unknown,
            },
        },
        "message.created" | "message.updated" | "message.deleted" => {
            let message_id = string(data, "messageId")?;
            valid_id(message_id)?;
            LifecycleEvent::MessageInvalidated {
                key: MessageKey {
                    room_id: room(data)?,
                    message_id: MessageId::new(message_id),
                },
                change: match event.name.as_str() {
                    "message.created" => MessageChange::Created,
                    "message.updated" => MessageChange::Updated,
                    _ => MessageChange::Deleted,
                },
                revision: count(data, "revision")?,
            }
        }
        "membership.changed" => LifecycleEvent::MembershipInvalidated(MembershipChange {
            room_id: room(data)?,
            is_member: match data.get("isMember") {
                None => None,
                Some(value) => Some(value.as_bool().ok_or(ProjectionError::Metadata)?),
            },
        }),
        "resync.required" => LifecycleEvent::ResyncRequired {
            // This event invalidates the subscriber's whole UI mirror view.
            scope: ChangeScope::WholeMirror,
            reason: match string(data, "reason")? {
                "event_lag" => ResyncReason::EventLag,
                _ => ResyncReason::Unknown,
            },
        },
        // The whole-frame mapper handles B8 controls before this payload helper.
        _ => return Ok(None),
    };
    Ok(Some(projected))
}

pub(crate) fn project_continuity(
    value: dto::Continuity,
) -> Result<ContinuityBoundary, ProjectionError> {
    valid_id(&value.instance_id)?;
    Ok(ContinuityBoundary {
        instance_id: DaemonInstanceId::new(value.instance_id),
        watermark: value.watermark,
    })
}

fn event_boundary(event: &BridgeEvent) -> Result<Option<ContinuityBoundary>, ProjectionError> {
    let nested = event.data.get("continuity").is_some();
    let top = event.data.get("instanceId").is_some() || event.data.get("watermark").is_some();
    if nested && top {
        return Err(ProjectionError::Metadata);
    }
    if !nested && !top {
        return Ok(None);
    }
    // Ready/lag markers are top-level; normal UI changes carry nested metadata.
    if event.name == CONTINUITY_READY_EVENT && nested {
        return Err(ProjectionError::Metadata);
    }
    if top && event.name != CONTINUITY_READY_EVENT && !event.requires_resync() {
        return Err(ProjectionError::Metadata);
    }
    let value = event.continuity().ok_or(ProjectionError::Metadata)?;
    project_continuity(value).map(Some)
}

/// B8 frame mapping only: controls must be processed by the owning subscription
/// before snapshot/generation filtering. No renewal, reconnect or write occurs.
pub(crate) fn project_ui_event(event: &BridgeEvent) -> Result<UiLifecycleFrame, ProjectionError> {
    if event.is_session_expired() {
        if string(&event.data, "reason")? != "session_expired" {
            return Err(ProjectionError::Metadata);
        }
        // The actual terminal event has no continuity. Do not fabricate a marker
        // or let extra payload data/snapshot filtering erase subscription expiry.
        return Ok(UiLifecycleFrame::Control(LifecycleControl::SessionExpired));
    }
    let boundary = event_boundary(event)?;
    if event.name == CONTINUITY_READY_EVENT {
        return Ok(UiLifecycleFrame::Control(LifecycleControl::Ready(
            boundary.ok_or(ProjectionError::Metadata)?,
        )));
    }
    if event.requires_resync() {
        return Ok(UiLifecycleFrame::Control(
            LifecycleControl::ResyncRequired {
                boundary: boundary.ok_or(ProjectionError::Metadata)?,
                scope: ChangeScope::WholeMirror,
                reason: match string(&event.data, "reason")? {
                    "event_lag" => ResyncReason::EventLag,
                    _ => ResyncReason::Unknown,
                },
            },
        ));
    }
    match project_event(event)? {
        Some(projected) => {
            // publish_both(connection.status) intentionally has no UI watermark.
            // No other known UI invalidation may silently lose its marker.
            if boundary.is_none() && event.name != "connection.status" {
                return Err(ProjectionError::Metadata);
            }
            Ok(UiLifecycleFrame::Change {
                boundary,
                event: projected,
            })
        }
        None => Ok(UiLifecycleFrame::Unrecognized { boundary }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(name: &str, data: Value) -> BridgeEvent {
        BridgeEvent {
            name: name.into(),
            data,
        }
    }

    #[test]
    fn message_event_keeps_scope_and_revision_but_not_unexpected_content() {
        let value = project_event(&event("message.updated", json!({"roomId":"opaque/room?x", "messageId":"message-a", "revision":4, "text":"untrusted extra content", "token":"extra"}))).unwrap().unwrap();
        assert_eq!(
            value,
            LifecycleEvent::MessageInvalidated {
                key: MessageKey {
                    room_id: RoomId::new("opaque/room?x"),
                    message_id: MessageId::new("message-a")
                },
                change: MessageChange::Updated,
                revision: Some(4)
            }
        );
    }

    #[test]
    fn room_reconciliation_does_not_invent_a_deleted_message() {
        assert_eq!(
            project_event(&event(
                "room.resync",
                json!({"roomId":"room-a","reason":"reconciled"})
            ))
            .unwrap(),
            Some(LifecycleEvent::ResyncRequired {
                scope: ChangeScope::Room(RoomId::new("room-a")),
                reason: ResyncReason::Reconciliation
            })
        );
    }

    #[test]
    fn malformed_known_metadata_and_negative_revisions_are_rejected() {
        assert!(project_event(&event(
            "message.created",
            json!({"roomId":"room-a","messageId":"m","revision":-1})
        ))
        .is_err());
        assert!(project_event(&event(
            "membership.changed",
            json!({"roomId":"room-a","isMember":"false"})
        ))
        .is_err());
        assert!(project_event(&event("room.updated", json!({"roomId":"room\u{1b}"}))).is_err());
    }

    #[test]
    fn sync_completed_count_is_not_initial_pending_and_lazy_history_is_scoped() {
        assert_eq!(
            project_event(&event(
                "sync.progress",
                json!({"rooms":12,"initial":7,"gaps":0,"failed":2})
            ))
            .unwrap(),
            Some(LifecycleEvent::Sync(SyncProgress {
                rooms: Some(12),
                gaps: Some(0),
                failures: Some(2),
                ..Default::default()
            }))
        );
        assert_eq!(
            project_event(&event(
                "sync.progress",
                json!({"roomId":"room-a","lazyHistory":true})
            ))
            .unwrap(),
            Some(LifecycleEvent::ResyncRequired {
                scope: ChangeScope::Room(RoomId::new("room-a")),
                reason: ResyncReason::Sync
            })
        );
    }

    #[test]
    fn lag_maps_to_global_resync_but_unknown_and_b8_controls_are_not_applied() {
        assert_eq!(
            project_event(&event("resync.required", json!({"reason":"event_lag"}))).unwrap(),
            Some(LifecycleEvent::ResyncRequired {
                scope: ChangeScope::WholeMirror,
                reason: ResyncReason::EventLag
            })
        );
        assert_eq!(
            project_event(&event(
                "continuity.ready",
                json!({"instanceId":"opaque","watermark":2})
            ))
            .unwrap(),
            None
        );
        assert_eq!(
            project_event(&event("unknown.name", json!({"text":"extra content"}))).unwrap(),
            None
        );
    }
}
