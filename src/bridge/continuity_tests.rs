//! Pure B8 mapping tests against bridge a6761bc's actual event/DTO shapes.

use super::events::{project_continuity, project_ui_event};
use crate::frontend::{
    continuity::{
        ContinuityBoundary, DaemonInstanceId, LifecycleControl, SnapshotRelation, UiLifecycleFrame,
    },
    data::{MessageId, RoomId},
    events::{
        ChangeScope, ConnectionLayer, ConnectionState, LifecycleEvent, MessageChange, MessageKey,
        ResyncReason,
    },
};
use serde_json::{json, Value};
use webex_bridge::{client::events::BridgeEvent, dto};

fn event(name: &str, data: Value) -> BridgeEvent {
    BridgeEvent {
        name: name.into(),
        data,
    }
}

fn boundary(instance: &str, watermark: u64) -> ContinuityBoundary {
    ContinuityBoundary {
        instance_id: DaemonInstanceId::new(instance),
        watermark,
    }
}

#[test]
fn ready_maps_top_level_zero_and_max_boundaries_without_parsing_instance_ids() {
    for watermark in [0, u64::MAX] {
        let frame = project_ui_event(&event(
            "continuity.ready",
            json!({"instanceId":"opaque/instance?not-a-URL", "watermark":watermark}),
        ))
        .unwrap();
        assert_eq!(
            frame,
            UiLifecycleFrame::Control(LifecycleControl::Ready(boundary(
                "opaque/instance?not-a-URL",
                watermark
            )))
        );
    }
}

#[test]
fn normal_ui_change_keeps_nested_marker_scope_and_revision_without_extra_content() {
    let frame = project_ui_event(&event(
        "message.updated",
        json!({
            "continuity":{"instanceId":"instance-a","watermark":12},
            "roomId":"room-a","messageId":"message-a","revision":4,
            "body":"untrusted extra body","token":"unexpected extra token"
        }),
    ))
    .unwrap();
    assert_eq!(
        frame,
        UiLifecycleFrame::Change {
            boundary: Some(boundary("instance-a", 12)),
            event: LifecycleEvent::MessageInvalidated {
                key: MessageKey {
                    room_id: RoomId::new("room-a"),
                    message_id: MessageId::new("message-a")
                },
                change: MessageChange::Updated,
                revision: Some(4)
            }
        }
    );
}

#[test]
fn ui_lag_resync_maps_top_level_marker_to_a_control_not_an_unframed_change() {
    assert_eq!(
        project_ui_event(&event(
            "resync.required",
            json!({
                "instanceId":"instance-a","watermark":42,"reason":"event_lag"
            })
        ))
        .unwrap(),
        UiLifecycleFrame::Control(LifecycleControl::ResyncRequired {
            boundary: boundary("instance-a", 42),
            scope: ChangeScope::WholeMirror,
            reason: ResyncReason::EventLag
        })
    );
    // The same agent-scope signal has no UI continuity and must not be accepted
    // as proof of a continuous UI stream.
    assert!(project_ui_event(&event("resync.required", json!({"reason":"event_lag"}))).is_err());
}

#[test]
fn shared_connection_status_is_intentionally_unsequenced_but_normal_ui_changes_are_not() {
    assert_eq!(
        project_ui_event(&event(
            "connection.status",
            json!({"connection":"disconnected"})
        ))
        .unwrap(),
        UiLifecycleFrame::Change {
            boundary: None,
            event: LifecycleEvent::Connection {
                layer: ConnectionLayer::UpstreamWebex,
                state: ConnectionState::Disconnected
            }
        }
    );
    for name in [
        "room.updated",
        "room.resync",
        "message.created",
        "membership.changed",
        "sync.progress",
    ] {
        assert!(project_ui_event(&event(name,json!({"roomId":"room-a","messageId":"message-a","revision":1,"reason":"sync","isMember":true,"rooms":1}))).is_err());
    }
}

#[test]
fn expiry_is_a_terminal_control_without_fabricated_continuity_or_upstream_logout() {
    let frame = project_ui_event(&event(
        "session.expired",
        json!({
            "reason":"session_expired","body":"unexpected remote content","token":"unexpected",
            "continuity":{"malformed":"extra metadata cannot erase terminal expiry"}
        }),
    ))
    .unwrap();
    assert_eq!(
        frame,
        UiLifecycleFrame::Control(LifecycleControl::SessionExpired)
    );
    // There is no watermark to compare with a snapshot, no connection mutation,
    // and no local session renewal/pairing action executed by this mapper.
    assert!(project_ui_event(&event("session.expired", json!({"reason":"wrong_reason"}))).is_err());
    assert!(project_ui_event(&event("session.expired", json!({}))).is_err());
}

#[test]
fn missing_malformed_ambiguous_or_wrongly_shaped_markers_fail_closed() {
    for data in [
        json!({}),
        json!({"instanceId":"","watermark":1}),
        json!({"instanceId":"bad\u{1b}id","watermark":1}),
        json!({"instanceId":"a".repeat(1025),"watermark":1}),
        json!({"instanceId":"instance-a"}),
        json!({"watermark":1}),
        json!({"instanceId":"instance-a","watermark":-1}),
        json!({"instanceId":"instance-a","watermark":"1"}),
        json!({"instanceId":"instance-a","watermark":1.5}),
        json!({"instanceId":"instance-a","watermark":null}),
        json!({"continuity":{"instanceId":"instance-a","watermark":1}}),
    ] {
        assert!(project_ui_event(&event("continuity.ready", data)).is_err());
    }
    assert!(project_ui_event(&event(
        "room.updated",
        json!({"roomId":"room-a","continuity":null})
    ))
    .is_err());
    assert!(project_ui_event(&event(
        "room.updated",
        json!({"roomId":"room-a","instanceId":"instance-a","watermark":1})
    ))
    .is_err());
    assert!(project_ui_event(&event("room.updated",json!({"roomId":"room-a","continuity":{"instanceId":"instance-a","watermark":1},"instanceId":"instance-b","watermark":2}))).is_err());
    // A malformed marker must not fall back to the shared connection exception.
    assert!(project_ui_event(&event(
        "connection.status",
        json!({"connection":"connected","continuity":{"instanceId":"","watermark":1}})
    ))
    .is_err());
}

#[test]
fn unknown_ui_events_preserve_valid_marker_but_never_payload_data() {
    assert_eq!(
        project_ui_event(&event(
            "future.ui.event",
            json!({
                "continuity":{"instanceId":"instance-a","watermark":13},"body":"unexpected body"
            })
        ))
        .unwrap(),
        UiLifecycleFrame::Unrecognized {
            boundary: Some(boundary("instance-a", 13))
        }
    );
    assert_eq!(
        project_ui_event(&event(
            "future.unsequenced",
            json!({"body":"unexpected body"})
        ))
        .unwrap(),
        UiLifecycleFrame::Unrecognized { boundary: None }
    );
    assert!(project_ui_event(&event(
        "future.ui.event",
        json!({"continuity":{"instanceId":"instance-a","watermark":-1}})
    ))
    .is_err());
}

#[test]
fn protected_continuity_dto_conversion_validates_instance_and_retains_watermark() {
    let value = project_continuity(dto::Continuity {
        instance_id: "opaque-instance".into(),
        watermark: 7,
    })
    .unwrap();
    assert_eq!(value.instance_id.as_str(), "opaque-instance");
    assert_eq!(value.watermark, 7);
    assert!(project_continuity(dto::Continuity {
        instance_id: String::new(),
        watermark: 0
    })
    .is_err());
}

#[test]
fn frame_boundaries_compare_only_same_instance_and_do_not_discard_controls() {
    let frame = project_ui_event(&event(
        "continuity.ready",
        json!({"instanceId":"instance-b","watermark":0}),
    ))
    .unwrap();
    let UiLifecycleFrame::Control(LifecycleControl::Ready(marker)) = frame else {
        panic!("ready control lost");
    };
    assert_eq!(
        marker.relative_to(&boundary("instance-a", u64::MAX)),
        SnapshotRelation::DifferentInstance
    );
    assert_eq!(
        marker.relative_to(&boundary("instance-b", 0)),
        SnapshotRelation::Covered
    );
    // Covered is a comparison result; the mapper still returned the Ready control.
}
