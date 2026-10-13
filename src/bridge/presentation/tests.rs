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
            PumpEvent::Read(_) => "read",
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

fn only_lifecycle(events: Vec<PumpEvent<()>>) -> ProjectionEffects {
    assert_eq!(events.len(), 1);
    let Some(PumpEvent::Lifecycle { effects, .. }) = events.into_iter().next() else {
        panic!("lifecycle")
    };
    effects
}

#[test]
fn bursts_of_one_message_stay_one_target_with_highest_revision() {
    let (mut host, endpoint) = pump::<()>();
    let projection = LifecycleProjection::default();
    for revision in 0..5000_u64 {
        host.lifecycle(
            &projection,
            ProjectionEffects {
                refresh: vec![
                    RefreshTarget::Message {
                        key: key("hot"),
                        min_revision: Some(revision % 4000),
                    },
                    RefreshTarget::Room(room("room")),
                ],
                redact: vec![key("hot")],
                ..Default::default()
            },
        );
    }
    let effects = only_lifecycle(endpoint.drain(usize::MAX));
    assert_eq!(
        effects.refresh,
        [
            RefreshTarget::Message {
                key: key("hot"),
                min_revision: Some(3999),
            },
            RefreshTarget::Room(room("room")),
        ]
    );
    assert_eq!(effects.redact, [key("hot")]);
    assert_eq!(effects.resync, None);
}

#[test]
fn distinct_targets_beyond_cap_collapse_to_whole_mirror_lag() {
    let (mut host, endpoint) = pump::<()>();
    let projection = LifecycleProjection::default();
    for index in 0..REFRESH_CAP * 10 {
        host.lifecycle(
            &projection,
            ProjectionEffects {
                refresh: vec![RefreshTarget::Message {
                    key: key(&format!("m{index}")),
                    min_revision: None,
                }],
                redact: vec![key(&format!("m{index}"))],
                ..Default::default()
            },
        );
    }
    let effects = only_lifecycle(endpoint.drain(usize::MAX));
    assert_eq!(
        effects.resync,
        Some((ChangeScope::WholeMirror, ResyncReason::EventLag))
    );
    assert_eq!(
        effects.refresh,
        [
            RefreshTarget::LoadedMessages(ChangeScope::WholeMirror),
            RefreshTarget::Status,
            RefreshTarget::Rooms,
        ]
    );
    // Redactions are never dropped, only deduplicated.
    assert_eq!(effects.redact.len(), REFRESH_CAP * 10);
}

#[test]
fn whole_mirror_reconcile_subsumes_scoped_targets() {
    let (mut host, endpoint) = pump::<()>();
    let projection = LifecycleProjection::default();
    host.lifecycle(&projection, effects(vec![RefreshTarget::Room(room("a"))]));
    host.lifecycle(
        &projection,
        effects(vec![RefreshTarget::LoadedMessages(
            ChangeScope::WholeMirror,
        )]),
    );
    host.lifecycle(
        &projection,
        effects(vec![
            RefreshTarget::Room(room("b")),
            RefreshTarget::LoadedMessages(ChangeScope::Room(room("c"))),
            RefreshTarget::Message {
                key: key("m"),
                min_revision: Some(1),
            },
            RefreshTarget::Status,
        ]),
    );
    let effects = only_lifecycle(endpoint.drain(usize::MAX));
    assert_eq!(
        effects.refresh,
        [
            RefreshTarget::LoadedMessages(ChangeScope::WholeMirror),
            RefreshTarget::Status,
            RefreshTarget::Rooms,
        ]
    );
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

// ---- Read mailbox and worker read dispatch --------------------------------

mod reads {
    use super::*;
    use crate::{
        bridge::{
            commands::{ReadAdmission, READ_CAPACITY},
            read_dispatch::{ReadTerminal, ReadUnavailable, UiReadBackend},
        },
        frontend::envelopes::{
            FrontendIntent, FrontendResult, ReadOutcome, RequestEnvelope, RequestSequence,
        },
    };
    use std::{
        future::Future,
        pin::Pin,
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
        task::{Context, Poll, Waker},
        time::Duration,
    };
    use webex_bridge::{client::ClientError, dto};

    type Reply<T> = Result<T, ClientError>;

    #[derive(Clone, Default)]
    struct Reader {
        calls: Arc<AtomicUsize>,
        pending: Arc<AtomicBool>,
    }
    fn inert<T>() -> Reply<T> {
        Err(ClientError::Transport(
            std::io::Error::other("synthetic").into(),
        ))
    }
    impl UiReadBackend for Reader {
        async fn me(&self) -> Reply<dto::Person> {
            inert()
        }
        async fn rooms(&self) -> Reply<Vec<dto::Room>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.pending.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            Ok(Vec::new())
        }
        async fn room(&self, _: &str) -> Reply<dto::Room> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            inert()
        }
        async fn team(&self, _: &str) -> Reply<dto::Team> {
            inert()
        }
        async fn person_read(&self, _: &str) -> Reply<dto::PersonRead> {
            inert()
        }
        async fn room_messages_page(
            &self,
            _: &str,
            _: Option<&str>,
            _: Option<u32>,
        ) -> Reply<dto::MessagePage> {
            inert()
        }
        async fn message_read(&self, _: &str) -> Reply<dto::MessageRead> {
            inert()
        }
        async fn thread_children(
            &self,
            _: &str,
            _: Option<&str>,
            _: Option<u32>,
        ) -> Reply<dto::ThreadChildren> {
            inert()
        }
    }

    struct ReadBackend(Reader);
    impl SessionBackend for ReadBackend {
        type Client = Reader;
        type Stream = ();
        type Writer = ();
        type SnapshotReader<'a> = Reader;
        async fn issue(&mut self, _: &str) -> Result<Reader, SessionFailure> {
            Ok(self.0.clone())
        }
        async fn subscribe(&mut self, _: &Reader) -> Result<(), SessionFailure> {
            Ok(())
        }
        async fn continuity(&mut self, _: &Reader) -> Result<ContinuityBoundary, SessionFailure> {
            Ok(marker())
        }
        fn lease_ttl(_: &Reader) -> u64 {
            600
        }
        fn writer(_: &Reader) {}
        fn snapshot_reader(client: &Reader) -> Reader {
            client.clone()
        }
    }

    type Worker = CommandWorker<ReadBackend, Clock, PumpHost<&'static str>>;
    struct Fixture {
        mailbox: crate::bridge::commands::CommandMailbox,
        worker: Worker,
        endpoint: PresentationEndpoint<&'static str>,
        reader: Reader,
    }
    fn fixture() -> Fixture {
        let reader = Reader::default();
        let (host, endpoint) = pump();
        let (mailbox, worker) = CommandWorker::new(
            SessionLifecycle::new(ReadBackend(reader.clone()), Clock(Instant::now())),
            host,
        );
        Fixture {
            mailbox,
            worker,
            endpoint,
            reader,
        }
    }
    async fn active(f: &mut Fixture) {
        let sub = f.worker.connect("synthetic-secret").await.unwrap();
        f.worker
            .frame(
                sub,
                UiLifecycleFrame::Control(LifecycleControl::Ready(marker())),
            )
            .unwrap();
        let ticket = f.worker.begin_snapshot().await.unwrap();
        f.worker.commit_snapshot(ticket, "snapshot").unwrap();
    }
    fn read(intent: FrontendIntent) -> RequestEnvelope {
        RequestSequence::default().begin(intent, None).unwrap()
    }
    fn rooms() -> RequestEnvelope {
        read(FrontendIntent::ListAllRooms)
    }
    fn room_read(id: &str) -> RequestEnvelope {
        read(FrontendIntent::UpdateRoom(room(id)))
    }
    fn reads_of(events: Vec<PumpEvent<&'static str>>) -> Vec<ReadTerminal> {
        events
            .into_iter()
            .filter_map(|event| match event {
                PumpEvent::Read(terminal) => Some(terminal),
                _ => None,
            })
            .collect()
    }
    fn unavailable(terminal: &ReadTerminal) -> Option<ReadUnavailable> {
        match terminal {
            ReadTerminal::Unavailable { reason, .. } => Some(*reason),
            ReadTerminal::Result(_) => None,
        }
    }
    fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
        future.poll(&mut Context::from_waker(Waker::noop()))
    }

    #[tokio::test]
    async fn admission_refuses_invalid_duplicate_full_and_closed() {
        let mut f = fixture();
        assert_eq!(
            f.mailbox
                .try_read(read(FrontendIntent::LeaveRoom(room("r")))),
            ReadAdmission::InvalidRequest
        );
        assert_eq!(
            f.mailbox.try_read(room_read("")),
            ReadAdmission::InvalidRequest
        );
        assert_eq!(f.mailbox.try_read(rooms()), ReadAdmission::Accepted);
        assert_eq!(f.mailbox.try_read(rooms()), ReadAdmission::Duplicate);
        for index in 1..READ_CAPACITY {
            assert_eq!(
                f.mailbox.try_read(room_read(&format!("room-{index}"))),
                ReadAdmission::Accepted
            );
        }
        assert_eq!(
            f.mailbox.try_read(room_read("one-too-many")),
            ReadAdmission::Full
        );
        f.worker.shutdown();
        assert_eq!(f.mailbox.try_read(room_read("late")), ReadAdmission::Closed);
        // Every admitted read got exactly one Shutdown terminal, zero I/O.
        let terminals = reads_of(f.endpoint.drain(usize::MAX));
        assert_eq!(terminals.len(), READ_CAPACITY);
        assert!(terminals
            .iter()
            .all(|t| unavailable(t) == Some(ReadUnavailable::Shutdown)));
        assert_eq!(f.reader.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn reads_are_active_only_and_ordered_after_their_snapshot() {
        let mut f = fixture();
        let early = rooms();
        let early_context = early.context.clone();
        assert_eq!(f.mailbox.try_read(early), ReadAdmission::Accepted);
        assert!(f.worker.handle_read().await);
        let terminals = reads_of(f.endpoint.drain(usize::MAX));
        assert_eq!(terminals.len(), 1);
        assert_eq!(terminals[0].context(), &early_context);
        assert_eq!(unavailable(&terminals[0]), Some(ReadUnavailable::NotReady));
        assert_eq!(f.reader.calls.load(Ordering::SeqCst), 0);

        active(&mut f).await;
        let r = rooms();
        let context = r.context.clone();
        assert_eq!(f.mailbox.try_read(r), ReadAdmission::Accepted);
        assert!(f.worker.handle_read().await);
        assert!(!f.worker.handle_read().await, "queue empty, no replay");
        let events = f.endpoint.drain(usize::MAX);
        let kinds = kinds(&events);
        let snapshot = kinds.iter().position(|k| *k == "snapshot").unwrap();
        let read_at = kinds.iter().position(|k| *k == "read").unwrap();
        assert!(snapshot < read_at, "{kinds:?}");
        let terminals = reads_of(events);
        let ReadTerminal::Result(envelope) = &terminals[0] else {
            panic!("result")
        };
        assert_eq!(envelope.context, context);
        assert!(matches!(
            envelope.result,
            FrontendResult::ListAllRooms(ReadOutcome::Success(_))
        ));
        assert_eq!(f.reader.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stuck_read_times_out_and_frees_the_controller() {
        let mut f = fixture();
        active(&mut f).await;
        f.worker.set_read_deadline(Duration::from_millis(20));
        f.reader.pending.store(true, Ordering::SeqCst);
        assert_eq!(f.mailbox.try_read(rooms()), ReadAdmission::Accepted);
        assert!(f.worker.handle_read().await);
        let terminals = reads_of(f.endpoint.drain(usize::MAX));
        assert_eq!(unavailable(&terminals[0]), Some(ReadUnavailable::TimedOut));
        assert_eq!(f.reader.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            f.worker.check_lease(),
            crate::bridge::session::LeaseAction::None
        );
        // The same read may be requested afresh; it was not retried by itself.
        assert_eq!(f.mailbox.try_read(rooms()), ReadAdmission::Accepted);
    }

    #[tokio::test]
    async fn dropped_read_is_settled_once_as_interrupted() {
        let mut f = fixture();
        active(&mut f).await;
        f.reader.pending.store(true, Ordering::SeqCst);
        let r = rooms();
        let context = r.context.clone();
        assert_eq!(f.mailbox.try_read(r), ReadAdmission::Accepted);
        {
            let mut handle = Box::pin(f.worker.handle_read());
            assert!(poll_once(handle.as_mut()).is_pending());
        } // dropped mid-await by its owner
          // In flight: an identical request is still coalesced, not re-sent.
        assert_eq!(f.mailbox.try_read(rooms()), ReadAdmission::Duplicate);
        assert!(reads_of(f.endpoint.drain(usize::MAX)).is_empty());
        f.reader.pending.store(false, Ordering::SeqCst);
        assert!(!f.worker.handle_read().await);
        let terminals = reads_of(f.endpoint.drain(usize::MAX));
        assert_eq!(terminals.len(), 1);
        assert_eq!(terminals[0].context(), &context);
        assert_eq!(
            unavailable(&terminals[0]),
            Some(ReadUnavailable::Interrupted)
        );
        f.worker.shutdown();
        assert!(
            reads_of(f.endpoint.drain(usize::MAX)).is_empty(),
            "exactly once"
        );
        assert_eq!(f.reader.calls.load(Ordering::SeqCst), 1);
    }
}
