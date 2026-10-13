//! Fake-only pump tests: no runtime, socket, daemon, Keychain or terminal.
use super::*;
use crate::{
    bridge::{
        commands::{CommandAdmission, CommandWorker},
        session::{SessionBackend, SessionClock, SessionFailure, SessionLifecycle},
    },
    frontend::{
        continuity::{ContinuityBoundary, DaemonInstanceId, LifecycleControl, UiLifecycleFrame},
        data::{ComposedBody, MessageId, RoomId},
        editor_submission::{
            DraftRevision, DraftSnapshot, DraftStamp, DraftTarget, EditorSubmission,
        },
        events::{MembershipChange, MessageKey},
    },
};
use std::time::Instant;

fn effects(refresh: Vec<RefreshTarget>) -> ProjectionEffects {
    ProjectionEffects {
        refresh,
        ..Default::default()
    }
}
fn room(id: &str) -> RoomId {
    RoomId::new(id)
}
fn key(id: &str) -> MessageKey {
    MessageKey {
        room_id: room("room"),
        message_id: MessageId::new(id),
    }
}
fn membership(id: &str, is_member: bool) -> ProjectionEffects {
    ProjectionEffects {
        refresh: vec![RefreshTarget::Rooms],
        membership: Some(MembershipChange {
            room_id: room(id),
            is_member: Some(is_member),
        }),
        ..Default::default()
    }
}
fn kinds<S>(events: &[PumpEvent<S>]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event {
            PumpEvent::Phase(_) => "phase",
            PumpEvent::Lifecycle { .. } => "lifecycle",
            PumpEvent::Snapshot { .. } => "snapshot",
        })
        .collect()
}

#[test]
fn preserves_production_order_and_coalesces_adjacent_updates() {
    let (mut host, endpoint) = pump::<&'static str>();
    let projection = LifecycleProjection::default();
    host.session_phase(SessionPhase::Connecting);
    host.session_phase(SessionPhase::AwaitingReady); // coalesces
    host.lifecycle(&projection, effects(vec![RefreshTarget::Status]));
    host.lifecycle(
        &projection,
        effects(vec![RefreshTarget::Status, RefreshTarget::Room(room("a"))]),
    ); // merges, dedup
    host.install_snapshot("snapshot-1");
    host.session_phase(SessionPhase::Active);
    host.lifecycle(&projection, effects(vec![RefreshTarget::Room(room("b"))]));
    let events = endpoint.drain(usize::MAX);
    assert_eq!(
        kinds(&events),
        ["phase", "lifecycle", "snapshot", "phase", "lifecycle"]
    );
    assert!(matches!(
        events[0],
        PumpEvent::Phase(SessionPhase::AwaitingReady)
    ));
    let PumpEvent::Lifecycle { effects, .. } = &events[1] else {
        panic!("lifecycle")
    };
    assert_eq!(
        effects.refresh,
        [RefreshTarget::Status, RefreshTarget::Room(room("a"))]
    );
    // Post-boundary metadata follows its snapshot, never precedes it.
    assert!(matches!(
        events[2],
        PumpEvent::Snapshot {
            snapshot: "snapshot-1",
            ..
        }
    ));
    assert!(endpoint.drain(usize::MAX).is_empty());
}

#[test]
fn drain_is_bounded_and_resumes_in_order() {
    let (mut host, endpoint) = pump::<()>();
    let projection = LifecycleProjection::default();
    for id in ["a", "b", "c"] {
        host.lifecycle(&projection, membership(id, false));
    }
    assert_eq!(endpoint.drain(2).len(), 2);
    let rest = endpoint.drain(2);
    assert_eq!(rest.len(), 1);
    let PumpEvent::Lifecycle { effects, .. } = &rest[0] else {
        panic!("lifecycle")
    };
    assert_eq!(effects.membership.as_ref().unwrap().room_id, room("c"));
}

#[test]
fn conflicting_membership_stays_separate_and_overflow_upgrades_to_lag_resync() {
    let (mut host, endpoint) = pump::<()>();
    let projection = LifecycleProjection::default();
    // Distinct memberships cannot merge, so each takes one entry up to the cap.
    for index in 0..LIFECYCLE_CAP + 10 {
        let mut hint = membership(&format!("room-{index}"), true);
        hint.redact.push(key(&format!("deleted-{index}")));
        host.lifecycle(&projection, hint);
    }
    let events = endpoint.drain(usize::MAX);
    assert_eq!(events.len(), LIFECYCLE_CAP);
    let PumpEvent::Lifecycle { effects, .. } = events.last().unwrap() else {
        panic!("lifecycle")
    };
    assert_eq!(
        effects.resync,
        Some((ChangeScope::WholeMirror, ResyncReason::EventLag))
    );
    assert!(effects.refresh.contains(&RefreshTarget::Rooms));
    assert!(effects
        .refresh
        .contains(&RefreshTarget::LoadedMessages(ChangeScope::WholeMirror)));
    // Every redaction key is retained across overflow merges.
    let mut redacted: Vec<_> = events
        .iter()
        .flat_map(|event| match event {
            PumpEvent::Lifecycle { effects, .. } => effects.redact.clone(),
            _ => Vec::new(),
        })
        .collect();
    redacted.dedup();
    assert_eq!(redacted.len(), LIFECYCLE_CAP + 10);
}

#[test]
fn differing_resync_scopes_merge_to_whole_mirror() {
    let (mut host, endpoint) = pump::<()>();
    let projection = LifecycleProjection::default();
    let room_resync = ProjectionEffects {
        resync: Some((ChangeScope::Room(room("a")), ResyncReason::Reconciliation)),
        ..Default::default()
    };
    let other = ProjectionEffects {
        resync: Some((ChangeScope::Room(room("b")), ResyncReason::Reconciliation)),
        ..Default::default()
    };
    host.lifecycle(&projection, room_resync);
    host.lifecycle(&projection, other);
    let events = endpoint.drain(usize::MAX);
    let PumpEvent::Lifecycle { effects, .. } = &events[0] else {
        panic!("lifecycle")
    };
    assert_eq!(
        effects.resync,
        Some((ChangeScope::WholeMirror, ResyncReason::Reconciliation))
    );
    assert!(effects.refresh.contains(&RefreshTarget::Rooms));
}

#[test]
fn only_the_latest_drained_snapshot_can_be_acknowledged() {
    let (mut host, endpoint) = pump::<u32>();
    assert!(host.snapshot_acknowledged(), "no snapshot yet");
    host.install_snapshot(1);
    assert!(!host.snapshot_acknowledged());
    host.install_snapshot(2); // supersedes the undrained first snapshot
    let events = endpoint.drain(usize::MAX);
    assert_eq!(events.len(), 1);
    let PumpEvent::Snapshot { install, snapshot } = &events[0] else {
        panic!("snapshot")
    };
    assert_eq!(*snapshot, 2);
    let latest = *install;
    assert!(!endpoint.ack(InstallId(1)), "superseded install");
    assert!(endpoint.ack(latest));
    assert!(host.snapshot_acknowledged());
    // A newer install revokes until drained and acknowledged again.
    host.install_snapshot(3);
    assert!(!host.snapshot_acknowledged());
    assert!(
        !endpoint.ack(latest),
        "older ack cannot satisfy the new install"
    );
    let PumpEvent::Snapshot { install, .. } = endpoint.drain(1).remove(0) else {
        panic!("snapshot")
    };
    assert!(!host.snapshot_acknowledged());
    assert!(endpoint.ack(install));
    assert!(host.snapshot_acknowledged());
}

#[test]
fn ack_before_drain_is_refused_and_drop_closes_the_endpoint() {
    let (mut host, endpoint) = pump::<u32>();
    host.install_snapshot(7);
    let undrained = InstallId(1);
    assert!(!endpoint.ack(undrained));
    assert!(!host.snapshot_acknowledged());
    assert!(!endpoint.is_closed());
    drop(host);
    assert!(endpoint.is_closed());
    // Already queued events remain drainable; acks are refused after close.
    let PumpEvent::Snapshot { install, .. } = endpoint.drain(1).remove(0) else {
        panic!("snapshot")
    };
    assert!(!endpoint.ack(install));
}

// ---- Integration with the worker's write admission ------------------------

#[derive(Clone)]
struct Clock(Instant);
impl SessionClock for Clock {
    fn now(&self) -> Instant {
        self.0
    }
}

struct Backend;
impl SessionBackend for Backend {
    type Client = ();
    type Stream = ();
    type Writer = ();
    type SnapshotReader<'a> = ();
    async fn issue(&mut self, _: &str) -> Result<(), SessionFailure> {
        Ok(())
    }
    async fn subscribe(&mut self, _: &()) -> Result<(), SessionFailure> {
        Ok(())
    }
    async fn continuity(&mut self, _: &()) -> Result<ContinuityBoundary, SessionFailure> {
        Ok(marker())
    }
    fn lease_ttl(_: &()) -> u64 {
        600
    }
    fn writer(_: &()) {}
    fn snapshot_reader(_: &()) {}
}

fn marker() -> ContinuityBoundary {
    ContinuityBoundary {
        instance_id: DaemonInstanceId::new("synthetic-instance"),
        watermark: 0,
    }
}

fn request() -> crate::frontend::envelopes::RequestEnvelope {
    EditorSubmission::default()
        .prepare(&DraftSnapshot {
            stamp: DraftStamp {
                target: DraftTarget::Send {
                    room_id: room("room/a?opaque"),
                },
                revision: DraftRevision::default(),
            },
            body: ComposedBody::PlainText("private body".into()),
        })
        .unwrap()
}

#[tokio::test]
async fn write_admission_waits_for_app_snapshot_acknowledgement() {
    let clock = Clock(Instant::now());
    let (host, endpoint) = pump::<&'static str>();
    let (mailbox, mut worker) =
        CommandWorker::new(SessionLifecycle::new(Backend, clock.clone()), host);
    let sub = worker.connect("synthetic-secret").await.unwrap();
    worker
        .frame(
            sub,
            UiLifecycleFrame::Control(LifecycleControl::Ready(marker())),
        )
        .unwrap();
    let ticket = worker.begin_snapshot().await.unwrap();
    worker.commit_snapshot(ticket, "snapshot-1").unwrap();
    assert_eq!(worker.phase(), SessionPhase::Active);
    // Active, but the App has not installed the snapshot yet.
    assert_eq!(
        mailbox.try_submit_at(request(), clock.0),
        CommandAdmission::NotReady
    );

    let events = endpoint.drain(usize::MAX);
    let install = events
        .iter()
        .find_map(|event| match event {
            PumpEvent::Snapshot { install, snapshot } => {
                assert_eq!(*snapshot, "snapshot-1");
                Some(*install)
            }
            _ => None,
        })
        .unwrap();
    assert!(endpoint.ack(install));
    // Readiness is restored only by the worker re-evaluating admission.
    assert_eq!(
        mailbox.try_submit_at(request(), clock.0),
        CommandAdmission::NotReady
    );
    worker.refresh_admission();
    assert_eq!(
        mailbox.try_submit_at(request(), clock.0),
        CommandAdmission::Accepted
    );
    worker.shutdown();
    let _ = mailbox.drain();

    // A lag resync revokes; the old acknowledgement cannot restore admission.
    let (mailbox, mut worker) = {
        let (host, endpoint2) = pump::<&'static str>();
        let (mailbox, mut worker) =
            CommandWorker::new(SessionLifecycle::new(Backend, clock.clone()), host);
        let sub = worker.connect("synthetic-secret").await.unwrap();
        worker
            .frame(
                sub,
                UiLifecycleFrame::Control(LifecycleControl::Ready(marker())),
            )
            .unwrap();
        let ticket = worker.begin_snapshot().await.unwrap();
        worker.commit_snapshot(ticket, "first").unwrap();
        let first = endpoint2
            .drain(usize::MAX)
            .into_iter()
            .find_map(|event| match event {
                PumpEvent::Snapshot { install, .. } => Some(install),
                _ => None,
            })
            .unwrap();
        assert!(endpoint2.ack(first));
        worker.refresh_admission();
        assert_eq!(
            mailbox.try_submit_at(request(), clock.0),
            CommandAdmission::Accepted
        );
        assert!(worker.handle_one().await);
        assert!(mailbox.drain().is_some());
        worker
            .frame(
                sub,
                UiLifecycleFrame::Control(LifecycleControl::ResyncRequired {
                    boundary: marker(),
                    scope: ChangeScope::WholeMirror,
                    reason: ResyncReason::EventLag,
                }),
            )
            .unwrap();
        let ticket = worker.begin_snapshot().await.unwrap();
        worker.commit_snapshot(ticket, "second").unwrap();
        assert!(!endpoint2.ack(first));
        worker.refresh_admission();
        assert_eq!(
            mailbox.try_submit_at(request(), clock.0),
            CommandAdmission::NotReady
        );
        (mailbox, worker)
    };
    worker.shutdown();
    let _ = mailbox.drain();
}
