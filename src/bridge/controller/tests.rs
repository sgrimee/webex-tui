//! Fake-only controller integration: no daemon, UI startup or user-write path.

use super::*;
use crate::frontend::{
    continuity::{ContinuityBoundary, DaemonInstanceId, LifecycleControl},
    data::{ComposedBody, MessageId, RoomId},
    envelopes::{FrontendIntent, FrontendResult, WriteOutcome},
    events::{ChangeScope, ConnectionLayer, MessageChange, MessageKey, ResyncReason, SyncProgress},
    lifecycle::RefreshTarget,
};
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Clock(Arc<Mutex<Instant>>);
impl Clock {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Instant::now())))
    }
    fn advance(&self, seconds: u64) {
        let mut t = self.0.lock().unwrap();
        *t += Duration::from_secs(seconds);
    }
}
impl SessionClock for Clock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

struct Writer(Arc<AtomicUsize>);

impl crate::bridge::write_dispatch::UiWriteBackend for Writer {
    async fn send(
        &mut self,
        _: &str,
        _: Option<&str>,
        _: &webex_bridge::dto::MessageBodyInput,
    ) -> Result<webex_bridge::dto::Message, webex_bridge::client::ClientError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(webex_bridge::client::ClientError::Transport(
            std::io::Error::other("controller fake writer").into(),
        ))
    }
    async fn edit(
        &mut self,
        _: &str,
        _: &str,
        _: &webex_bridge::dto::MessageBodyInput,
    ) -> Result<webex_bridge::dto::Message, webex_bridge::client::ClientError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(webex_bridge::client::ClientError::Transport(
            std::io::Error::other("controller fake writer").into(),
        ))
    }
    async fn delete(&mut self, _: &str) -> Result<(), webex_bridge::client::ClientError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(webex_bridge::client::ClientError::Transport(
            std::io::Error::other("controller fake writer").into(),
        ))
    }
    async fn leave(&mut self, _: &str) -> Result<(), webex_bridge::client::ClientError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(webex_bridge::client::ClientError::Transport(
            std::io::Error::other("controller fake writer").into(),
        ))
    }
}

struct Client(Arc<AtomicUsize>);

struct Backend {
    calls: Arc<Mutex<Vec<&'static str>>>,
    writer_calls: Arc<AtomicUsize>,
    fail: Arc<Mutex<Option<SessionFailure>>>,
    pending_issue: Arc<AtomicBool>,
    boundary: Arc<Mutex<ContinuityBoundary>>,
}
impl SessionBackend for Backend {
    type Client = Client;
    type Stream = ();
    type Writer = Writer;
    type SnapshotReader = ();
    async fn issue(&mut self, secret: &str) -> Result<Client, SessionFailure> {
        assert_eq!(secret, "synthetic-secret");
        self.calls.lock().unwrap().push("issue");
        if self.pending_issue.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        if let Some(e) = self.fail.lock().unwrap().take() {
            Err(e)
        } else {
            Ok(Client(Arc::clone(&self.writer_calls)))
        }
    }
    async fn subscribe(&mut self, _: &Client) -> Result<(), SessionFailure> {
        self.calls.lock().unwrap().push("subscribe");
        Ok(())
    }
    async fn continuity(&mut self, _: &Client) -> Result<ContinuityBoundary, SessionFailure> {
        self.calls.lock().unwrap().push("continuity");
        Ok(self.boundary.lock().unwrap().clone())
    }
    async fn renew(&mut self, _: &Client) -> Result<u64, crate::bridge::session::RenewFailure> {
        // Not exercised here: behaves like a daemon at the 12 h cap.
        Err(crate::bridge::session::RenewFailure::CapReached)
    }
    fn lease_ttl(_: &Client) -> u64 {
        100
    }
    fn writer(client: &Client) -> Writer {
        Writer(Arc::clone(&client.0))
    }
    fn snapshot_reader(_: &Client) {}
}

#[derive(Default)]
struct Host {
    phases: Vec<SessionPhase>,
    effects: Vec<ProjectionEffects>,
    installed: Vec<String>,
    order: Vec<&'static str>,
    selected_room: Option<String>,
    last_local: Option<ConnectionState>,
    last_upstream: Option<ConnectionState>,
}
impl PresentationHost for Host {
    type Snapshot = String;
    fn deliver_read(
        &mut self,
        _: crate::bridge::read_dispatch::ReadTerminal,
        _: Option<crate::bridge::presentation::InstallId>,
    ) {
    }
    fn session_phase(&mut self, phase: SessionPhase) {
        self.phases.push(phase);
    }
    fn lifecycle(&mut self, projection: &LifecycleProjection, effects: ProjectionEffects) {
        self.last_local = Some(projection.local_daemon);
        self.last_upstream = Some(projection.upstream_webex);
        self.order.push(if effects.refresh.is_empty() {
            "lifecycle"
        } else {
            "refresh"
        });
        self.effects.push(effects);
    }
    fn install_snapshot(&mut self, snapshot: String) {
        self.order.push("install");
        self.installed.push(snapshot);
    }
}

type Controller = HostController<Backend, Clock, Host>;
struct Fixture {
    controller: Controller,
    clock: Clock,
    calls: Arc<Mutex<Vec<&'static str>>>,
    fail: Arc<Mutex<Option<SessionFailure>>>,
    pending_issue: Arc<AtomicBool>,
    boundary: Arc<Mutex<ContinuityBoundary>>,
    writer_calls: Arc<AtomicUsize>,
}
fn marker(watermark: u64) -> ContinuityBoundary {
    ContinuityBoundary {
        instance_id: DaemonInstanceId::new("instance-a"),
        watermark,
    }
}
fn fixture() -> Fixture {
    let clock = Clock::new();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let fail = Arc::new(Mutex::new(None));
    let pending_issue = Arc::new(AtomicBool::new(false));
    let boundary = Arc::new(Mutex::new(marker(0)));
    let writer_calls = Arc::new(AtomicUsize::new(0));
    let backend = Backend {
        calls: Arc::clone(&calls),
        writer_calls: Arc::clone(&writer_calls),
        fail: Arc::clone(&fail),
        pending_issue: Arc::clone(&pending_issue),
        boundary: Arc::clone(&boundary),
    };
    let host = Host {
        selected_room: Some("selected-room".into()),
        ..Default::default()
    };
    Fixture {
        controller: HostController::new(SessionLifecycle::new(backend, clock.clone()), host),
        clock,
        calls,
        fail,
        pending_issue,
        boundary,
        writer_calls,
    }
}
fn ready() -> UiLifecycleFrame {
    UiLifecycleFrame::Control(LifecycleControl::Ready(marker(0)))
}
fn lag(watermark: u64) -> UiLifecycleFrame {
    UiLifecycleFrame::Control(LifecycleControl::ResyncRequired {
        boundary: marker(watermark),
        scope: ChangeScope::WholeMirror,
        reason: ResyncReason::EventLag,
    })
}
fn message(watermark: u64, change: MessageChange) -> UiLifecycleFrame {
    UiLifecycleFrame::Change {
        boundary: Some(marker(watermark)),
        event: LifecycleEvent::MessageInvalidated {
            key: MessageKey {
                room_id: RoomId::new("room-a"),
                message_id: MessageId::new("message-a"),
            },
            change,
            revision: Some(2),
        },
    }
}
async fn active(f: &mut Fixture) -> SubscriptionId {
    let id = f.controller.connect("synthetic-secret").await.unwrap();
    f.controller.frame(id, ready()).unwrap();
    let ticket = f.controller.begin_snapshot().await.unwrap();
    f.controller
        .commit_snapshot(ticket, "initial".into())
        .unwrap();
    id
}

#[tokio::test]
async fn host_active_write_uses_private_writer_once() {
    let mut fixture = fixture();
    active(&mut fixture).await;
    let phase_len = fixture.controller.host().phases.len();
    let lifecycle_len = fixture.controller.host().effects.len();
    let result = fixture
        .controller
        .dispatch_write(FrontendIntent::SendMessage {
            room_id: RoomId::new("r"),
            parent_id: None,
            body: ComposedBody::PlainText("body".into()),
        })
        .await;
    assert!(matches!(
        result,
        Ok(FrontendResult::SendMessage(WriteOutcome::DeliveryUnknown(
            _
        )))
    ));
    assert_eq!(fixture.writer_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.controller.host().installed, ["initial"]);
    assert_eq!(fixture.controller.host().phases.len(), phase_len);
    assert_eq!(fixture.controller.host().effects.len(), lifecycle_len);
}

#[tokio::test]
async fn host_write_rejects_awaiting_and_snapshot_without_mutation() {
    let mut f = fixture();
    let intent = || FrontendIntent::SendMessage {
        room_id: RoomId::new("r"),
        parent_id: None,
        body: ComposedBody::PlainText("body".into()),
    };
    f.controller.connect("synthetic-secret").await.unwrap();
    assert!(matches!(
        f.controller.dispatch_write(intent()).await,
        Err(SessionWriteError::NotReady)
    ));
    let id = f.controller.subscription().unwrap();
    f.controller.frame(id, ready()).unwrap();
    assert!(matches!(
        f.controller.dispatch_write(intent()).await,
        Err(SessionWriteError::NotReady)
    ));
    assert_eq!(f.writer_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn host_write_wrong_intent_has_no_mutation() {
    let mut f = fixture();
    active(&mut f).await;
    assert!(matches!(
        f.controller
            .dispatch_write(FrontendIntent::ListAllRooms)
            .await,
        Err(SessionWriteError::Dispatch(_))
    ));
    assert_eq!(f.writer_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn host_write_expiry_publishes_one_disconnected_update() {
    let mut f = fixture();
    active(&mut f).await;
    let phase_len = f.controller.host().phases.len();
    let lifecycle_len = f.controller.host().effects.len();
    f.clock.advance(100);
    let intent = FrontendIntent::SendMessage {
        room_id: RoomId::new("r"),
        parent_id: None,
        body: ComposedBody::PlainText("body".into()),
    };
    assert!(matches!(
        f.controller.dispatch_write(intent).await,
        Err(SessionWriteError::Expired)
    ));
    assert_eq!(f.writer_calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.controller.host().phases.len(), phase_len + 1);
    assert_eq!(f.controller.host().effects.len(), lifecycle_len + 1);
    assert_eq!(
        f.controller.host().phases.last(),
        Some(&SessionPhase::Expired)
    );
    assert_eq!(
        f.controller.host().last_local,
        Some(ConnectionState::Disconnected)
    );
    let intent = FrontendIntent::SendMessage {
        room_id: RoomId::new("r"),
        parent_id: None,
        body: ComposedBody::PlainText("body".into()),
    };
    assert!(matches!(
        f.controller.dispatch_write(intent).await,
        Err(SessionWriteError::Expired)
    ));
    assert_eq!(f.writer_calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.controller.host().phases.len(), phase_len + 1);
    assert_eq!(f.controller.host().effects.len(), lifecycle_len + 1);
}

#[tokio::test]
async fn connect_ready_requests_neutral_snapshot_refresh_without_changing_selected_room() {
    let mut f = fixture();
    let id = f.controller.connect("synthetic-secret").await.unwrap();
    assert_eq!(f.controller.phase(), SessionPhase::AwaitingReady);
    let action = f.controller.frame(id, ready()).unwrap();
    assert_eq!(
        action,
        FrameAction::NeedSnapshot {
            scope: ChangeScope::WholeMirror,
            reason: ResyncReason::StreamReconnect
        }
    );
    assert_eq!(
        f.controller.host().selected_room.as_deref(),
        Some("selected-room")
    );
    assert!(f.controller.host().effects.iter().any(|e| e
        .refresh
        .contains(&RefreshTarget::LoadedMessages(ChangeScope::WholeMirror))));
    assert_eq!(*f.calls.lock().unwrap(), ["issue", "subscribe"]);
}

#[tokio::test]
async fn commit_installs_ticket_validated_snapshot_before_replaying_newer_metadata() {
    let mut f = fixture();
    let id = f.controller.connect("synthetic-secret").await.unwrap();
    f.controller.frame(id, ready()).unwrap();
    *f.boundary.lock().unwrap() = marker(5);
    let ticket = f.controller.begin_snapshot().await.unwrap();
    assert!(f.controller.snapshot_reader(&ticket).is_ok());
    assert_eq!(
        f.controller
            .frame(id, message(3, MessageChange::Updated))
            .unwrap(),
        FrameAction::Buffered
    );
    assert_eq!(
        f.controller
            .frame(id, message(6, MessageChange::Updated))
            .unwrap(),
        FrameAction::Buffered
    );
    f.controller.host.order.clear();
    let count = f
        .controller
        .commit_snapshot(ticket, "prepared snapshot".into())
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(f.controller.phase(), SessionPhase::Active);
    assert_eq!(f.controller.host().installed, ["prepared snapshot"]);
    assert_eq!(
        f.controller.host().order,
        ["install", "lifecycle", "refresh"]
    );
    assert_eq!(
        f.controller.host().selected_room.as_deref(),
        Some("selected-room")
    );
}

#[tokio::test]
async fn active_message_delete_applies_one_redaction_and_revision_refresh_hint() {
    let mut f = fixture();
    let id = active(&mut f).await;
    f.controller.host.effects.clear();
    f.controller
        .frame(id, message(1, MessageChange::Deleted))
        .unwrap();
    let effects = f.controller.host().effects.last().unwrap();
    assert_eq!(
        effects.redact,
        [MessageKey {
            room_id: RoomId::new("room-a"),
            message_id: MessageId::new("message-a")
        }]
    );
    assert!(effects.refresh.contains(&RefreshTarget::Message {
        key: MessageKey {
            room_id: RoomId::new("room-a"),
            message_id: MessageId::new("message-a")
        },
        min_revision: Some(2)
    }));
    assert!(!effects.affects_room(&RoomId::new("other-room")));
    assert_eq!(
        f.controller.host().selected_room.as_deref(),
        Some("selected-room")
    );
}

#[tokio::test]
async fn stale_frames_and_stale_malformed_bridge_event_do_not_reach_host() {
    let mut f = fixture();
    let old = active(&mut f).await;
    let replacement = f.controller.connect("synthetic-secret").await.unwrap();
    let before = f.controller.host().effects.len();
    assert_eq!(
        f.controller
            .frame(
                old,
                UiLifecycleFrame::Control(LifecycleControl::SessionExpired)
            )
            .unwrap(),
        FrameAction::StaleSubscription
    );
    assert_eq!(
        f.controller
            .bridge_event(
                old,
                &BridgeEvent {
                    name: "message.created".into(),
                    data: json!({})
                }
            )
            .unwrap(),
        FrameAction::StaleSubscription
    );
    assert_eq!(f.controller.subscription(), Some(replacement));
    assert_eq!(f.controller.host().effects.len(), before);
}

#[tokio::test]
async fn lag_discards_old_snapshot_ticket_before_any_host_installation() {
    let mut f = fixture();
    let id = f.controller.connect("synthetic-secret").await.unwrap();
    f.controller.frame(id, ready()).unwrap();
    let old = f.controller.begin_snapshot().await.unwrap();
    f.controller.frame(id, lag(8)).unwrap();
    assert_eq!(
        f.controller.commit_snapshot(old, "must not install".into()),
        Err(SessionFailure::StaleSnapshot)
    );
    assert!(f.controller.host().installed.is_empty());
    *f.boundary.lock().unwrap() = marker(8);
    let current = f.controller.begin_snapshot().await.unwrap();
    f.controller
        .commit_snapshot(current, "current snapshot".into())
        .unwrap();
    assert_eq!(f.controller.host().installed, ["current snapshot"]);
}

#[tokio::test]
async fn terminal_expiry_notifies_local_session_without_guessing_upstream_oauth() {
    let mut f = fixture();
    let id = active(&mut f).await;
    f.controller
        .frame(
            id,
            UiLifecycleFrame::Change {
                boundary: None,
                event: LifecycleEvent::Connection {
                    layer: ConnectionLayer::UpstreamWebex,
                    state: ConnectionState::Connected,
                },
            },
        )
        .unwrap();
    let calls = f.calls.lock().unwrap().len();
    f.controller
        .frame(
            id,
            UiLifecycleFrame::Control(LifecycleControl::SessionExpired),
        )
        .unwrap();
    assert_eq!(f.controller.phase(), SessionPhase::Expired);
    assert_eq!(
        f.controller.host().last_local,
        Some(ConnectionState::Disconnected)
    );
    assert_eq!(
        f.controller.host().last_upstream,
        Some(ConnectionState::Connected)
    );
    assert_eq!(f.calls.lock().unwrap().len(), calls);
}

#[tokio::test]
async fn cancelled_connect_publishes_connecting_before_its_first_io_await() {
    use std::{
        future::Future,
        task::{Context, Poll, Wake, Waker},
    };

    struct Noop;
    impl Wake for Noop {
        fn wake(self: Arc<Self>) {}
    }

    let mut f = fixture();
    f.pending_issue.store(true, Ordering::SeqCst);
    let waker = Waker::from(Arc::new(Noop));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(f.controller.connect("synthetic-secret"));
    assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    drop(future);

    // Cancellation leaves the owner in Connecting, but the host has already
    // seen that transition and cannot retain the old presentation state.
    assert_eq!(f.controller.phase(), SessionPhase::Connecting);
    assert_eq!(
        f.controller.host().phases.last(),
        Some(&SessionPhase::Connecting)
    );
    assert_eq!(
        f.controller.host().last_local,
        Some(ConnectionState::Connecting)
    );
    assert_eq!(*f.calls.lock().unwrap(), ["issue"]);
}

#[tokio::test]
async fn renewal_and_renewal_failure_are_explicit_without_auto_retry_or_snapshot_install() {
    let mut f = fixture();
    active(&mut f).await;
    f.clock.advance(79);
    let effects = f.controller.host().effects.len();
    assert!(!f.controller.renew_if_due("synthetic-secret").await.unwrap());
    assert_eq!(f.controller.host().effects.len(), effects);
    f.clock.advance(1);
    assert!(f.controller.renew_if_due("synthetic-secret").await.unwrap());
    assert_eq!(f.controller.phase(), SessionPhase::AwaitingReady);
    assert_eq!(f.controller.host().installed, ["initial"]);
    *f.fail.lock().unwrap() = Some(SessionFailure::AuthRequired);
    assert_eq!(
        f.controller.connect("synthetic-secret").await,
        Err(SessionFailure::AuthRequired)
    );
    assert_eq!(
        f.controller.phase(),
        SessionPhase::Failed(SessionFailure::AuthRequired)
    );
    assert_eq!(
        f.controller.host().last_local,
        Some(ConnectionState::Disconnected)
    );
    assert_eq!(
        *f.calls.lock().unwrap(),
        [
            "issue",
            "subscribe",
            "continuity",
            "issue",
            "subscribe",
            "issue"
        ]
    );
}

#[tokio::test]
async fn lease_tick_and_eof_fail_closed_and_keep_existing_snapshot_data() {
    let mut f = fixture();
    let old = active(&mut f).await;
    f.clock.advance(100);
    assert_eq!(f.controller.check_lease(), LeaseAction::Reconnect);
    assert_eq!(f.controller.phase(), SessionPhase::Expired);
    assert_eq!(f.controller.host().installed, ["initial"]);
    assert_eq!(
        f.controller.stream_failed(old, SessionFailure::StreamEnded),
        FrameAction::StaleSubscription
    );
    let current = f.controller.connect("synthetic-secret").await.unwrap();
    assert_eq!(
        f.controller
            .stream_failed(current, SessionFailure::StreamEnded),
        FrameAction::ReconnectRequired
    );
    assert_eq!(
        f.controller.phase(),
        SessionPhase::Failed(SessionFailure::StreamEnded)
    );
    assert_eq!(f.controller.host().installed, ["initial"]);
}

#[tokio::test]
async fn invalid_current_bridge_metadata_fails_before_lifecycle_redaction() {
    let mut f = fixture();
    let id = active(&mut f).await;
    f.controller.host.effects.clear();
    let error = f.controller.bridge_event(
        id,
        &BridgeEvent {
            name: "message.deleted".into(),
            data: json!({"messageId":"message-a","roomId":"room-a"}),
        },
    );
    assert_eq!(error, Err(SessionFailure::InvalidResponse));
    assert_eq!(
        f.controller.phase(),
        SessionPhase::Failed(SessionFailure::InvalidResponse)
    );
    assert!(f
        .controller
        .host()
        .effects
        .iter()
        .all(|e| e.redact.is_empty()));
}

#[tokio::test]
async fn real_bridge_frame_projects_sparse_sync_data_only_after_stream_fencing() {
    let mut f = fixture();
    let id = active(&mut f).await;
    f.controller.bridge_event(id,&BridgeEvent {name:"sync.progress".into(),data:json!({"continuity":{"instanceId":"instance-a","watermark":1},"rooms":12,"gaps":0,"initial":5,"failed":1,"text":"extra remote data"})}).unwrap();
    assert_eq!(
        f.controller.projection().sync,
        SyncProgress {
            rooms: Some(12),
            gaps: Some(0),
            failures: Some(1),
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn snapshot_that_expires_before_commit_never_reaches_host_installation() {
    let mut f = fixture();
    let id = f.controller.connect("synthetic-secret").await.unwrap();
    f.controller.frame(id, ready()).unwrap();
    let ticket = f.controller.begin_snapshot().await.unwrap();
    f.clock.advance(100);
    assert_eq!(
        f.controller
            .commit_snapshot(ticket, "expired snapshot".into()),
        Err(SessionFailure::Expired)
    );
    assert!(f.controller.host().installed.is_empty());
    assert_eq!(
        f.controller.host().last_local,
        Some(ConnectionState::Disconnected)
    );
}

#[tokio::test]
async fn old_snapshot_ticket_cannot_install_after_reconnect() {
    let mut f = fixture();
    let old = f.controller.connect("synthetic-secret").await.unwrap();
    f.controller.frame(old, ready()).unwrap();
    let ticket = f.controller.begin_snapshot().await.unwrap();
    let current = f.controller.connect("synthetic-secret").await.unwrap();
    f.controller.frame(current, ready()).unwrap();
    assert_eq!(
        f.controller.commit_snapshot(ticket, "old snapshot".into()),
        Err(SessionFailure::StaleSnapshot)
    );
    assert!(f.controller.host().installed.is_empty());
    assert_eq!(f.controller.phase(), SessionPhase::SnapshotRequired);
}

#[tokio::test]
async fn scoped_room_and_membership_changes_stay_neutral_without_selection_or_body_removal() {
    let mut f = fixture();
    let id = active(&mut f).await;
    f.controller.bridge_event(id,&BridgeEvent {name:"room.resync".into(),data:json!({"continuity":{"instanceId":"instance-a","watermark":1},"roomId":"room-a","reason":"reconciled"})}).unwrap();
    let effects = f.controller.host().effects.last().unwrap();
    assert!(effects
        .refresh
        .contains(&RefreshTarget::LoadedMessages(ChangeScope::Room(
            RoomId::new("room-a")
        ))));
    assert!(effects.redact.is_empty());
    assert!(!effects.affects_room(&RoomId::new("other-room")));
    f.controller.bridge_event(id,&BridgeEvent {name:"membership.changed".into(),data:json!({"continuity":{"instanceId":"instance-a","watermark":2},"roomId":"room-a","isMember":false})}).unwrap();
    let effects = f.controller.host().effects.last().unwrap();
    assert_eq!(effects.membership.as_ref().unwrap().is_member, Some(false));
    assert!(effects.redact.is_empty());
    assert_eq!(
        f.controller.host().selected_room.as_deref(),
        Some("selected-room")
    );
}

#[tokio::test]
async fn unknown_sequenced_frames_and_disconnect_never_become_host_actions_or_writes() {
    let mut f = fixture();
    let id = active(&mut f).await;
    let before = f.controller.host().effects.len();
    f.controller
        .frame(
            id,
            UiLifecycleFrame::Unrecognized {
                boundary: Some(marker(1)),
            },
        )
        .unwrap();
    assert_eq!(f.controller.host().effects.len(), before);
    let calls = f.calls.lock().unwrap().clone();
    f.controller.disconnect();
    assert_eq!(*f.calls.lock().unwrap(), calls);
    assert_eq!(f.controller.phase(), SessionPhase::Disconnected);
    assert_eq!(f.controller.host().installed, ["initial"]);
}

#[tokio::test]
async fn host_read_expiry_publishes_one_phase_update_like_writes() {
    let mut f = fixture();
    active(&mut f).await;
    let phase_len = f.controller.host().phases.len();
    let lifecycle_len = f.controller.host().effects.len();
    f.clock.advance(100);
    let read = || {
        crate::frontend::envelopes::RequestSequence::default()
            .begin(FrontendIntent::ListAllRooms, None)
            .unwrap()
    };
    for _ in 0..2 {
        assert!(matches!(
            f.controller.dispatch_read(read()).await,
            Err(SessionReadError::Expired)
        ));
        assert_eq!(f.controller.host().phases.len(), phase_len + 1);
        assert_eq!(f.controller.host().effects.len(), lifecycle_len + 1);
    }
    assert_eq!(
        f.controller.host().phases.last(),
        Some(&SessionPhase::Expired)
    );
}
