//! Fake-only lifecycle tests: no socket, daemon, Keychain, OAuth or wall-time sleep.

use super::*;
use crate::frontend::{
    continuity::DaemonInstanceId,
    data::{MessageId, RoomId},
    events::{ConnectionLayer, ConnectionState, MessageChange, MessageKey},
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

#[derive(Clone)]
struct Clock(Arc<Mutex<Instant>>);
impl Clock {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Instant::now())))
    }
    fn advance(&self, duration: Duration) {
        let mut now = self.0.lock().unwrap();
        *now = now.checked_add(duration).unwrap();
    }
}
impl SessionClock for Clock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

#[derive(Default)]
struct Counts {
    clients_dropped: AtomicUsize,
    streams_dropped: AtomicUsize,
}
struct Client {
    id: usize,
    ttl: u64,
    counts: Arc<Counts>,
}
impl Drop for Client {
    fn drop(&mut self) {
        self.counts.clients_dropped.fetch_add(1, Ordering::SeqCst);
    }
}
struct Stream(Arc<Counts>);
impl Drop for Stream {
    fn drop(&mut self) {
        self.0.streams_dropped.fetch_add(1, Ordering::SeqCst);
    }
}

struct Backend {
    clock: Clock,
    counts: Arc<Counts>,
    calls: Vec<&'static str>,
    ttl: u64,
    issue_failure: Option<SessionFailure>,
    stream_failure: Option<SessionFailure>,
    snapshot_failure: Option<SessionFailure>,
    issue_delay: Duration,
    stream_delay: Duration,
    pending_issue: bool,
    boundary: ContinuityBoundary,
}
impl SessionBackend for Backend {
    type Client = Client;
    type Stream = Stream;
    type SnapshotReader<'a> = ();
    async fn issue(&mut self, secret: &str) -> Result<Client, SessionFailure> {
        assert!(!secret.is_empty()); // fixture input is used, never retained in the backend
        self.calls.push("issue");
        self.clock.advance(self.issue_delay);
        if self.pending_issue {
            std::future::pending::<()>().await;
        }
        if let Some(error) = self.issue_failure.take() {
            return Err(error);
        }
        Ok(Client {
            id: self.calls.len(),
            ttl: self.ttl,
            counts: Arc::clone(&self.counts),
        })
    }
    async fn subscribe(&mut self, _client: &Client) -> Result<Stream, SessionFailure> {
        self.calls.push("subscribe");
        self.clock.advance(self.stream_delay);
        if let Some(error) = self.stream_failure.take() {
            return Err(error);
        }
        Ok(Stream(Arc::clone(&self.counts)))
    }
    async fn continuity(&mut self, _client: &Client) -> Result<ContinuityBoundary, SessionFailure> {
        self.calls.push("continuity");
        if let Some(error) = self.snapshot_failure.take() {
            return Err(error);
        }
        Ok(self.boundary.clone())
    }
    fn lease_ttl(client: &Client) -> u64 {
        client.ttl
    }
    fn snapshot_reader(_: &Client) {}
}

type Owner = SessionLifecycle<Backend, Clock>;
const SECRET: &str = "synthetic-paired-secret";
fn boundary(instance: &str, watermark: u64) -> ContinuityBoundary {
    ContinuityBoundary {
        instance_id: DaemonInstanceId::new(instance),
        watermark,
    }
}
fn owner() -> Owner {
    let clock = Clock::new();
    let backend = Backend {
        clock: clock.clone(),
        counts: Arc::new(Counts::default()),
        calls: Vec::new(),
        ttl: 100,
        issue_failure: None,
        stream_failure: None,
        snapshot_failure: None,
        issue_delay: Duration::ZERO,
        stream_delay: Duration::ZERO,
        pending_issue: false,
        boundary: boundary("instance-a", 0),
    };
    SessionLifecycle::new(backend, clock)
}
fn ready(instance: &str, watermark: u64) -> UiLifecycleFrame {
    UiLifecycleFrame::Control(LifecycleControl::Ready(boundary(instance, watermark)))
}
fn change(watermark: u64) -> UiLifecycleFrame {
    UiLifecycleFrame::Change {
        boundary: Some(boundary("instance-a", watermark)),
        event: LifecycleEvent::MessageInvalidated {
            key: MessageKey {
                room_id: RoomId::new("room-a"),
                message_id: MessageId::new("message-a"),
            },
            change: MessageChange::Updated,
            revision: Some(watermark),
        },
    }
}
fn lag(watermark: u64) -> UiLifecycleFrame {
    UiLifecycleFrame::Control(LifecycleControl::ResyncRequired {
        boundary: boundary("instance-a", watermark),
        scope: ChangeScope::WholeMirror,
        reason: ResyncReason::EventLag,
    })
}
async fn active(owner: &mut Owner) -> SubscriptionId {
    let id = owner.connect(SECRET).await.unwrap();
    owner.observe(id, ready("instance-a", 0)).unwrap();
    let ticket = owner.begin_snapshot().await.unwrap();
    owner.finish_snapshot(ticket).unwrap();
    id
}

#[tokio::test]
async fn connect_owns_client_stream_deadlines_but_waits_for_ready_before_access() {
    let mut owner = owner();
    let start = owner.clock.now();
    let id = owner.connect(SECRET).await.unwrap();
    assert_eq!(owner.phase(), SessionPhase::AwaitingReady);
    assert_eq!(owner.subscription(), Some(id));
    assert_eq!(
        owner.lease_deadline(),
        Some(start + Duration::from_secs(100))
    );
    assert_eq!(owner.renew_at(), Some(start + Duration::from_secs(80)));
    assert!(owner.active.is_some());
    assert!(matches!(
        owner.begin_snapshot().await,
        Err(SessionFailure::NotReady)
    ));
    assert_eq!(owner.backend.calls, ["issue", "subscribe"]);
    assert!(owner.stream_mut().is_some());
}

#[tokio::test]
async fn ready_then_subscribe_first_snapshot_enables_client_without_any_write() {
    let mut owner = owner();
    let id = owner.connect(SECRET).await.unwrap();
    assert_eq!(
        owner.observe(id, ready("instance-a", 0)).unwrap(),
        FrameAction::NeedSnapshot {
            scope: ChangeScope::WholeMirror,
            reason: ResyncReason::StreamReconnect
        }
    );
    assert_eq!(owner.phase(), SessionPhase::SnapshotRequired);
    let ticket = owner.begin_snapshot().await.unwrap();
    assert!(owner.snapshot_reader(&ticket).is_ok());
    assert!(owner.finish_snapshot(ticket.clone()).unwrap().is_empty());
    assert_eq!(owner.phase(), SessionPhase::Active);
    assert_eq!(
        owner.snapshot_reader(&ticket),
        Err(SessionFailure::StaleSnapshot)
    );
    assert_eq!(owner.backend.calls, ["issue", "subscribe", "continuity"]);
}

#[tokio::test]
async fn renewal_is_caller_scheduled_at_eighty_percent_and_replaces_handles() {
    let mut owner = owner();
    let old = active(&mut owner).await;
    let old_client = owner.active.as_ref().unwrap().client.id;
    owner.clock.advance(Duration::from_secs(79));
    assert!(!owner.renew_if_due(SECRET).await.unwrap());
    owner.clock.advance(Duration::from_secs(1));
    assert_eq!(owner.check_lease(), LeaseAction::Renew);
    assert!(owner.renew_if_due(SECRET).await.unwrap());
    assert_ne!(owner.subscription(), Some(old));
    assert_eq!(owner.phase(), SessionPhase::AwaitingReady);
    assert_ne!(owner.active.as_ref().unwrap().client.id, old_client);
    assert_eq!(
        owner.backend.counts.clients_dropped.load(Ordering::SeqCst),
        1
    );
    assert_eq!(
        owner.backend.counts.streams_dropped.load(Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn lease_expiry_hides_client_without_waiting_for_sse_and_tick_drops_handles() {
    let mut owner = owner();
    active(&mut owner).await;
    owner.clock.advance(Duration::from_secs(100));
    assert_eq!(owner.phase(), SessionPhase::Expired);
    assert!(owner.active.is_some());
    assert_eq!(owner.check_lease(), LeaseAction::Reconnect);
    assert_eq!(owner.phase(), SessionPhase::Expired);
    assert!(owner.subscription().is_none());
    assert!(owner.stream_mut().is_none());
    assert_eq!(
        owner.backend.counts.clients_dropped.load(Ordering::SeqCst),
        1
    );
    assert_eq!(
        owner.backend.counts.streams_dropped.load(Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn terminal_expiry_is_processed_before_deadline_or_snapshot_filtering() {
    let mut owner = owner();
    let id = active(&mut owner).await;
    owner.clock.advance(Duration::from_secs(100));
    assert_eq!(
        owner
            .observe(
                id,
                UiLifecycleFrame::Control(LifecycleControl::SessionExpired)
            )
            .unwrap(),
        FrameAction::Expired
    );
    assert_eq!(owner.phase(), SessionPhase::Expired);
    assert!(owner.active.is_none());
    assert_eq!(owner.backend.calls, ["issue", "subscribe", "continuity"]);
}

#[tokio::test]
async fn stale_expiry_ready_and_lag_cannot_invalidate_replacement_subscription() {
    let mut owner = owner();
    let old = active(&mut owner).await;
    let replacement = owner.connect(SECRET).await.unwrap();
    for frame in [
        UiLifecycleFrame::Control(LifecycleControl::SessionExpired),
        ready("other-instance", 8),
        lag(8),
    ] {
        assert_eq!(
            owner.observe(old, frame).unwrap(),
            FrameAction::StaleSubscription
        );
        assert_eq!(owner.phase(), SessionPhase::AwaitingReady);
        assert_eq!(owner.subscription(), Some(replacement));
    }
}

#[tokio::test]
async fn auth_and_stream_setup_failures_fail_closed_and_do_not_auto_retry() {
    let mut owner = owner();
    owner.backend.issue_failure = Some(SessionFailure::AuthRequired);
    assert_eq!(
        owner.connect(SECRET).await,
        Err(SessionFailure::AuthRequired)
    );
    assert_eq!(
        owner.phase(),
        SessionPhase::Failed(SessionFailure::AuthRequired)
    );
    assert!(owner.active.is_none());
    assert_eq!(owner.backend.calls, ["issue"]);
    owner.backend.stream_failure = Some(SessionFailure::Transport);
    assert_eq!(owner.connect(SECRET).await, Err(SessionFailure::Transport));
    assert_eq!(
        owner.phase(),
        SessionPhase::Failed(SessionFailure::Transport)
    );
    assert!(owner.active.is_none());
    assert_eq!(
        owner.backend.counts.clients_dropped.load(Ordering::SeqCst),
        1
    );
    assert_eq!(owner.backend.calls, ["issue", "issue", "subscribe"]);
}

#[tokio::test]
async fn periodic_renewal_never_reconnects_without_an_eligible_active_lease() {
    let mut owner = owner();
    // A leftover timer cannot turn an initial/no-active owner into a session.
    assert!(!owner.renew_if_due(SECRET).await.unwrap());
    assert!(owner.backend.calls.is_empty());

    active(&mut owner).await;
    owner.disconnect();
    let calls = owner.backend.calls.len();
    assert!(!owner.renew_if_due(SECRET).await.unwrap());
    assert_eq!(owner.phase(), SessionPhase::Disconnected);
    assert_eq!(owner.backend.calls.len(), calls);

    let id = active(&mut owner).await;
    owner.stream_failed(id, SessionFailure::StreamEnded);
    let calls = owner.backend.calls.len();
    assert!(!owner.renew_if_due(SECRET).await.unwrap());
    assert_eq!(
        owner.phase(),
        SessionPhase::Failed(SessionFailure::StreamEnded)
    );
    assert_eq!(owner.backend.calls.len(), calls);

    active(&mut owner).await;
    owner.clock.advance(Duration::from_secs(100));
    assert_eq!(owner.check_lease(), LeaseAction::Reconnect);
    let calls = owner.backend.calls.len();
    assert!(!owner.renew_if_due(SECRET).await.unwrap());
    assert_eq!(owner.phase(), SessionPhase::Expired);
    assert_eq!(owner.backend.calls.len(), calls);
}

#[tokio::test]
async fn failed_renewal_never_restores_old_token_or_subscription() {
    let mut owner = owner();
    active(&mut owner).await;
    owner.clock.advance(Duration::from_secs(80));
    owner.backend.issue_failure = Some(SessionFailure::Transport);
    assert_eq!(
        owner.renew_if_due(SECRET).await,
        Err(SessionFailure::Transport)
    );
    assert!(owner.active.is_none());
    assert_eq!(
        owner.backend.counts.clients_dropped.load(Ordering::SeqCst),
        1
    );
    assert_eq!(
        owner.backend.counts.streams_dropped.load(Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn invalid_leases_fail_without_subscription_and_short_lease_has_fractional_margin() {
    for ttl in [0, u64::MAX] {
        let mut owner = owner();
        owner.backend.ttl = ttl;
        assert_eq!(
            owner.connect(SECRET).await,
            Err(SessionFailure::InvalidLease)
        );
        assert_eq!(owner.backend.calls, ["issue"]);
        assert!(owner.active.is_none());
    }
    let mut owner = owner();
    owner.backend.ttl = 1;
    let start = owner.clock.now();
    owner.connect(SECRET).await.unwrap();
    assert_eq!(owner.renew_at(), Some(start + Duration::from_millis(800)));
}

#[tokio::test]
async fn io_latency_cannot_extend_the_conservative_lease_deadline() {
    let mut owner = owner();
    owner.backend.issue_delay = Duration::from_secs(50);
    owner.backend.stream_delay = Duration::from_secs(51);
    assert_eq!(owner.connect(SECRET).await, Err(SessionFailure::Expired));
    assert!(owner.active.is_none());
    assert_eq!(
        owner.backend.counts.clients_dropped.load(Ordering::SeqCst),
        1
    );
    assert_eq!(
        owner.backend.counts.streams_dropped.load(Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn snapshot_replays_only_newer_metadata_and_preserves_unsequenced_connection() {
    let mut owner = owner();
    let id = owner.connect(SECRET).await.unwrap();
    owner.observe(id, ready("instance-a", 0)).unwrap();
    owner.backend.boundary = boundary("instance-a", 5);
    let ticket = owner.begin_snapshot().await.unwrap();
    assert_eq!(owner.observe(id, change(3)).unwrap(), FrameAction::Buffered);
    assert_eq!(owner.observe(id, change(6)).unwrap(), FrameAction::Buffered);
    let connection = UiLifecycleFrame::Change {
        boundary: None,
        event: LifecycleEvent::Connection {
            layer: ConnectionLayer::UpstreamWebex,
            state: ConnectionState::Disconnected,
        },
    };
    assert_eq!(
        owner.observe(id, connection.clone()).unwrap(),
        FrameAction::Buffered
    );
    assert_eq!(
        owner.finish_snapshot(ticket).unwrap(),
        [change(6), connection]
    );
    assert_eq!(owner.observe(id, change(5)).unwrap(), FrameAction::Covered);
    assert_eq!(
        owner.observe(id, change(7)).unwrap(),
        FrameAction::Apply(change(7))
    );
}

#[tokio::test]
async fn lag_invalidates_inflight_snapshot_and_requires_a_new_whole_state_fence() {
    let mut owner = owner();
    let id = owner.connect(SECRET).await.unwrap();
    owner.observe(id, ready("instance-a", 0)).unwrap();
    let stale = owner.begin_snapshot().await.unwrap();
    assert_eq!(
        owner.observe(id, lag(10)).unwrap(),
        FrameAction::NeedSnapshot {
            scope: ChangeScope::WholeMirror,
            reason: ResyncReason::EventLag
        }
    );
    assert!(matches!(
        owner.finish_snapshot(stale),
        Err(SessionFailure::StaleSnapshot)
    ));
    assert!(owner.active.is_some());
    owner.backend.boundary = boundary("instance-a", 10);
    let current = owner.begin_snapshot().await.unwrap();
    owner.finish_snapshot(current).unwrap();
    assert_eq!(owner.phase(), SessionPhase::Active);
}

#[tokio::test]
async fn changed_daemon_instance_drops_old_session_then_reconnect_marks_restart() {
    let mut owner = owner();
    let old = active(&mut owner).await;
    let frame = UiLifecycleFrame::Unrecognized {
        boundary: Some(boundary("instance-b", 1)),
    };
    assert_eq!(
        owner.observe(old, frame).unwrap(),
        FrameAction::ReconnectRequired
    );
    assert_eq!(owner.phase(), SessionPhase::Disconnected);
    assert!(owner.active.is_none());
    let new = owner.connect(SECRET).await.unwrap();
    assert_eq!(
        owner.observe(new, ready("instance-b", 0)).unwrap(),
        FrameAction::NeedSnapshot {
            scope: ChangeScope::WholeMirror,
            reason: ResyncReason::DaemonRestart
        }
    );
}

#[tokio::test]
async fn malformed_snapshot_boundary_and_snapshot_api_failure_drop_session() {
    for boundary_value in [
        boundary("instance-b", 0),
        boundary("instance-a", 0),
        boundary("", 10),
    ] {
        let mut owner = owner();
        let id = owner.connect(SECRET).await.unwrap();
        owner.observe(id, ready("instance-a", 5)).unwrap();
        owner.backend.boundary = boundary_value;
        assert!(matches!(
            owner.begin_snapshot().await,
            Err(SessionFailure::InvalidContinuity)
        ));
        assert!(owner.active.is_none());
    }
    let mut owner = owner();
    let id = owner.connect(SECRET).await.unwrap();
    owner.observe(id, ready("instance-a", 0)).unwrap();
    owner.backend.snapshot_failure = Some(SessionFailure::AuthRequired);
    assert!(matches!(
        owner.begin_snapshot().await,
        Err(SessionFailure::AuthRequired)
    ));
    assert!(owner.active.is_none());
}

#[tokio::test]
async fn invalid_frame_order_or_unsequenced_content_is_not_exposed() {
    let mut owner = owner();
    let id = owner.connect(SECRET).await.unwrap();
    assert_eq!(
        owner.observe(id, change(1)),
        Err(SessionFailure::InvalidContinuity)
    );
    assert!(owner.active.is_none());
    let mut owner = super::tests::owner();
    let id = active(&mut owner).await;
    assert_eq!(
        owner.observe(id, ready("instance-a", 0)),
        Err(SessionFailure::InvalidContinuity)
    );
    assert!(owner.active.is_none());
    let mut owner = super::tests::owner();
    let id = active(&mut owner).await;
    let invalid = UiLifecycleFrame::Change {
        boundary: None,
        event: LifecycleEvent::RoomInvalidated(RoomId::new("room-a")),
    };
    assert_eq!(
        owner.observe(id, invalid),
        Err(SessionFailure::InvalidContinuity)
    );
    assert!(owner.active.is_none());
}

#[tokio::test]
async fn pending_buffer_overflow_never_silently_commits_a_partial_snapshot() {
    let mut owner = owner();
    let id = owner.connect(SECRET).await.unwrap();
    owner.observe(id, ready("instance-a", 0)).unwrap();
    let stale = owner.begin_snapshot().await.unwrap();
    for watermark in 1..=MAX_PENDING as u64 {
        assert_eq!(
            owner.observe(id, change(watermark)).unwrap(),
            FrameAction::Buffered
        );
    }
    assert_eq!(
        owner.observe(id, change(MAX_PENDING as u64 + 1)).unwrap(),
        FrameAction::NeedSnapshot {
            scope: ChangeScope::WholeMirror,
            reason: ResyncReason::EventLag
        }
    );
    assert_eq!(owner.active.as_ref().unwrap().pending.len(), 0);
    assert!(matches!(
        owner.finish_snapshot(stale),
        Err(SessionFailure::StaleSnapshot)
    ));
    owner.backend.boundary = boundary("instance-a", MAX_PENDING as u64 + 1);
    let current = owner.begin_snapshot().await.unwrap();
    assert!(owner.finish_snapshot(current).unwrap().is_empty());
}

#[tokio::test]
async fn disconnect_drops_owned_client_and_stream_without_stopping_any_daemon() {
    let mut owner = owner();
    active(&mut owner).await;
    owner.disconnect();
    assert_eq!(owner.phase(), SessionPhase::Disconnected);
    assert!(owner.active.is_none());
    assert_eq!(
        owner.backend.counts.clients_dropped.load(Ordering::SeqCst),
        1
    );
    assert_eq!(
        owner.backend.counts.streams_dropped.load(Ordering::SeqCst),
        1
    );
    assert_eq!(owner.backend.calls, ["issue", "subscribe", "continuity"]);
}

#[tokio::test]
async fn old_eof_cannot_close_replacement_but_current_eof_requires_reconnect() {
    let mut owner = owner();
    let old = active(&mut owner).await;
    let replacement = owner.connect(SECRET).await.unwrap();
    assert_eq!(
        owner.stream_failed(old, SessionFailure::StreamEnded),
        FrameAction::StaleSubscription
    );
    assert_eq!(owner.subscription(), Some(replacement));
    assert_eq!(
        owner.stream_failed(replacement, SessionFailure::StreamEnded),
        FrameAction::ReconnectRequired
    );
    assert_eq!(
        owner.phase(),
        SessionPhase::Failed(SessionFailure::StreamEnded)
    );
    assert!(owner.active.is_none());
}

#[tokio::test]
async fn stale_snapshot_from_replaced_subscription_cannot_activate_new_client() {
    let mut owner = owner();
    let old = owner.connect(SECRET).await.unwrap();
    owner.observe(old, ready("instance-a", 0)).unwrap();
    let old_ticket = owner.begin_snapshot().await.unwrap();
    let current = owner.connect(SECRET).await.unwrap();
    owner.observe(current, ready("instance-a", 0)).unwrap();
    assert!(matches!(
        owner.finish_snapshot(old_ticket),
        Err(SessionFailure::StaleSnapshot)
    ));
    assert_eq!(owner.phase(), SessionPhase::SnapshotRequired);
    assert!(owner.active.is_some());
}

#[tokio::test]
async fn subscription_and_snapshot_counters_fail_closed_instead_of_wrapping() {
    let mut owner = owner();
    owner.last_subscription = u64::MAX;
    assert_eq!(
        owner.connect(SECRET).await,
        Err(SessionFailure::CounterExhausted)
    );
    assert!(owner.backend.calls.is_empty());
    let mut owner = super::tests::owner();
    let id = active(&mut owner).await;
    owner.active.as_mut().unwrap().snapshot_generation = u64::MAX;
    assert_eq!(
        owner.observe(id, lag(1)),
        Err(SessionFailure::CounterExhausted)
    );
    assert!(owner.active.is_none());
}

#[tokio::test]
async fn cancelled_connect_reserves_epoch_and_never_leaves_old_handles_exposed() {
    use std::task::{Context, Poll, Wake, Waker};
    struct Noop;
    impl Wake for Noop {
        fn wake(self: Arc<Self>) {}
    }
    let mut owner = owner();
    let old = active(&mut owner).await;
    owner.backend.pending_issue = true;
    let waker = Waker::from(Arc::new(Noop));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(owner.connect(SECRET));
    assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    drop(future);
    assert_eq!(owner.phase(), SessionPhase::Connecting);
    assert!(owner.active.is_none());
    owner.backend.pending_issue = false;
    let replacement = owner.connect(SECRET).await.unwrap();
    assert_eq!(replacement.0, old.0 + 2);
}
