//! Fake-only runtime loop tests: no socket, daemon, Keychain, terminal or App.
//! The loop and a test driver run concurrently on one current-thread runtime.
use super::*;
use crate::{
    bridge::{
        commands::{CommandAdmission, CommandMailbox, ReadAdmission, READ_CAPACITY},
        pairing::UiSecret,
        presentation::{pump, PresentationEndpoint, PumpEvent, PumpHost},
        read_dispatch::{ReadTerminal, ReadUnavailable},
        session::SessionLifecycle,
        write_dispatch::UiWriteBackend,
    },
    frontend::{
        continuity::ContinuityBoundary,
        continuity::DaemonInstanceId,
        data::{ComposedBody, RoomId},
        editor_submission::{
            DraftRevision, DraftSnapshot, DraftStamp, DraftTarget, EditorSubmission,
            NotDispatchedReason, SubmissionTerminal, UncertainReason,
        },
        envelopes::{FrontendIntent, RequestEnvelope, RequestSequence},
    },
};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Instant as StdInstant,
};
use tokio::sync::mpsc;
use webex_bridge::{client::ClientError, dto};

type Item = Option<Result<BridgeEvent, SessionFailure>>;

#[derive(Default)]
struct Shared {
    issues: AtomicUsize,
    subscribes: AtomicUsize,
    writes: AtomicUsize,
    issue_failures: Mutex<VecDeque<SessionFailure>>,
    stream: Mutex<Option<mpsc::UnboundedSender<Item>>>,
    writer_pending: AtomicBool,
    issue_pending: AtomicBool,
}

impl Shared {
    fn push(&self, item: Item) {
        let sender = self.stream.lock().unwrap().clone().expect("subscribed");
        sender.send(item).unwrap();
    }
}

struct FakeStream(mpsc::UnboundedReceiver<Item>);
impl EventSource for FakeStream {
    async fn next_event(&mut self) -> Item {
        match self.0.recv().await {
            Some(item) => item,
            // A replaced subscription simply never yields again.
            None => std::future::pending().await,
        }
    }
}

struct Writer(Arc<Shared>);
impl Writer {
    async fn write(&self) -> Result<dto::Message, ClientError> {
        self.0.writes.fetch_add(1, Ordering::SeqCst);
        if self.0.writer_pending.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        Err(ClientError::Transport(
            std::io::Error::other("synthetic").into(),
        ))
    }
}
impl UiWriteBackend for Writer {
    async fn send(
        &mut self,
        _: &str,
        _: Option<&str>,
        _: &dto::MessageBodyInput,
    ) -> Result<dto::Message, ClientError> {
        self.write().await
    }
    async fn edit(
        &mut self,
        _: &str,
        _: &str,
        _: &dto::MessageBodyInput,
    ) -> Result<dto::Message, ClientError> {
        self.write().await
    }
    async fn delete(&mut self, _: &str) -> Result<(), ClientError> {
        unreachable!("editor writes only")
    }
    async fn leave(&mut self, _: &str) -> Result<(), ClientError> {
        unreachable!("editor writes only")
    }
}

struct Backend(Arc<Shared>);
impl SessionBackend for Backend {
    type Client = Arc<Shared>;
    type Stream = FakeStream;
    type Writer = Writer;
    type SnapshotReader = ();
    async fn issue(&mut self, secret: &str) -> Result<Arc<Shared>, SessionFailure> {
        assert!(!secret.is_empty());
        self.0.issues.fetch_add(1, Ordering::SeqCst);
        if self.0.issue_pending.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        match self.0.issue_failures.lock().unwrap().pop_front() {
            Some(failure) => Err(failure),
            None => Ok(Arc::clone(&self.0)),
        }
    }
    async fn subscribe(&mut self, _: &Arc<Shared>) -> Result<FakeStream, SessionFailure> {
        self.0.subscribes.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = mpsc::unbounded_channel();
        *self.0.stream.lock().unwrap() = Some(sender);
        Ok(FakeStream(receiver))
    }
    async fn continuity(&mut self, _: &Arc<Shared>) -> Result<ContinuityBoundary, SessionFailure> {
        Ok(ContinuityBoundary {
            instance_id: DaemonInstanceId::new("synthetic-instance"),
            watermark: 0,
        })
    }
    fn lease_ttl(_: &Arc<Shared>) -> u64 {
        100
    }
    fn writer(client: &Arc<Shared>) -> Writer {
        Writer(Arc::clone(client))
    }
    fn snapshot_reader(_: &Arc<Shared>) {}
}

#[derive(Clone)]
struct Clock(Arc<Mutex<StdInstant>>);
impl Clock {
    fn advance(&self, seconds: u64) {
        *self.0.lock().unwrap() += std::time::Duration::from_secs(seconds);
    }
}
impl SessionClock for Clock {
    fn now(&self) -> StdInstant {
        *self.0.lock().unwrap()
    }
}

#[derive(Clone, Default)]
struct Secrets {
    loads: Arc<AtomicUsize>,
    missing: Arc<AtomicBool>,
    denied: Arc<AtomicBool>,
    /// Number of upcoming loads that fail as transiently Unavailable.
    unavailable: Arc<AtomicUsize>,
}
impl UiSecretStore for Secrets {
    async fn load(&mut self) -> Result<Option<UiSecret>, SecretStoreFailure> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        if self.missing.load(Ordering::SeqCst) {
            return Ok(None);
        }
        if self.denied.load(Ordering::SeqCst) {
            return Err(SecretStoreFailure::Denied);
        }
        if self
            .unavailable
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(SecretStoreFailure::Unavailable);
        }
        // 43 base64url characters encode 32 zero bytes: a canonical secret.
        UiSecret::from_persisted("A".repeat(43)).map(Some)
    }
    async fn persist(&mut self, _: &UiSecret) -> Result<(), SecretStoreFailure> {
        unreachable!("the runtime never creates or persists secrets")
    }
}

#[derive(Clone, Default)]
struct Snapshots {
    pending: Arc<AtomicBool>,
    builds: Arc<AtomicUsize>,
    failures: Arc<Mutex<VecDeque<SessionFailure>>>,
}
impl SnapshotSource<Backend> for Snapshots {
    type Snapshot = &'static str;
    async fn build(&mut self, _: &()) -> Result<&'static str, SessionFailure> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        if self.pending.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        match self.failures.lock().unwrap().pop_front() {
            Some(failure) => Err(failure),
            None => Ok("snapshot"),
        }
    }
}

type TestRuntime = Runtime<Backend, Clock, PumpHost<&'static str>, Secrets, Snapshots>;

/// Everything the test driver observes; the runtime itself is returned apart.
struct Fixture {
    control: RuntimeControl,
    mailbox: CommandMailbox,
    endpoint: PresentationEndpoint<&'static str>,
    shared: Arc<Shared>,
    clock: Clock,
    secrets: Secrets,
    snapshots: Snapshots,
}

fn config() -> RuntimeConfig {
    RuntimeConfig {
        tick: Duration::from_millis(5),
        connect_deadline: Duration::from_millis(500),
        snapshot_deadline: Duration::from_millis(500),
        backoff_min: Duration::from_millis(10),
        backoff_max: Duration::from_millis(40),
    }
}

fn fixture() -> (TestRuntime, Fixture) {
    let shared = Arc::new(Shared::default());
    let clock = Clock(Arc::new(Mutex::new(StdInstant::now())));
    let (host, endpoint) = pump();
    let (mailbox, worker) = CommandWorker::new(
        SessionLifecycle::new(Backend(Arc::clone(&shared)), clock.clone()),
        host,
    );
    let secrets = Secrets::default();
    let snapshots = Snapshots::default();
    let (runtime, control) = Runtime::new(worker, secrets.clone(), snapshots.clone(), config());
    (
        runtime,
        Fixture {
            control,
            mailbox,
            endpoint,
            shared,
            clock,
            secrets,
            snapshots,
        },
    )
}

fn ready() -> Item {
    Some(Ok(BridgeEvent {
        name: "continuity.ready".into(),
        data: json!({"instanceId": "synthetic-instance", "watermark": 0}),
    }))
}

/// Poll `condition` while the runtime makes progress on the same thread.
async fn until(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..2000 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("timed out waiting for {what}");
}

/// Drain until the latest snapshot appears; ack it and wake the runtime.
async fn ack_next_snapshot(f: &Fixture) {
    let mut install = None;
    until("snapshot install", || {
        for event in f.endpoint.drain(usize::MAX) {
            if let PumpEvent::Snapshot { install: id, .. } = event {
                install = Some(id);
            }
        }
        install.is_some()
    })
    .await;
    assert!(f.endpoint.ack(install.unwrap()));
    f.control.wake();
}

fn write_request() -> RequestEnvelope {
    EditorSubmission::default()
        .prepare(&DraftSnapshot {
            stamp: DraftStamp {
                target: DraftTarget::Send {
                    room_id: RoomId::new("room/a?opaque"),
                },
                revision: DraftRevision::default(),
            },
            body: ComposedBody::PlainText("private body".into()),
        })
        .unwrap()
}

fn read_request() -> RequestEnvelope {
    RequestSequence::default()
        .begin(FrontendIntent::ListAllRooms, None)
        .unwrap()
}

async fn submit_until_accepted(f: &Fixture) {
    until("write admission", || {
        f.mailbox.try_submit(write_request()) == CommandAdmission::Accepted
    })
    .await;
    f.control.wake();
}

async fn connect_and_install(f: &Fixture, subscribes: usize) {
    until("subscription", || {
        f.shared.subscribes.load(Ordering::SeqCst) == subscribes
    })
    .await;
    f.shared.push(ready());
    ack_next_snapshot(f).await;
}

fn read_terminals(f: &Fixture) -> Vec<ReadTerminal> {
    f.endpoint
        .drain(usize::MAX)
        .into_iter()
        .filter_map(|event| match event {
            PumpEvent::Read { terminal, .. } => Some(terminal),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn connects_installs_behind_ack_dispatches_one_write_and_shuts_down() {
    let (runtime, f) = fixture();
    let driver = async {
        until("first subscription", || {
            f.shared.subscribes.load(Ordering::SeqCst) == 1
        })
        .await;
        // Disconnected worker: the first connect is immediate, one secret borrow.
        assert_eq!(f.secrets.loads.load(Ordering::SeqCst), 1);
        f.shared.push(ready());
        // Snapshot installed but not acknowledged: writes stay closed.
        until("snapshot build", || {
            f.snapshots.builds.load(Ordering::SeqCst) == 1
        })
        .await;
        assert_eq!(
            f.mailbox.try_submit(write_request()),
            CommandAdmission::NotReady
        );
        ack_next_snapshot(&f).await;
        submit_until_accepted(&f).await;
        let mut terminal = None;
        until("write terminal", || {
            terminal = f.mailbox.drain();
            terminal.is_some()
        })
        .await;
        assert!(matches!(terminal, Some(SubmissionTerminal::Result(_))));
        f.control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
    assert_eq!(f.shared.writes.load(Ordering::SeqCst), 1, "never replayed");
    assert_eq!(f.secrets.loads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn due_renewal_reborrows_the_secret_and_resnapshots() {
    let (runtime, f) = fixture();
    let driver = async {
        connect_and_install(&f, 1).await;
        submit_until_accepted(&f).await;
        until("write terminal", || f.mailbox.drain().is_some()).await;
        // Past the 80% renewal point of the 100 s fake lease.
        f.clock.advance(85);
        until("renewal subscription", || {
            f.shared.subscribes.load(Ordering::SeqCst) == 2
        })
        .await;
        assert_eq!(f.secrets.loads.load(Ordering::SeqCst), 2, "one re-borrow");
        assert_eq!(f.shared.issues.load(Ordering::SeqCst), 2);
        // Renewed session is not writable until its own snapshot is acked.
        assert_eq!(
            f.mailbox.try_submit(write_request()),
            CommandAdmission::NotReady
        );
        f.shared.push(ready());
        ack_next_snapshot(&f).await;
        assert_eq!(f.snapshots.builds.load(Ordering::SeqCst), 2);
        submit_until_accepted(&f).await;
        f.control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
    // The second write was admitted; shutdown settled it without replay.
    let terminal = f.mailbox.drain();
    assert!(terminal.is_some());
    assert!(f.shared.writes.load(Ordering::SeqCst) <= 2);
}

#[tokio::test]
async fn stream_end_reconnects_with_bounded_backoff() {
    let (runtime, f) = fixture();
    let driver = async {
        connect_and_install(&f, 1).await;
        f.shared
            .issue_failures
            .lock()
            .unwrap()
            .extend([SessionFailure::Transport, SessionFailure::Transport]);
        let started = tokio::time::Instant::now();
        f.shared.push(None); // clean EOF
        until("reconnected subscription", || {
            f.shared.subscribes.load(Ordering::SeqCst) == 2
        })
        .await;
        // Immediate attempt + two failures with 10 ms and 20 ms backoff.
        assert_eq!(f.shared.issues.load(Ordering::SeqCst), 4);
        assert!(started.elapsed() >= Duration::from_millis(30));
        assert_eq!(f.secrets.loads.load(Ordering::SeqCst), 4);
        f.shared.push(ready());
        ack_next_snapshot(&f).await;
        f.control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn failed_snapshot_ends_the_subscription_and_reconnects() {
    let (runtime, f) = fixture();
    f.snapshots
        .failures
        .lock()
        .unwrap()
        .push_back(SessionFailure::Transport);
    let driver = async {
        until("subscription", || {
            f.shared.subscribes.load(Ordering::SeqCst) == 1
        })
        .await;
        f.shared.push(ready());
        until("retry subscription", || {
            f.shared.subscribes.load(Ordering::SeqCst) == 2
        })
        .await;
        f.shared.push(ready());
        ack_next_snapshot(&f).await;
        assert_eq!(f.snapshots.builds.load(Ordering::SeqCst), 2);
        f.control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn refused_secret_exits_pairing_required_and_settles_reads() {
    let (runtime, f) = fixture();
    f.shared
        .issue_failures
        .lock()
        .unwrap()
        .push_back(SessionFailure::AuthRequired);
    let read = read_request();
    let context = read.context.clone();
    assert_eq!(f.mailbox.try_read(read), ReadAdmission::Accepted);
    let exit = runtime.run().await;
    assert_eq!(
        exit,
        RuntimeExit::PairingRequired(SessionFailure::AuthRequired)
    );
    let terminals = read_terminals(&f);
    assert_eq!(terminals.len(), 1);
    assert_eq!(terminals[0].context(), &context);
    assert!(matches!(
        terminals[0],
        ReadTerminal::Unavailable {
            reason: ReadUnavailable::Shutdown,
            ..
        }
    ));
    assert_eq!(f.mailbox.try_read(read_request()), ReadAdmission::Closed);
}

#[tokio::test]
async fn missing_secret_at_renewal_exits_without_creating_one() {
    let (runtime, f) = fixture();
    let driver = async {
        connect_and_install(&f, 1).await;
        f.secrets.missing.store(true, Ordering::SeqCst);
        f.clock.advance(85);
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::SecretUnavailable);
    assert_eq!(
        f.shared.issues.load(Ordering::SeqCst),
        1,
        "no renewal attempt"
    );
}

#[tokio::test]
async fn shutdown_before_dispatch_settles_queued_write_and_read_once() {
    let (runtime, f) = fixture();
    let driver = async {
        connect_and_install(&f, 1).await;
        // Hold the loop inside a pending write so later work stays queued.
        f.shared.writer_pending.store(true, Ordering::SeqCst);
        submit_until_accepted(&f).await;
        until("write attempt", || {
            f.shared.writes.load(Ordering::SeqCst) == 1
        })
        .await;
        let read = read_request();
        let context = read.context.clone();
        assert_eq!(f.mailbox.try_read(read), ReadAdmission::Accepted);
        context
    };
    // Drop the running loop mid-write, as an owner interruption would.
    let context = tokio::select! {
        exit = runtime.run() => panic!("runtime exited early: {exit:?}"),
        context = driver => context,
    };
    let terminal = f.mailbox.drain().expect("in-flight write settled");
    assert!(matches!(
        terminal,
        SubmissionTerminal::Uncertain {
            reason: UncertainReason::AttemptInterrupted,
            ..
        }
    ));
    let terminals = read_terminals(&f);
    assert_eq!(terminals.len(), 1);
    assert_eq!(terminals[0].context(), &context);
    assert!(matches!(
        terminals[0],
        ReadTerminal::Unavailable {
            reason: ReadUnavailable::Shutdown,
            ..
        }
    ));
    assert!(f.mailbox.drain().is_none(), "exactly once");
    assert_eq!(f.shared.writes.load(Ordering::SeqCst), 1, "never replayed");
}

#[tokio::test]
async fn shutdown_racing_an_admitted_write_settles_it_unstarted_exactly_once() {
    let (runtime, f) = fixture();
    let driver = async {
        connect_and_install(&f, 1).await;
        // Admit a write and request shutdown in the same synchronous step, so
        // the loop observes shutdown before it can dispatch the write.
        until("write admission", || {
            let accepted = f.mailbox.try_submit(write_request()) == CommandAdmission::Accepted;
            if accepted {
                f.control.shutdown();
            }
            accepted
        })
        .await;
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
    let terminal = f.mailbox.drain().expect("settled");
    assert!(matches!(
        terminal,
        SubmissionTerminal::NotDispatched {
            reason: NotDispatchedReason::UnstartedShutdown,
            ..
        }
    ));
    assert!(f.mailbox.drain().is_none(), "exactly once");
    assert_eq!(f.shared.writes.load(Ordering::SeqCst), 0, "never sent");
}

#[test]
fn backoff_is_immediate_then_exponential_and_capped() {
    let (mut runtime, _f) = fixture();
    let delays: Vec<_> = (0..6)
        .map(|attempts| {
            runtime.attempts = attempts;
            runtime.backoff()
        })
        .collect();
    assert_eq!(
        delays,
        [0, 10, 20, 40, 40, 40].map(Duration::from_millis).to_vec()
    );
    runtime.attempts = u32::MAX;
    assert_eq!(runtime.backoff(), Duration::from_millis(40));
}

/// Compile-time: the loop over the real bridge backend and real snapshot
/// source is spawnable later.
#[allow(dead_code)]
fn real_bridge_runtime_future_is_send(
    runtime: Runtime<
        crate::bridge::session::BridgeSessionBackend,
        crate::bridge::session::SystemClock,
        PumpHost<crate::bridge::snapshot::BridgeSnapshot>,
        Secrets,
        crate::bridge::snapshot::BridgeSnapshotSource,
    >,
) {
    fn assert_send<T: Send>(_: &T) {}
    let run = runtime.run();
    assert_send(&run);
}

#[test]
fn fake_runtime_future_is_send() {
    fn assert_send<T: Send>(_: &T) {}
    let (runtime, _f) = fixture();
    let run = runtime.run();
    assert_send(&run);
}

#[tokio::test]
async fn reads_are_served_in_order_and_stream_errors_reconnect() {
    let (runtime, f) = fixture();
    let driver = async {
        connect_and_install(&f, 1).await;
        let read = read_request();
        let context = read.context.clone();
        assert_eq!(f.mailbox.try_read(read), ReadAdmission::Accepted);
        f.control.wake();
        let mut terminals = Vec::new();
        until("read terminal", || {
            terminals.extend(read_terminals(&f));
            !terminals.is_empty()
        })
        .await;
        assert_eq!(terminals[0].context(), &context);
        // The inert snapshot reader rejects, but the read is a typed result
        // of the current install, not a session failure.
        assert!(matches!(terminals[0], ReadTerminal::Result(_)));
        // A transport error on SSE ends the subscription and reconnects.
        f.shared.push(Some(Err(SessionFailure::Transport)));
        until("reconnect after stream error", || {
            f.shared.subscribes.load(Ordering::SeqCst) == 2
        })
        .await;
        assert_eq!(f.secrets.loads.load(Ordering::SeqCst), 2);
        f.control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn sustained_sse_traffic_cannot_starve_lease_renewal() {
    let (runtime, f) = fixture();
    let driver = async {
        connect_and_install(&f, 1).await;
        // Past the renewal point, then a steady stream of events 1 ms apart
        // (faster than the 5 ms tick) for longer than many ticks.
        f.clock.advance(85);
        let started = tokio::time::Instant::now();
        let mut watermark = 1;
        while f.shared.subscribes.load(Ordering::SeqCst) < 2 {
            assert!(
                started.elapsed() < Duration::from_millis(500),
                "renewal starved by SSE traffic"
            );
            f.shared.push(Some(Ok(BridgeEvent {
                name: "room.updated".into(),
                data: json!({
                    "roomId": "room/a?opaque",
                    "continuity": {"instanceId": "synthetic-instance", "watermark": watermark},
                }),
            })));
            watermark += 1;
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(f.secrets.loads.load(Ordering::SeqCst), 2);
        f.control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn failures_right_after_active_keep_backing_off() {
    let (runtime, f) = fixture();
    let driver = async {
        let mut connected_at = Vec::new();
        for subscription in 1..=4 {
            connect_and_install(&f, subscription).await;
            connected_at.push(tokio::time::Instant::now());
            // The stream ends immediately after becoming Active (crash loop).
            f.shared.push(None);
        }
        // Immediate first reconnect, then 10 ms and 20 ms backoff: a hot
        // loop would reconnect without any delay every time.
        let gaps: Vec<_> = connected_at.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps[1] >= Duration::from_millis(10), "{gaps:?}");
        assert!(gaps[2] >= Duration::from_millis(20), "{gaps:?}");
        f.control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn backoff_resets_after_staying_active_for_backoff_max() {
    let (runtime, f) = fixture();
    let driver = async {
        connect_and_install(&f, 1).await;
        f.shared.push(None);
        connect_and_install(&f, 2).await;
        // Stay Active longer than backoff_max (40 ms), then fail again: the
        // reconnect is immediate once more.
        tokio::time::sleep(Duration::from_millis(60)).await;
        let failed = tokio::time::Instant::now();
        f.shared.push(None);
        until("immediate reconnect", || {
            f.shared.subscribes.load(Ordering::SeqCst) == 3
        })
        .await;
        assert!(failed.elapsed() < Duration::from_millis(10));
        f.control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn shutdown_abandons_a_pending_connect_or_snapshot_promptly() {
    for stage in ["connect", "snapshot"] {
        let (runtime, f) = fixture();
        if stage == "connect" {
            f.shared.issue_pending.store(true, Ordering::SeqCst);
        } else {
            f.snapshots.pending.store(true, Ordering::SeqCst);
        }
        let read = read_request();
        assert_eq!(f.mailbox.try_read(read), ReadAdmission::Accepted);
        let driver = async {
            if stage == "connect" {
                until("pending issue", || {
                    f.shared.issues.load(Ordering::SeqCst) == 1
                })
                .await;
            } else {
                until("subscription", || {
                    f.shared.subscribes.load(Ordering::SeqCst) == 1
                })
                .await;
                f.shared.push(ready());
                until("pending build", || {
                    f.snapshots.builds.load(Ordering::SeqCst) == 1
                })
                .await;
            }
            let requested = tokio::time::Instant::now();
            f.control.shutdown();
            requested
        };
        let (exit, requested) = tokio::join!(runtime.run(), driver);
        assert_eq!(exit, RuntimeExit::Shutdown, "{stage}");
        assert!(
            requested.elapsed() < Duration::from_millis(100),
            "{stage}: shutdown waited for the deadline"
        );
        // Admitted reads are still settled exactly once.
        let terminals = read_terminals(&f);
        assert_eq!(terminals.len(), 1, "{stage}");
        // Nothing was installed by an abandoned snapshot.
        let installed = f
            .endpoint
            .drain(usize::MAX)
            .into_iter()
            .any(|event| matches!(event, PumpEvent::Snapshot { .. }));
        assert!(!installed, "{stage}");
    }
}

#[tokio::test]
async fn transient_secret_store_failures_are_retried_not_fatal() {
    let (runtime, f) = fixture();
    let driver = async {
        connect_and_install(&f, 1).await;
        // Renewal while the store is briefly unavailable (locked keychain).
        f.secrets.unavailable.store(3, Ordering::SeqCst);
        f.clock.advance(85);
        until("renewal after retries", || {
            f.shared.subscribes.load(Ordering::SeqCst) == 2
        })
        .await;
        assert_eq!(f.secrets.loads.load(Ordering::SeqCst), 1 + 3 + 1);
        f.shared.push(ready());
        ack_next_snapshot(&f).await;
        // Reconnect path: transient failures back off and retry as well.
        f.secrets.unavailable.store(2, Ordering::SeqCst);
        f.shared.push(None);
        until("reconnect after retries", || {
            f.shared.subscribes.load(Ordering::SeqCst) == 3
        })
        .await;
        f.control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn denied_secret_at_renewal_exits() {
    let (runtime, f) = fixture();
    let driver = async {
        connect_and_install(&f, 1).await;
        f.secrets.denied.store(true, Ordering::SeqCst);
        f.clock.advance(85);
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::SecretUnavailable);
}

#[tokio::test]
async fn reads_wait_for_active_instead_of_failing_not_ready() {
    let (runtime, f) = fixture();
    let driver = async {
        until("subscription", || {
            f.shared.subscribes.load(Ordering::SeqCst) == 1
        })
        .await;
        // AwaitingReady: the read is held, not answered NotReady.
        let read = read_request();
        let context = read.context.clone();
        assert_eq!(f.mailbox.try_read(read), ReadAdmission::Accepted);
        f.control.wake();
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert!(read_terminals(&f).is_empty());
        f.shared.push(ready());
        let mut terminals = Vec::new();
        until("held read served", || {
            terminals.extend(read_terminals(&f));
            !terminals.is_empty()
        })
        .await;
        assert_eq!(terminals[0].context(), &context);
        assert!(matches!(terminals[0], ReadTerminal::Result(_)));
        f.control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn a_full_read_queue_does_not_starve_sse() {
    let (runtime, f) = fixture();
    let driver = async {
        connect_and_install(&f, 1).await;
        for index in 0..READ_CAPACITY {
            let request = RequestSequence::default()
                .begin(
                    FrontendIntent::UpdateRoom(RoomId::new(format!("room-{index}"))),
                    None,
                )
                .unwrap();
            assert_eq!(f.mailbox.try_read(request), ReadAdmission::Accepted);
        }
        f.shared.push(Some(Ok(BridgeEvent {
            name: "room.updated".into(),
            data: json!({
                "roomId": "room/a?opaque",
                "continuity": {"instanceId": "synthetic-instance", "watermark": 1},
            }),
        })));
        f.control.wake();
        let mut kinds = Vec::new();
        until("all reads served", || {
            for event in f.endpoint.drain(usize::MAX) {
                kinds.push(match event {
                    PumpEvent::Read { .. } => "read",
                    PumpEvent::Lifecycle { .. } => "lifecycle",
                    _ => "other",
                });
            }
            kinds.iter().filter(|kind| **kind == "read").count() == READ_CAPACITY
        })
        .await;
        let event_at = kinds.iter().rposition(|kind| *kind == "lifecycle").unwrap();
        let last_read = kinds.iter().rposition(|kind| *kind == "read").unwrap();
        assert!(
            event_at < last_read,
            "SSE waited behind every read: {kinds:?}"
        );
        f.control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}
