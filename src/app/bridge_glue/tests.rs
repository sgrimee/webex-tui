//! Fake-backed glue tests: the real runtime loop, pump, mailbox and snapshot
//! source run against a scripted in-memory bridge. No socket, daemon,
//! Keychain or terminal.
use super::*;
use crate::bridge::{
    commands::CommandWorker,
    pairing::{SecretStoreFailure, UiSecret, UiSecretStore},
    presentation::{pump, PumpHost},
    read_dispatch::UiReadBackend,
    runtime::{EventSource, Runtime, RuntimeConfig, RuntimeExit},
    session::{SessionBackend, SessionClock, SessionFailure, SessionLifecycle},
    snapshot::BridgeSnapshotSource,
    write_dispatch::UiWriteBackend,
};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use webex_bridge::{
    client::{events::BridgeEvent, ClientError},
    dto,
};

type Reply<T> = Result<T, ClientError>;
type Item = Option<Result<BridgeEvent, SessionFailure>>;

const ROOM_A: &str = "room/a?opaque";
const ROOM_B: &str = "room/b?opaque";

fn message(id: &str, room: &str, sender: &str, minute: u32, revision: i64) -> dto::Message {
    dto::Message {
        id: id.into(),
        room_id: room.into(),
        revision,
        parent_id: None,
        sender_id: Some(sender.into()),
        created_at: Some(format!("2026-01-01T00:{minute:02}:00Z")),
        updated_at: None,
        status: "active".into(),
        markdown: None,
        body: Some(dto::MessageBody {
            kind: "plainText".into(),
            content: format!("synthetic {id}"),
        }),
        attachment_count: 0,
    }
}

fn room(id: &str) -> dto::Room {
    dto::Room {
        id: id.into(),
        title: Some(format!("title {id}")),
        room_type: "group".into(),
        team_id: None,
        last_activity: Some("2026-01-01T00:00:00Z".into()),
        is_member: true,
        is_locked: Some(false),
    }
}

#[derive(Default)]
struct Shared {
    subscribes: AtomicUsize,
    page_reads: AtomicUsize,
    message_reads: AtomicUsize,
    person_reads: AtomicUsize,
    pages_pending: AtomicBool,
    watermark: AtomicU64,
    stream: Mutex<Option<mpsc::UnboundedSender<Item>>>,
    rooms: Mutex<Vec<dto::Room>>,
    pages: Mutex<HashMap<String, Vec<dto::Message>>>,
    messages: Mutex<HashMap<String, dto::Message>>,
}

impl Shared {
    fn push(&self, name: &str, mut data: Value) {
        let watermark = self.watermark.fetch_add(1, Ordering::SeqCst) + 1;
        let marker = json!({"instanceId": "synthetic-instance", "watermark": watermark});
        if name == "continuity.ready" || name == "resync.required" {
            data["instanceId"] = marker["instanceId"].clone();
            data["watermark"] = marker["watermark"].clone();
        } else {
            data["continuity"] = marker;
        }
        let sender = self.stream.lock().unwrap().clone().expect("subscribed");
        sender
            .send(Some(Ok(BridgeEvent {
                name: name.into(),
                data,
            })))
            .unwrap();
    }
}

#[derive(Clone)]
struct Reader(Arc<Shared>);
impl UiReadBackend for Reader {
    async fn me(&self) -> Reply<dto::Person> {
        Ok(dto::Person {
            id: "me".into(),
            display_name: Some("Me".into()),
            emails: None,
        })
    }
    async fn rooms(&self) -> Reply<Vec<dto::Room>> {
        Ok(self.0.rooms.lock().unwrap().clone())
    }
    async fn room(&self, id: &str) -> Reply<dto::Room> {
        Ok(room(id))
    }
    async fn team(&self, _: &str) -> Reply<dto::Team> {
        unreachable!("no teams in fixtures")
    }
    async fn person_read(&self, id: &str) -> Reply<dto::PersonRead> {
        self.0.person_reads.fetch_add(1, Ordering::SeqCst);
        Ok(dto::PersonRead {
            id: id.into(),
            state: "mirrored".into(),
            person: Some(dto::Person {
                id: id.into(),
                display_name: Some(format!("name {id}")),
                emails: None,
            }),
        })
    }
    async fn room_messages_page(
        &self,
        room: &str,
        _: Option<&str>,
        _: Option<u32>,
    ) -> Reply<dto::MessagePage> {
        self.0.page_reads.fetch_add(1, Ordering::SeqCst);
        if self.0.pages_pending.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        Ok(dto::MessagePage {
            messages: self
                .0
                .pages
                .lock()
                .unwrap()
                .get(room)
                .cloned()
                .unwrap_or_default(),
            next_before: None,
            complete: true,
            partial_reason: None,
        })
    }
    async fn message_read(&self, id: &str) -> Reply<dto::MessageRead> {
        self.0.message_reads.fetch_add(1, Ordering::SeqCst);
        let message = self.0.messages.lock().unwrap().get(id).cloned();
        Ok(match message {
            Some(message) => dto::MessageRead {
                id: id.into(),
                state: "mirrored".into(),
                message: Some(message),
            },
            None => dto::MessageRead {
                id: id.into(),
                state: "not_found".into(),
                message: None,
            },
        })
    }
    async fn thread_children(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<u32>,
    ) -> Reply<dto::ThreadChildren> {
        unreachable!("threads are not exercised here")
    }
}

struct FakeStream(mpsc::UnboundedReceiver<Item>);
impl EventSource for FakeStream {
    async fn next_event(&mut self) -> Item {
        match self.0.recv().await {
            Some(item) => item,
            None => std::future::pending().await,
        }
    }
}

struct NoWrites;
impl UiWriteBackend for NoWrites {
    async fn send(
        &mut self,
        _: &str,
        _: Option<&str>,
        _: &dto::MessageBodyInput,
    ) -> Reply<dto::Message> {
        unreachable!("the glue never writes")
    }
    async fn edit(&mut self, _: &str, _: &str, _: &dto::MessageBodyInput) -> Reply<dto::Message> {
        unreachable!("the glue never writes")
    }
    async fn delete(&mut self, _: &str) -> Reply<()> {
        unreachable!("the glue never writes")
    }
    async fn leave(&mut self, _: &str) -> Reply<()> {
        unreachable!("the glue never writes")
    }
}

struct Backend(Arc<Shared>);
impl SessionBackend for Backend {
    type Client = Arc<Shared>;
    type Stream = FakeStream;
    type Writer = NoWrites;
    type SnapshotReader = Reader;
    async fn issue(&mut self, _: &str) -> Result<Arc<Shared>, SessionFailure> {
        Ok(Arc::clone(&self.0))
    }
    async fn subscribe(&mut self, _: &Arc<Shared>) -> Result<FakeStream, SessionFailure> {
        self.0.subscribes.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = mpsc::unbounded_channel();
        *self.0.stream.lock().unwrap() = Some(sender);
        Ok(FakeStream(receiver))
    }
    async fn continuity(
        &mut self,
        _: &Arc<Shared>,
    ) -> Result<crate::frontend::continuity::ContinuityBoundary, SessionFailure> {
        Ok(crate::frontend::continuity::ContinuityBoundary {
            instance_id: crate::frontend::continuity::DaemonInstanceId::new("synthetic-instance"),
            watermark: self.0.watermark.load(Ordering::SeqCst),
        })
    }
    fn lease_ttl(_: &Arc<Shared>) -> u64 {
        600
    }
    fn writer(_: &Arc<Shared>) -> NoWrites {
        NoWrites
    }
    fn snapshot_reader(client: &Arc<Shared>) -> Reader {
        Reader(Arc::clone(client))
    }
}

#[derive(Clone)]
struct Clock(Instant);
impl SessionClock for Clock {
    fn now(&self) -> Instant {
        self.0
    }
}

struct Secrets;
impl UiSecretStore for Secrets {
    async fn load(&mut self) -> Result<Option<UiSecret>, SecretStoreFailure> {
        UiSecret::from_persisted("A".repeat(43)).map(Some)
    }
    async fn persist(&mut self, _: &UiSecret) -> Result<(), SecretStoreFailure> {
        unreachable!("the glue never creates secrets")
    }
}

type TestRuntime = Runtime<Backend, Clock, PumpHost<BridgeSnapshot>, Secrets, BridgeSnapshotSource>;

fn fixture(
    read_deadline: Option<Duration>,
) -> (TestRuntime, BridgeGlue, RuntimeControl, Arc<Shared>) {
    let shared = Arc::new(Shared::default());
    *shared.rooms.lock().unwrap() = vec![room(ROOM_A), room(ROOM_B)];
    shared.pages.lock().unwrap().insert(
        ROOM_A.into(),
        vec![
            message("a1", ROOM_A, "alice", 1, 1),
            message("a2", ROOM_A, "me", 2, 2),
        ],
    );
    let (host, endpoint) = pump();
    let (mailbox, mut worker) = CommandWorker::new(
        SessionLifecycle::new(Backend(Arc::clone(&shared)), Clock(Instant::now())),
        host,
    );
    if let Some(deadline) = read_deadline {
        worker.set_read_deadline(deadline);
    }
    let config = RuntimeConfig {
        tick: Duration::from_millis(5),
        connect_deadline: Duration::from_millis(500),
        snapshot_deadline: Duration::from_millis(500),
        backoff_min: Duration::from_millis(10),
        backoff_max: Duration::from_millis(40),
    };
    let (runtime, control) = Runtime::new(worker, Secrets, BridgeSnapshotSource::default(), config);
    let glue = BridgeGlue::new(endpoint, mailbox, control.clone(), 50);
    (runtime, glue, control, shared)
}

/// Drain the glue while the runtime makes progress, until `condition` holds.
async fn settle(
    what: &str,
    glue: &mut BridgeGlue,
    state: &mut AppState<'_>,
    mut condition: impl FnMut(&BridgeGlue, &AppState) -> bool,
) {
    for _ in 0..2000 {
        glue.drain(state, usize::MAX);
        if condition(glue, state) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("timed out waiting for {what}");
}

fn ids(state: &AppState, room: &str) -> Vec<String> {
    state
        .cache
        .messages_in_room(&room.to_owned())
        .map(|message| message.id.as_str().to_owned())
        .collect()
}

async fn ready(glue: &mut BridgeGlue, state: &mut AppState<'_>, shared: &Shared) {
    let subscribes = shared.subscribes.load(Ordering::SeqCst);
    for _ in 0..2000 {
        if shared.stream.lock().unwrap().is_some() && subscribes > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    shared.push("continuity.ready", json!({}));
    settle("snapshot installed", glue, state, |_, state| {
        state.cache.rooms.room_with_id(&ROOM_A.to_owned()).is_some()
    })
    .await;
}

async fn wait_subscribed(shared: &Shared, count: usize) {
    for _ in 0..2000 {
        if shared.subscribes.load(Ordering::SeqCst) >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("timed out waiting for subscription {count}");
}

#[tokio::test]
async fn snapshot_installs_identity_and_rooms_then_open_room_loads_its_page() {
    let (runtime, mut glue, control, shared) = fixture(None);
    let mut state = AppState::default();
    let driver = async {
        wait_subscribed(&shared, 1).await;
        ready(&mut glue, &mut state, &shared).await;
        assert!(state.cache.is_me(&Some("me".into())));
        assert!(state.cache.rooms.room_with_id(&ROOM_B.to_owned()).is_some());
        // Nothing is loaded until the user opens a room.
        assert_eq!(shared.page_reads.load(Ordering::SeqCst), 0);
        glue.open_room(&mut state, &ROOM_A.to_owned());
        settle("page", &mut glue, &mut state, |_, state| {
            ids(state, ROOM_A).len() == 2
        })
        .await;
        assert_eq!(ids(&state, ROOM_A), ["a1", "a2"]);
        // Unknown senders are requested once.
        settle("persons", &mut glue, &mut state, |_, state| {
            state.cache.persons.get("alice").is_some()
        })
        .await;
        // Opening again does not re-request.
        glue.open_room(&mut state, &ROOM_A.to_owned());
        glue.drain(&mut state, usize::MAX);
        assert_eq!(shared.page_reads.load(Ordering::SeqCst), 1);
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn unread_comes_only_from_created_messages_of_others_in_inactive_rooms() {
    let (runtime, mut glue, control, shared) = fixture(None);
    let mut state = AppState::default();
    let driver = async {
        wait_subscribed(&shared, 1).await;
        ready(&mut glue, &mut state, &shared).await;
        state.rooms_list.set_active_room_id(Some(ROOM_A.into()));
        glue.open_room(&mut state, &ROOM_A.to_owned());
        settle("page", &mut glue, &mut state, |_, state| {
            ids(state, ROOM_A).len() == 2
        })
        .await;
        let unread = |state: &AppState, room: &str| {
            state
                .cache
                .rooms
                .room_with_id(&room.to_owned())
                .unwrap()
                .unread
        };
        // Loading pages never marks anything unread.
        assert!(!unread(&state, ROOM_A) && !unread(&state, ROOM_B));

        // Own message created in an inactive room: not unread.
        shared
            .messages
            .lock()
            .unwrap()
            .insert("b-mine".into(), message("b-mine", ROOM_B, "me", 3, 1));
        shared.push(
            "message.created",
            json!({"roomId": ROOM_B, "messageId": "b-mine", "revision": 1}),
        );
        settle("own created", &mut glue, &mut state, |glue, _| {
            shared.message_reads.load(Ordering::SeqCst) >= 1 && glue.outstanding_reads() == 0
        })
        .await;
        assert!(!unread(&state, ROOM_B));

        // Someone else's message in an inactive room: unread.
        shared
            .messages
            .lock()
            .unwrap()
            .insert("b-theirs".into(), message("b-theirs", ROOM_B, "bob", 4, 1));
        shared.push(
            "message.created",
            json!({"roomId": ROOM_B, "messageId": "b-theirs", "revision": 1}),
        );
        settle("their created", &mut glue, &mut state, |_, state| {
            unread(state, ROOM_B)
        })
        .await;
        // The inactive room was not loaded by the unread check.
        assert!(state.cache.canonical_view(&ROOM_B.to_owned()).is_none());

        // Someone else's message in the active loaded room: shown, not unread.
        shared
            .messages
            .lock()
            .unwrap()
            .insert("a3".into(), message("a3", ROOM_A, "bob", 5, 1));
        shared.push(
            "message.created",
            json!({"roomId": ROOM_A, "messageId": "a3", "revision": 1}),
        );
        settle("active created", &mut glue, &mut state, |_, state| {
            ids(state, ROOM_A).contains(&"a3".to_owned())
        })
        .await;
        assert!(!unread(&state, ROOM_A));

        // An edit is never activity.
        shared
            .messages
            .lock()
            .unwrap()
            .insert("b-theirs".into(), message("b-theirs", ROOM_B, "bob", 4, 2));
        state.cache.rooms.mark_read(&ROOM_B.to_owned());
        shared.push(
            "message.updated",
            json!({"roomId": ROOM_B, "messageId": "b-theirs", "revision": 2}),
        );
        for _ in 0..20 {
            glue.drain(&mut state, usize::MAX);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert!(!unread(&state, ROOM_B));
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn deletes_redact_immediately_and_lag_resync_refetches_loaded_rooms() {
    let (runtime, mut glue, control, shared) = fixture(None);
    let mut state = AppState::default();
    let driver = async {
        wait_subscribed(&shared, 1).await;
        ready(&mut glue, &mut state, &shared).await;
        glue.open_room(&mut state, &ROOM_A.to_owned());
        settle("page", &mut glue, &mut state, |_, state| {
            ids(state, ROOM_A).len() == 2
        })
        .await;
        // The daemon deletes a1: the body disappears before any refetch.
        shared
            .pages
            .lock()
            .unwrap()
            .get_mut(ROOM_A)
            .unwrap()
            .retain(|m| m.id != "a1");
        shared.push(
            "message.deleted",
            json!({"roomId": ROOM_A, "messageId": "a1", "revision": 3}),
        );
        settle("redaction", &mut glue, &mut state, |_, state| {
            ids(state, ROOM_A) == ["a2"]
        })
        .await;

        // Lag: the runtime re-snapshots, the glue resets loaded views and
        // refetches the newest page of every loaded room.
        let before = shared.page_reads.load(Ordering::SeqCst);
        shared
            .pages
            .lock()
            .unwrap()
            .get_mut(ROOM_A)
            .unwrap()
            .push(message("a4", ROOM_A, "alice", 6, 4));
        shared.push("resync.required", json!({"reason": "event_lag"}));
        settle("refetch after resync", &mut glue, &mut state, |_, state| {
            ids(state, ROOM_A) == ["a2", "a4"]
        })
        .await;
        assert!(shared.page_reads.load(Ordering::SeqCst) > before);
        // Room B was never loaded, so it was not fetched.
        assert!(state.cache.canonical_view(&ROOM_B.to_owned()).is_none());
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn timed_out_page_is_recorded_and_never_retried_automatically() {
    let (runtime, mut glue, control, shared) = fixture(Some(Duration::from_millis(20)));
    let mut state = AppState::default();
    let driver = async {
        wait_subscribed(&shared, 1).await;
        ready(&mut glue, &mut state, &shared).await;
        shared.pages_pending.store(true, Ordering::SeqCst);
        glue.open_room(&mut state, &ROOM_A.to_owned());
        settle("timeout", &mut glue, &mut state, |glue, _| {
            glue.status().failed_rooms.contains(ROOM_A)
        })
        .await;
        assert_eq!(glue.outstanding_reads(), 0);
        for _ in 0..30 {
            glue.drain(&mut state, usize::MAX);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(shared.page_reads.load(Ordering::SeqCst), 1, "no auto retry");
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn assembled_stack_connects_and_installs_without_extra_wiring() {
    let shared = Arc::new(Shared::default());
    *shared.rooms.lock().unwrap() = vec![room(ROOM_A)];
    let (runtime, mut glue) = BridgeGlue::assemble(
        SessionLifecycle::new(Backend(Arc::clone(&shared)), Clock(Instant::now())),
        Secrets,
        RuntimeConfig {
            tick: Duration::from_millis(5),
            ..RuntimeConfig::default()
        },
        500, // clamped to the UI API maximum of 100
    );
    assert_eq!(glue.page_size, 100);
    let control = glue.control.clone();
    let mut state = AppState::default();
    let driver = async {
        wait_subscribed(&shared, 1).await;
        ready(&mut glue, &mut state, &shared).await;
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn runtime_exit_marks_the_glue_closed_and_forgets_outstanding_reads() {
    let (runtime, mut glue, control, shared) = fixture(None);
    let mut state = AppState::default();
    let driver = async {
        wait_subscribed(&shared, 1).await;
        ready(&mut glue, &mut state, &shared).await;
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
    // The runtime (and its pump host) is gone.
    glue.drain(&mut state, usize::MAX);
    assert!(glue.status().closed);
    assert_eq!(glue.outstanding_reads(), 0);
    // Already displayed data stays, as stale.
    assert!(state.cache.rooms.room_with_id(&ROOM_A.to_owned()).is_some());
}

// ---- Deterministic unit tests over a manually driven pump -----------------

mod manual {
    use super::*;
    use crate::{
        bridge::{controller::PresentationHost, presentation::PresentationEndpoint},
        frontend::envelopes::RequestEnvelope,
    };

    /// The glue reads from `host`'s pump; its mailbox belongs to a worker whose
    /// own (ignored) pump receives the worker-side terminals.
    struct Manual {
        host: PumpHost<BridgeSnapshot>,
        glue: BridgeGlue,
        worker: CommandWorker<Backend, Clock, PumpHost<BridgeSnapshot>>,
        _worker_endpoint: PresentationEndpoint<BridgeSnapshot>,
        state: AppState<'static>,
    }

    fn manual() -> Manual {
        let (host, endpoint) = pump();
        let (worker_host, worker_endpoint) = pump();
        let (mailbox, worker) = CommandWorker::new(
            SessionLifecycle::new(Backend(Arc::new(Shared::default())), Clock(Instant::now())),
            worker_host,
        );
        Manual {
            host,
            glue: BridgeGlue::new(endpoint, mailbox, RuntimeControl::detached(), 50),
            worker,
            _worker_endpoint: worker_endpoint,
            state: AppState::default(),
        }
    }

    fn snapshot() -> BridgeSnapshot {
        BridgeSnapshot {
            account: None,
            rooms: vec![RoomSnapshot {
                id: data::RoomId::new(ROOM_A),
                title: None,
                kind: data::RoomKind::Group,
                team_id: None,
                last_activity: None,
                is_member: true,
                is_locked: None,
            }],
        }
    }

    fn page_result(request: &RequestEnvelope, ids: &[&str], revision: u64) -> ReadTerminal {
        let messages = ids
            .iter()
            .enumerate()
            .map(|(index, id)| data::MessageSnapshot {
                id: MessageId::new(*id),
                room_id: data::RoomId::new(ROOM_A),
                revision: Some(revision),
                parent_id: None,
                sender_id: None,
                sender_email: None,
                created_at: Some(format!("2026-01-01T00:0{index}:00Z")),
                updated_at: None,
                status: data::MessageStatus::Active,
                body: None,
                attachment_count: None,
            })
            .collect();
        let result = FrontendResult::ListMessagesInRoom(ReadOutcome::Success(MessagePage {
            room_id: data::RoomId::new(ROOM_A),
            messages,
            next_cursor: None,
        }));
        ReadTerminal::Result(Box::new(
            RequestEnvelope {
                context: request.context.clone(),
                intent: request.intent.clone(),
            }
            .complete(result)
            .unwrap(),
        ))
    }

    /// Pending newest-page requests for room A, rebuilt as envelopes.
    fn pending_pages(m: &Manual) -> Vec<RequestEnvelope> {
        m.glue
            .pending
            .iter()
            .filter(|(_, (_, purpose))| matches!(purpose, Purpose::NewestPage { .. }))
            .map(|(id, (intent, _))| {
                let mut request = RequestSequence::default()
                    .begin(intent.clone(), None)
                    .unwrap();
                request.context.id = *id;
                request
            })
            .collect()
    }

    /// Install a snapshot, open room A and let the worker dequeue the page
    /// read (as the runtime would) so the mailbox no longer holds it.
    async fn opened_and_dequeued(m: &mut Manual) -> RequestEnvelope {
        m.host.install_snapshot(snapshot());
        m.glue.drain(&mut m.state, usize::MAX);
        m.glue.open_room(&mut m.state, &ROOM_A.to_owned());
        let pages = pending_pages(m);
        assert_eq!(pages.len(), 1);
        assert!(m.worker.handle_read().await);
        pages.into_iter().next().unwrap()
    }

    #[tokio::test]
    async fn a_pre_reset_page_result_never_applies_after_a_snapshot() {
        let mut m = manual();
        let old = opened_and_dequeued(&mut m).await;
        // A new snapshot resets the view and bumps the room generation; the
        // glue issues a fresh newest-page request.
        m.host.install_snapshot(snapshot());
        m.glue.drain(&mut m.state, usize::MAX);
        let fresh = pending_pages(&m);
        assert_eq!(fresh.len(), 2, "old still pending plus the fresh request");
        let fresh = fresh
            .into_iter()
            .find(|request| request.context.id != old.context.id)
            .unwrap();
        // Even tagged with the current install (so the pump cannot mark it
        // Stale), the old result is dropped by the per-room generation fence.
        let install = m.host.current_install();
        m.host.deliver_read(page_result(&old, &["old"], 1), install);
        m.glue.drain(&mut m.state, usize::MAX);
        assert!(ids(&m.state, ROOM_A).is_empty());
        m.host
            .deliver_read(page_result(&fresh, &["new"], 2), install);
        m.glue.drain(&mut m.state, usize::MAX);
        assert_eq!(ids(&m.state, ROOM_A), ["new"]);
        assert_eq!(m.glue.outstanding_reads(), 0);
    }

    #[tokio::test]
    async fn stale_terminal_is_settled_once_and_coalesced_with_the_fresh_request() {
        let mut m = manual();
        let old = opened_and_dequeued(&mut m).await;
        let old_install = m.host.current_install();
        m.host.install_snapshot(snapshot());
        m.host
            .deliver_read(page_result(&old, &["old"], 1), old_install);
        m.glue.drain(&mut m.state, usize::MAX);
        assert!(ids(&m.state, ROOM_A).is_empty(), "stale never applied");
        assert!(
            !m.glue.pending.contains_key(&old.context.id),
            "settled once"
        );
        // The snapshot's refetch and the Stale re-request coalesce into one.
        assert_eq!(pending_pages(&m).len(), 1);
    }

    #[tokio::test]
    async fn verified_departure_removes_the_room_and_redaction_hits_loaded_views_only() {
        let mut m = manual();
        let _ = opened_and_dequeued(&mut m).await;
        let projection = crate::frontend::lifecycle::LifecycleProjection::default();
        m.host.lifecycle(
            &projection,
            ProjectionEffects {
                redact: vec![MessageKey {
                    room_id: data::RoomId::new(ROOM_B),
                    message_id: MessageId::new("unloaded"),
                }],
                membership: Some(crate::frontend::events::MembershipChange {
                    room_id: data::RoomId::new(ROOM_A),
                    is_member: Some(false),
                }),
                ..Default::default()
            },
        );
        m.glue.drain(&mut m.state, usize::MAX);
        assert!(m
            .state
            .cache
            .rooms
            .room_with_id(&ROOM_A.to_owned())
            .is_none());
        // A redaction for an unloaded room never creates a view.
        assert!(m.state.cache.canonical_view(&ROOM_B.to_owned()).is_none());
    }
}
