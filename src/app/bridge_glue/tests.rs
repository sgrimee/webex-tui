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
    team_reads: AtomicUsize,
    /// Team reads answering 503 before succeeding.
    team_failures: AtomicUsize,
    /// Person reads never answer (they time out with a read deadline).
    persons_pending: AtomicBool,
    pages_pending: AtomicBool,
    watermark: AtomicU64,
    stream: Mutex<Option<mpsc::UnboundedSender<Item>>>,
    rooms: Mutex<Vec<dto::Room>>,
    pages: Mutex<HashMap<String, Vec<dto::Message>>>,
    messages: Mutex<HashMap<String, dto::Message>>,
    writes: AtomicUsize,
    write_replies: Mutex<std::collections::VecDeque<WriteReply>>,
    thread_reads: AtomicUsize,
    /// Thread roots served by `thread_children` (with their replies).
    threads: Mutex<HashMap<String, (dto::Message, Vec<dto::Message>)>>,
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
    async fn team(&self, id: &str) -> Reply<dto::Team> {
        self.0.team_reads.fetch_add(1, Ordering::SeqCst);
        if self
            .0
            .team_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(ClientError::Api {
                status: 503_u16.try_into().unwrap(),
                code: "bridge_unavailable".into(),
                message: "synthetic".into(),
            });
        }
        Ok(dto::Team {
            id: id.into(),
            name: Some(format!("team {id}")),
        })
    }
    async fn person_read(&self, id: &str) -> Reply<dto::PersonRead> {
        self.0.person_reads.fetch_add(1, Ordering::SeqCst);
        if self.0.persons_pending.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
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
        parent: &str,
        _: Option<&str>,
        _: Option<u32>,
    ) -> Reply<dto::ThreadChildren> {
        self.0.thread_reads.fetch_add(1, Ordering::SeqCst);
        let (root, children) = self
            .0
            .threads
            .lock()
            .unwrap()
            .get(parent)
            .cloned()
            .expect("scripted thread");
        Ok(dto::ThreadChildren {
            parent: dto::ThreadParent {
                id: parent.into(),
                state: "active".into(),
                message: Some(root),
            },
            children,
            next_before: None,
            complete: true,
            partial_reason: None,
        })
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

/// Scripted daemon write outcomes, consumed in order (default: success).
#[derive(Clone, Copy)]
enum WriteReply {
    Ok,
    Rejected,
    Unknown,
    Pending,
}

struct Writer(Arc<Shared>);
impl Writer {
    async fn outcome(&self) -> Reply<()> {
        self.0.writes.fetch_add(1, Ordering::SeqCst);
        let reply = self
            .0
            .write_replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(WriteReply::Ok);
        match reply {
            WriteReply::Ok => Ok(()),
            WriteReply::Rejected => Err(ClientError::Api {
                status: 403_u16.try_into().unwrap(),
                code: "write_not_allowed".into(),
                message: "synthetic".into(),
            }),
            WriteReply::Unknown => Err(ClientError::Api {
                status: 202_u16.try_into().unwrap(),
                code: "delivery_unknown".into(),
                message: "synthetic".into(),
            }),
            WriteReply::Pending => std::future::pending().await,
        }
    }
}
impl UiWriteBackend for Writer {
    async fn send(
        &mut self,
        room: &str,
        parent: Option<&str>,
        body: &dto::MessageBodyInput,
    ) -> Reply<dto::Message> {
        self.outcome().await?;
        let index = self.0.writes.load(Ordering::SeqCst);
        let mut sent = message(&format!("sent-{index}"), room, "me", 30, 1);
        sent.parent_id = parent.map(str::to_owned);
        sent.body = Some(dto::MessageBody {
            kind: body.kind.clone(),
            content: body.content.clone(),
        });
        Ok(sent)
    }
    async fn edit(
        &mut self,
        message_id: &str,
        room: &str,
        body: &dto::MessageBodyInput,
    ) -> Reply<dto::Message> {
        self.outcome().await?;
        let mut edited = message(message_id, room, "me", 2, 9);
        edited.body = Some(dto::MessageBody {
            kind: body.kind.clone(),
            content: body.content.clone(),
        });
        Ok(edited)
    }
    async fn delete(&mut self, _: &str) -> Reply<()> {
        self.outcome().await
    }
    async fn leave(&mut self, _: &str) -> Reply<()> {
        self.outcome().await
    }
}

struct Backend(Arc<Shared>);
impl SessionBackend for Backend {
    type Client = Arc<Shared>;
    type Stream = FakeStream;
    type Writer = Writer;
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
    async fn renew(
        &mut self,
        _: &Arc<Shared>,
    ) -> Result<u64, crate::bridge::session::RenewFailure> {
        // Not exercised here: behaves like a daemon at the 12 h cap.
        Err(crate::bridge::session::RenewFailure::CapReached)
    }
    fn lease_ttl(_: &Arc<Shared>) -> u64 {
        600
    }
    fn writer(client: &Arc<Shared>) -> Writer {
        Writer(Arc::clone(client))
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

/// Daemon restart (e.g. `launchctl kickstart`): the event stream fails, the
/// runtime reconnects and installs a new snapshot. Afterwards the loaded room
/// is refetched and a newly opened room still loads its page (live S5).
#[tokio::test]
async fn rooms_still_load_after_a_reconnect_and_snapshot_reinstall() {
    let (runtime, mut glue, control, shared) = fixture(None);
    shared
        .pages
        .lock()
        .unwrap()
        .insert(ROOM_B.into(), vec![message("b1", ROOM_B, "alice", 3, 1)]);
    let mut state = AppState::default();
    let driver = async {
        wait_subscribed(&shared, 1).await;
        ready(&mut glue, &mut state, &shared).await;
        glue.open_room(&mut state, &ROOM_A.to_owned());
        settle("page A", &mut glue, &mut state, |glue, state| {
            ids(state, ROOM_A).len() == 2 && glue.outstanding_reads() == 0
        })
        .await;
        let reads_before = shared.page_reads.load(Ordering::SeqCst);

        // The daemon goes away: the stream fails with a transport error.
        let sender = shared.stream.lock().unwrap().clone().unwrap();
        sender.send(Some(Err(SessionFailure::Transport))).unwrap();
        wait_subscribed(&shared, 2).await;
        shared.push("continuity.ready", json!({}));
        settle(
            "loaded room refetched",
            &mut glue,
            &mut state,
            |glue, state| {
                shared.page_reads.load(Ordering::SeqCst) > reads_before
                    && ids(state, ROOM_A).len() == 2
                    && glue.outstanding_reads() == 0
            },
        )
        .await;

        // A room opened only after the reinstall loads too.
        glue.open_room(&mut state, &ROOM_B.to_owned());
        settle(
            "page B after reconnect",
            &mut glue,
            &mut state,
            |_, state| ids(state, ROOM_B) == ["b1"],
        )
        .await;
        assert!(glue.status().failed_rooms.is_empty());
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

/// Fail the current event stream as a daemon restart does, then complete
/// the reconnect's continuity handshake so a new snapshot is installed.
async fn restart_daemon(
    glue: &mut BridgeGlue,
    state: &mut AppState<'_>,
    shared: &Shared,
    loaded: &str,
) {
    let generation = glue.generation(&loaded.to_owned());
    let subscribes = shared.subscribes.load(Ordering::SeqCst);
    let sender = shared.stream.lock().unwrap().clone().unwrap();
    sender.send(Some(Err(SessionFailure::Transport))).unwrap();
    wait_subscribed(shared, subscribes + 1).await;
    shared.push("continuity.ready", json!({}));
    // An install bumps every loaded room's generation.
    settle("reinstall", glue, state, |glue, _| {
        glue.generation(&loaded.to_owned()) > generation
    })
    .await;
}

/// Live S5: after a daemon restart, rooms loaded in the session stayed
/// empty. The reconnect's whole-mirror reconcile asks for every loaded room
/// just before the new snapshot; with more loaded rooms than read credits,
/// some of those requests wait in the deferred queue under the old room
/// generation. The install then asks again under the new generation, but the
/// deferred duplicate kept its old generation, so its page was dropped as
/// pre-reset and the room was left empty, not pending and not failed.
#[tokio::test]
async fn every_loaded_room_is_refilled_after_a_reinstall_with_a_full_read_queue() {
    let (runtime, mut glue, control, shared) = fixture(None);
    let mut loaded: Vec<String> = (0..20).map(|i| format!("loaded-{i:02}")).collect();
    {
        let mut pages = shared.pages.lock().unwrap();
        for (i, room) in loaded.iter().enumerate() {
            pages.insert(
                room.clone(),
                vec![message(&format!("m{i}"), room, "alice", 1, 1)],
            );
        }
    }
    loaded.push(ROOM_A.into());
    let mut state = AppState::default();
    let filled = |state: &AppState, loaded: &[String]| {
        loaded.iter().all(|room| !ids(state, room).is_empty())
    };
    let driver = async {
        wait_subscribed(&shared, 1).await;
        ready(&mut glue, &mut state, &shared).await;
        for room in &loaded {
            glue.open_room(&mut state, room);
        }
        settle("all pages", &mut glue, &mut state, |glue, state| {
            filled(state, &loaded) && glue.outstanding_reads() == 0
        })
        .await;

        restart_daemon(&mut glue, &mut state, &shared, ROOM_A).await;
        settle("reads settled", &mut glue, &mut state, |glue, _| {
            glue.outstanding_reads() == 0
        })
        .await;
        let empty: Vec<&String> = loaded
            .iter()
            .filter(|room| ids(&state, room).is_empty())
            .collect();
        assert!(
            empty.is_empty(),
            "rooms left empty after the reinstall: {empty:?}"
        );
        assert!(glue.status().failed_rooms.is_empty());
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

/// The status line says "reopen to retry": reopening a room whose page read
/// failed (here: timed out) requests it again.
#[tokio::test]
async fn reopening_a_room_whose_page_failed_retries_it() {
    let (runtime, mut glue, control, shared) = fixture(Some(Duration::from_millis(20)));
    let mut state = AppState::default();
    let driver = async {
        wait_subscribed(&shared, 1).await;
        ready(&mut glue, &mut state, &shared).await;
        shared.pages_pending.store(true, Ordering::SeqCst);
        glue.open_room(&mut state, &ROOM_A.to_owned());
        settle("page timed out", &mut glue, &mut state, |glue, _| {
            glue.status().failed_rooms.contains(ROOM_A)
        })
        .await;
        shared.pages_pending.store(false, Ordering::SeqCst);
        glue.open_room(&mut state, &ROOM_A.to_owned());
        settle("retried page", &mut glue, &mut state, |_, state| {
            ids(state, ROOM_A).len() == 2
        })
        .await;
        assert!(glue.status().failed_rooms.is_empty());
        // A loaded room is still not re-requested on reopen.
        let reads = shared.page_reads.load(Ordering::SeqCst);
        glue.open_room(&mut state, &ROOM_A.to_owned());
        glue.drain(&mut state, usize::MAX);
        assert_eq!(shared.page_reads.load(Ordering::SeqCst), reads);
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn team_names_are_fetched_once_per_team_and_decorate_room_titles() {
    let (runtime, mut glue, control, shared) = fixture(None);
    {
        let mut rooms = shared.rooms.lock().unwrap();
        for room in rooms.iter_mut() {
            room.team_id = Some("team-1".into());
        }
        // The team's General room carries the team's own name.
        let mut general = room("general");
        general.team_id = Some("team-1".into());
        general.title = Some("team team-1".into());
        rooms.push(general);
    }
    let mut state = AppState::default();
    let driver = async {
        wait_subscribed(&shared, 1).await;
        ready(&mut glue, &mut state, &shared).await;
        settle("team", &mut glue, &mut state, |_, state| {
            state
                .cache
                .room_and_team_title(&ROOM_A.to_owned())
                .is_ok_and(|title| title.team_name.is_some())
        })
        .await;
        let title = state.cache.room_and_team_title(&ROOM_A.to_owned()).unwrap();
        assert_eq!(title.team_name.as_deref(), Some("team team-1"));
        assert_eq!(title.room_title, format!("title {ROOM_A}"));
        let general = state.cache.room_and_team_title(&"general".into()).unwrap();
        assert_eq!(general.room_title, "General");
        // Three rooms, one team: one read, and a later room refresh does not
        // re-request it.
        glue.request(FrontendIntent::ListAllRooms, Purpose::Rooms);
        settle("rooms refresh", &mut glue, &mut state, |glue, _| {
            glue.outstanding_reads() == 0
        })
        .await;
        assert_eq!(shared.team_reads.load(Ordering::SeqCst), 1);
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn a_team_lookup_failed_transiently_is_retried_after_the_next_snapshot() {
    let (runtime, mut glue, control, shared) = fixture(None);
    shared.rooms.lock().unwrap()[0].team_id = Some("team-1".into());
    shared.team_failures.store(1, Ordering::SeqCst);
    let mut state = AppState::default();
    let driver = async {
        wait_subscribed(&shared, 1).await;
        ready(&mut glue, &mut state, &shared).await;
        settle("failed team read", &mut glue, &mut state, |glue, _| {
            shared.team_reads.load(Ordering::SeqCst) == 1 && glue.outstanding_reads() == 0
        })
        .await;
        assert!(state.cache.teams.team_with_id(&"team-1".into()).is_none());
        // A new snapshot (here after event lag) asks again.
        shared.push("resync.required", json!({"reason": "event_lag"}));
        settle("team after snapshot", &mut glue, &mut state, |_, state| {
            state.cache.teams.team_with_id(&"team-1".into()).is_some()
        })
        .await;
        assert_eq!(shared.team_reads.load(Ordering::SeqCst), 2);
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn a_timed_out_person_lookup_can_be_requested_again() {
    let (runtime, mut glue, control, shared) = fixture(Some(Duration::from_millis(20)));
    shared.persons_pending.store(true, Ordering::SeqCst);
    let mut state = AppState::default();
    let driver = async {
        wait_subscribed(&shared, 1).await;
        ready(&mut glue, &mut state, &shared).await;
        glue.open_room(&mut state, &ROOM_A.to_owned());
        settle("person timeouts", &mut glue, &mut state, |glue, state| {
            ids(state, ROOM_A).len() == 2 && glue.outstanding_reads() == 0
        })
        .await;
        // Not stuck as "requested": the next page with that sender asks again.
        assert!(!state.cache.persons.exists_or_requested("alice"));
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

// ---- Writes (Step 5D) ------------------------------------------------------

use crate::frontend::{
    editor_submission::{DraftSnapshot, SubmissionError},
    editor_write::EditorWriteStatus,
};

async fn active_room_loaded(glue: &mut BridgeGlue, state: &mut AppState<'_>, shared: &Shared) {
    wait_subscribed(shared, 1).await;
    ready(glue, state, shared).await;
    state.rooms_list.set_active_room_id(Some(ROOM_A.into()));
    glue.open_room(state, &ROOM_A.to_owned());
    settle("page", glue, state, |_, state| {
        ids(state, ROOM_A).len() == 2
    })
    .await;
}

fn draft_now(state: &AppState) -> DraftSnapshot {
    state
        .message_editor
        .draft(Some(&data::RoomId::new(ROOM_A)))
        .unwrap()
}

/// Admission opens once the runtime has processed the snapshot ack.
async fn submit(glue: &mut BridgeGlue, state: &mut AppState<'_>) {
    for _ in 0..2000 {
        let draft = draft_now(state);
        match glue.submit_draft(state, &draft) {
            Ok(()) => return,
            Err(WriteRefusal::Submission(SubmissionError::QueueNotReady)) => {
                glue.drain(state, usize::MAX);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Err(other) => panic!("unexpected refusal {other:?}"),
        }
    }
    panic!("write admission never opened");
}

#[tokio::test]
async fn senders_of_created_messages_threads_and_confirmed_writes_are_requested() {
    let (runtime, mut glue, control, shared) = fixture(None);
    let mut state = AppState::default();
    let known = |state: &AppState, id: &str| state.cache.persons.get(id).is_some();
    let driver = async {
        active_room_loaded(&mut glue, &mut state, &shared).await;
        settle("page senders", &mut glue, &mut state, |_, state| {
            known(state, "alice") && known(state, "me")
        })
        .await;
        let after_page = shared.person_reads.load(Ordering::SeqCst);
        // A new participant's live message.
        shared
            .messages
            .lock()
            .unwrap()
            .insert("a3".into(), message("a3", ROOM_A, "carol", 5, 1));
        shared.push(
            "message.created",
            json!({"roomId": ROOM_A, "messageId": "a3", "revision": 1}),
        );
        settle("created sender", &mut glue, &mut state, |_, state| {
            known(state, "carol")
        })
        .await;
        // A thread page: root and reply senders.
        let mut reply = message("t-reply", ROOM_A, "erin", 7, 1);
        reply.parent_id = Some("t-root".into());
        shared.threads.lock().unwrap().insert(
            "t-root".into(),
            (message("t-root", ROOM_A, "dave", 6, 1), vec![reply]),
        );
        glue.load_thread(&ROOM_A.to_owned(), &data::MessageId::new("t-root"));
        settle("thread senders", &mut glue, &mut state, |_, state| {
            known(state, "dave") && known(state, "erin")
        })
        .await;
        assert_eq!(shared.person_reads.load(Ordering::SeqCst), after_page + 3);
        // A confirmed write installs a message whose sender is looked up too.
        state.cache.persons = Default::default();
        state.message_editor.reset_with_text("hello".into());
        submit(&mut glue, &mut state).await;
        settle("write sender", &mut glue, &mut state, |_, state| {
            known(state, "me")
        })
        .await;
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

#[tokio::test]
async fn send_success_installs_the_canonical_message_and_clears_that_draft() {
    let (runtime, mut glue, control, shared) = fixture(None);
    let mut state = AppState::default();
    let driver = async {
        active_room_loaded(&mut glue, &mut state, &shared).await;
        state.message_editor.reset_with_text("hello".into());
        submit(&mut glue, &mut state).await;
        assert_eq!(glue.editor().status(), EditorWriteStatus::Pending);
        settle("sent", &mut glue, &mut state, |glue, _| {
            glue.editor().status() == EditorWriteStatus::Ready
        })
        .await;
        assert!(ids(&state, ROOM_A).contains(&"sent-1".to_owned()));
        assert!(
            state.message_editor.is_empty(),
            "the submitted draft is cleared"
        );
        // Its SSE echo is idempotent: still one row.
        shared.push(
            "message.created",
            json!({"roomId": ROOM_A, "messageId": "sent-1", "revision": 1}),
        );
        shared
            .messages
            .lock()
            .unwrap()
            .insert("sent-1".into(), message("sent-1", ROOM_A, "me", 30, 1));
        for _ in 0..20 {
            glue.drain(&mut state, usize::MAX);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let count = ids(&state, ROOM_A)
            .iter()
            .filter(|id| *id == "sent-1")
            .count();
        assert_eq!(count, 1);
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
    assert_eq!(shared.writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn newer_text_typed_while_pending_survives_the_success() {
    let (runtime, mut glue, control, shared) = fixture(None);
    let mut state = AppState::default();
    let driver = async {
        active_room_loaded(&mut glue, &mut state, &shared).await;
        state.message_editor.reset_with_text("first".into());
        shared
            .write_replies
            .lock()
            .unwrap()
            .push_back(WriteReply::Ok);
        submit(&mut glue, &mut state).await;
        // The user keeps typing before the result is reduced.
        state.message_editor.reset_with_text("second draft".into());
        settle("sent", &mut glue, &mut state, |glue, _| {
            glue.editor().status() == EditorWriteStatus::Ready
        })
        .await;
        assert_eq!(state.message_editor.lines(), ["second draft"]);
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
}

/// REVIEW-13 LOW-3: switching rooms while a send is in flight still clears
/// the sent text, so Enter in the new room cannot send it again.
#[tokio::test]
async fn a_room_switch_during_a_send_still_clears_the_sent_draft() {
    let (runtime, mut glue, control, shared) = fixture(None);
    let mut state = AppState::default();
    let driver = async {
        active_room_loaded(&mut glue, &mut state, &shared).await;
        state.message_editor.reset_with_text("for room A".into());
        submit(&mut glue, &mut state).await;
        state.rooms_list.set_active_room_id(Some(ROOM_B.into()));
        settle("sent", &mut glue, &mut state, |glue, _| {
            glue.editor().status() == EditorWriteStatus::Ready
        })
        .await;
        assert!(state.message_editor.is_empty(), "sent text cleared");
        assert!(ids(&state, ROOM_A).contains(&"sent-1".to_owned()));
        assert!(state
            .message_editor
            .draft(Some(&data::RoomId::new(ROOM_B)))
            .is_some_and(
                |draft| draft.body == crate::frontend::data::ComposedBody::PlainText(String::new())
            ));
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
    assert_eq!(shared.writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn rejected_and_unknown_sends_keep_a_review_and_are_never_replayed() {
    for (reply, status) in [
        (WriteReply::Rejected, EditorWriteStatus::Rejected),
        (WriteReply::Unknown, EditorWriteStatus::DeliveryUnknown),
    ] {
        let (runtime, mut glue, control, shared) = fixture(None);
        let mut state = AppState::default();
        let driver = async {
            active_room_loaded(&mut glue, &mut state, &shared).await;
            state.message_editor.reset_with_text("keep me".into());
            shared.write_replies.lock().unwrap().push_back(reply);
            submit(&mut glue, &mut state).await;
            settle("outcome", &mut glue, &mut state, |glue, _| {
                glue.editor().status() == status
            })
            .await;
            assert!(glue.editor().review().is_some());
            assert_eq!(state.message_editor.lines(), ["keep me"], "draft kept");
            // A fresh write needs an explicit review dismissal first.
            assert!(matches!(
                glue.submit_draft(&state, &draft_now(&state)),
                Err(WriteRefusal::Submission(SubmissionError::ReviewNeeded))
            ));
            for _ in 0..20 {
                glue.drain(&mut state, usize::MAX);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            assert_eq!(shared.writes.load(Ordering::SeqCst), 1, "never replayed");
            assert!(glue.dismiss_review());
            control.shutdown();
        };
        let (exit, ()) = tokio::join!(runtime.run(), driver);
        assert_eq!(exit, RuntimeExit::Shutdown);
    }
}

#[tokio::test]
async fn confirmed_delete_and_leave_apply_only_after_success() {
    let (runtime, mut glue, control, shared) = fixture(None);
    let mut state = AppState::default();
    let driver = async {
        active_room_loaded(&mut glue, &mut state, &shared).await;
        // Admission opens after the ack is processed.
        loop {
            match glue.delete_message(&ROOM_A.to_owned(), &MessageId::new("a1")) {
                Ok(()) => break,
                Err(WriteRefusal::NotReady) => {
                    glue.drain(&mut state, usize::MAX);
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                Err(other) => panic!("{other:?}"),
            }
        }
        // Not removed optimistically.
        assert!(ids(&state, ROOM_A).contains(&"a1".to_owned()));
        assert_eq!(
            glue.leave_room(&ROOM_B.to_owned()),
            Err(WriteRefusal::Busy),
            "one write at a time"
        );
        settle("delete confirmed", &mut glue, &mut state, |glue, _| {
            glue.last_action().is_some()
        })
        .await;
        assert_eq!(ids(&state, ROOM_A), ["a2"]);
        assert!(matches!(
            glue.last_action(),
            Some(ActionOutcome::Confirmed(_))
        ));
        glue.leave_room(&ROOM_B.to_owned()).unwrap();
        settle("leave confirmed", &mut glue, &mut state, |_, state| {
            state.cache.rooms.room_with_id(&ROOM_B.to_owned()).is_none()
        })
        .await;
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
    assert_eq!(shared.writes.load(Ordering::SeqCst), 2);
}

/// Unknown and (REVIEW-13 nit c) rejected deletes both reconcile by reading.
#[tokio::test]
async fn uncertain_or_rejected_delete_reconciles_by_reading_instead_of_replaying() {
    for (reply, rejected) in [(WriteReply::Unknown, false), (WriteReply::Rejected, true)] {
        let (runtime, mut glue, control, shared) = fixture(None);
        let mut state = AppState::default();
        let driver = async {
            active_room_loaded(&mut glue, &mut state, &shared).await;
            shared.write_replies.lock().unwrap().push_back(reply);
            loop {
                match glue.delete_message(&ROOM_A.to_owned(), &MessageId::new("a1")) {
                    Ok(()) => break,
                    Err(WriteRefusal::NotReady) => {
                        glue.drain(&mut state, usize::MAX);
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    Err(other) => panic!("{other:?}"),
                }
            }
            let reads_before = shared.message_reads.load(Ordering::SeqCst);
            settle("reconciling read", &mut glue, &mut state, |glue, _| {
                let outcome = match glue.last_action() {
                    Some(ActionOutcome::Unknown(_)) => !rejected,
                    Some(ActionOutcome::Rejected(_)) => rejected,
                    _ => false,
                };
                outcome && shared.message_reads.load(Ordering::SeqCst) > reads_before
            })
            .await;
            // Still shown: only a confirmed tombstone removes it.
            assert!(ids(&state, ROOM_A).contains(&"a1".to_owned()));
            for _ in 0..20 {
                glue.drain(&mut state, usize::MAX);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            assert_eq!(shared.writes.load(Ordering::SeqCst), 1, "never replayed");
            control.shutdown();
        };
        let (exit, ()) = tokio::join!(runtime.run(), driver);
        assert_eq!(exit, RuntimeExit::Shutdown);
    }
}

#[tokio::test]
async fn a_write_lost_with_the_runtime_is_uncertain_not_unsent() {
    let (runtime, mut glue, _control, shared) = fixture(None);
    let mut state = AppState::default();
    let driver = async {
        active_room_loaded(&mut glue, &mut state, &shared).await;
        shared
            .write_replies
            .lock()
            .unwrap()
            .push_back(WriteReply::Pending);
        state.message_editor.reset_with_text("in flight".into());
        submit(&mut glue, &mut state).await;
        for _ in 0..2000 {
            if shared.writes.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    };
    // The runtime task is dropped mid-write.
    tokio::select! {
        exit = runtime.run() => panic!("runtime exited early: {exit:?}"),
        () = driver => {}
    }
    glue.drain(&mut state, usize::MAX);
    assert!(glue.status().closed);
    assert_eq!(glue.editor().status(), EditorWriteStatus::DeliveryUnknown);
    assert!(glue.editor().review().is_some());
    assert_eq!(state.message_editor.lines(), ["in flight"]);
}

/// REVIEW-13 LOW-1: a write the worker confirmed just before the runtime
/// dropped stays confirmed. The glue first observes `closed`, then drains the
/// slot (the worker fills the slot before its Drop closes the endpoint).
#[tokio::test]
async fn writes_confirmed_just_before_the_runtime_exits_stay_confirmed() {
    let (runtime, mut glue, control, shared) = fixture(None);
    let mut state = AppState::default();
    let driver = async {
        active_room_loaded(&mut glue, &mut state, &shared).await;
        state.message_editor.reset_with_text("last words".into());
        submit(&mut glue, &mut state).await;
        for _ in 0..2000 {
            if shared.writes.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        // The App does not drain before the runtime is gone.
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
    glue.drain(&mut state, usize::MAX);
    assert!(glue.status().closed);
    assert_eq!(glue.editor().status(), EditorWriteStatus::Ready);
    assert!(glue.editor().review().is_none(), "not uncertain");
    assert!(ids(&state, ROOM_A).contains(&"sent-1".to_owned()));
    assert!(state.message_editor.is_empty());

    // Same for an explicit delete confirmed right before the exit.
    let (runtime, mut glue, control, shared) = fixture(None);
    let mut state = AppState::default();
    let driver = async {
        active_room_loaded(&mut glue, &mut state, &shared).await;
        loop {
            match glue.delete_message(&ROOM_A.to_owned(), &MessageId::new("a1")) {
                Ok(()) => break,
                Err(WriteRefusal::NotReady) => {
                    glue.drain(&mut state, usize::MAX);
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                Err(other) => panic!("{other:?}"),
            }
        }
        for _ in 0..2000 {
            if shared.writes.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
    glue.drain(&mut state, usize::MAX);
    assert!(matches!(
        glue.last_action(),
        Some(ActionOutcome::Confirmed(_))
    ));
    assert_eq!(ids(&state, ROOM_A), ["a2"]);
}

#[tokio::test]
async fn writes_stay_closed_until_the_snapshot_is_acknowledged() {
    let (runtime, mut glue, control, shared) = fixture(None);
    let mut state = AppState::default();
    let driver = async {
        wait_subscribed(&shared, 1).await;
        shared.push("continuity.ready", json!({}));
        // Wait for the install without draining it (so it is not acked).
        for _ in 0..200 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        state.message_editor.reset_with_text("too early".into());
        let draft = draft_now(&state);
        assert_eq!(
            glue.submit_draft(&state, &draft),
            Err(WriteRefusal::Submission(SubmissionError::QueueNotReady))
        );
        assert_eq!(state.message_editor.lines(), ["too early"]);
        control.shutdown();
    };
    let (exit, ()) = tokio::join!(runtime.run(), driver);
    assert_eq!(exit, RuntimeExit::Shutdown);
    assert_eq!(shared.writes.load(Ordering::SeqCst), 0);
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

    /// Re-review LOW-1 (a): a reply bound to a live reply whose thread root
    /// was deleted is refused before anything is queued.
    #[tokio::test]
    async fn a_reply_to_a_deleted_thread_root_is_refused_without_sending() {
        let mut m = manual();
        let room = ROOM_A.to_owned();
        let view = m.state.cache.canonical_view_mut(&room);
        view.tombstone(&data::RoomId::new(ROOM_A), &MessageId::new("root"), None)
            .unwrap();
        let reply = data::MessageSnapshot {
            id: MessageId::new("child"),
            room_id: data::RoomId::new(ROOM_A),
            revision: Some(1),
            parent_id: Some(MessageId::new("root")),
            sender_id: None,
            sender_email: None,
            created_at: None,
            updated_at: None,
            status: data::MessageStatus::Active,
            body: None,
            attachment_count: None,
        };
        m.state.message_editor.reset_with_text("answer".into());
        m.state.message_editor.set_response_to(Some(reply));
        let draft = m
            .state
            .message_editor
            .draft(Some(&data::RoomId::new(ROOM_A)))
            .unwrap();
        assert_eq!(
            m.glue.submit_draft(&m.state, &draft),
            Err(WriteRefusal::ThreadRootDeleted)
        );
        assert!(
            m.glue.editor().pending_context().is_none(),
            "nothing queued"
        );
        assert!(!m.worker.has_queued_write());
        assert_eq!(m.state.message_editor.lines(), ["answer"], "draft kept");
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

/// Step S3: App routing with an attached glue: every read and write goes
/// through the bridge.
mod app_routing {
    use super::*;
    use crate::app::{state::ActivePane, App};
    use crate::inputs::key::Key;

    struct Harness {
        app: App<'static>,
    }

    fn harness(glue: BridgeGlue) -> Harness {
        let mut app = App::new(false, crate::theme::Theme::default());
        app.attach_bridge(glue);
        Harness { app }
    }

    /// Tick the App (which drains the glue) until `condition` holds.
    async fn tick_until(what: &str, h: &mut Harness, mut condition: impl FnMut(&App) -> bool) {
        for _ in 0..2000 {
            h.app.update_on_tick().await;
            if condition(&h.app) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        panic!("timed out waiting for {what}");
    }

    /// Snapshot installed, room A active, selected in the list and loaded.
    async fn room_a_open(h: &mut Harness, shared: &Shared) {
        wait_subscribed(shared, 1).await;
        shared.push("continuity.ready", json!({}));
        tick_until("snapshot", h, |app| {
            app.state
                .cache
                .rooms
                .room_with_id(&ROOM_A.to_owned())
                .is_some()
        })
        .await;
        // The first rooms focus the room list, as the legacy load did.
        assert_eq!(h.app.state.active_pane(), &Some(ActivePane::Rooms));
        h.app
            .state
            .rooms_list
            .set_active_room_id(Some(ROOM_A.into()));
        h.app.state.update_room_selection_with_active_room();
        h.app.get_messages_if_room_empty(&ROOM_A.to_owned());
        tick_until("page", h, |app| ids(&app.state, ROOM_A).len() == 2).await;
        h.app.state.set_active_pane(Some(ActivePane::Messages));
    }

    fn select(h: &mut Harness, id: &str) {
        let index = h
            .app
            .state
            .cache
            .rows_in_room(&ROOM_A.to_owned())
            .position(|row| row.id().as_str() == id)
            .unwrap();
        let rows = h.app.state.num_messages_active_room();
        h.app.state.messages_list.set_nb_messages(rows);
        h.app.state.messages_list.select_index(index);
        h.app.bridge_selection_changed();
    }

    #[tokio::test]
    async fn send_edit_and_delete_go_through_the_bridge_only() {
        let (runtime, glue, control, shared) = fixture(None);
        let mut h = harness(glue);
        let driver = async {
            room_a_open(&mut h, &shared).await;
            assert!(h
                .app
                .state
                .bridge
                .as_ref()
                .unwrap()
                .line
                .contains("bridge online"));
            // Send: retried while admission opens after the snapshot ack.
            h.app.state.message_editor.reset_with_text("hello".into());
            loop {
                match h.app.send_message_buffer() {
                    Ok(()) => break,
                    Err(_) => {
                        h.app.update_on_tick().await;
                    }
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            assert_eq!(
                h.app.state.message_editor.lines(),
                ["hello"],
                "kept until confirmed"
            );
            tick_until("sent", &mut h, |app| {
                ids(&app.state, ROOM_A).contains(&"sent-1".to_owned())
                    && app.state.message_editor.is_empty()
            })
            .await;
            // Edit own message a2 (pre-filled from its source).
            select(&mut h, "a2");
            h.app.edit_selected_message().unwrap();
            h.app
                .state
                .message_editor
                .reset_with_text("a2 fixed".into());
            h.app.state.message_editor.set_editing_of(Some(
                h.app
                    .state
                    .cache
                    .canonical_view(&ROOM_A.to_owned())
                    .unwrap()
                    .get(&MessageId::new("a2"))
                    .unwrap()
                    .clone(),
            ));
            h.app.send_message_buffer().unwrap();
            tick_until("edited", &mut h, |app| {
                app.bridge().unwrap().editor().status() == EditorWriteStatus::Ready
                    && app.state.message_editor.is_empty()
            })
            .await;
            // Delete own message a2: removed only on confirmation.
            select(&mut h, "a2");
            h.app.delete_selected_message().unwrap();
            assert!(ids(&h.app.state, ROOM_A).contains(&"a2".to_owned()));
            tick_until("deleted", &mut h, |app| {
                !ids(&app.state, ROOM_A).contains(&"a2".to_owned())
            })
            .await;
            assert!(h
                .app
                .state
                .bridge
                .as_ref()
                .unwrap()
                .line
                .contains("delete done"));
            // Someone else's message cannot be deleted.
            select(&mut h, "a1");
            assert!(h.app.delete_selected_message().is_err());
            control.shutdown();
        };
        let (exit, ()) = tokio::join!(runtime.run(), driver);
        assert_eq!(exit, RuntimeExit::Shutdown);
        assert_eq!(shared.writes.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_rejected_send_keeps_the_draft_and_x_dismisses_the_review() {
        let (runtime, glue, control, shared) = fixture(None);
        let mut h = harness(glue);
        let driver = async {
            room_a_open(&mut h, &shared).await;
            shared
                .write_replies
                .lock()
                .unwrap()
                .push_back(WriteReply::Rejected);
            h.app.state.message_editor.reset_with_text("keep me".into());
            loop {
                if h.app.send_message_buffer().is_ok() {
                    break;
                }
                h.app.update_on_tick().await;
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            tick_until("rejected", &mut h, |app| {
                app.state.bridge.as_ref().is_some_and(|b| b.review_open)
            })
            .await;
            let bridge = h.app.state.bridge.clone().unwrap();
            assert!(bridge.line.contains("not sent: rejected (x to dismiss)"));
            assert!(!bridge.line.contains("keep me"), "no body in the status");
            assert_eq!(h.app.state.message_editor.lines(), ["keep me"]);
            assert!(h
                .app
                .state
                .actions
                .actions()
                .contains(&crate::app::actions::Action::DismissWriteReview));
            // A fresh send is refused until the review is dismissed.
            assert!(h.app.send_message_buffer().is_err());
            h.app.do_action(Key::Char('x'));
            assert!(!h.app.state.bridge.as_ref().unwrap().review_open);
            assert!(!h
                .app
                .state
                .actions
                .actions()
                .contains(&crate::app::actions::Action::DismissWriteReview));
            control.shutdown();
        };
        let (exit, ()) = tokio::join!(runtime.run(), driver);
        assert_eq!(exit, RuntimeExit::Shutdown);
        assert_eq!(shared.writes.load(Ordering::SeqCst), 1, "never replayed");
    }

    #[tokio::test]
    async fn multi_room_leave_is_queued_one_at_a_time() {
        let (runtime, glue, control, shared) = fixture(None);
        let mut h = harness(glue);
        let driver = async {
            room_a_open(&mut h, &shared).await;
            // Admission opens once the ack is processed.
            tick_until("writable", &mut h, |app| {
                app.bridge().unwrap().status().phase == Some(SessionPhase::Active)
            })
            .await;
            h.app
                .bridge_leave_rooms(vec![ROOM_B.to_owned(), ROOM_A.to_owned()]);
            tick_until("both left", &mut h, |app| {
                app.state
                    .cache
                    .rooms
                    .room_with_id(&ROOM_A.to_owned())
                    .is_none()
                    && app
                        .state
                        .cache
                        .rooms
                        .room_with_id(&ROOM_B.to_owned())
                        .is_none()
            })
            .await;
            assert!(h.app.pending_leaves.is_empty());
            control.shutdown();
        };
        let (exit, ()) = tokio::join!(runtime.run(), driver);
        assert_eq!(exit, RuntimeExit::Shutdown);
        assert_eq!(shared.writes.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn selecting_a_missing_root_placeholder_loads_its_thread() {
        let (runtime, glue, control, shared) = fixture(None);
        // Room A's page has a reply whose root is not on the page.
        let mut orphan = message("reply", ROOM_A, "alice", 3, 3);
        orphan.parent_id = Some("root".into());
        shared
            .pages
            .lock()
            .unwrap()
            .get_mut(ROOM_A)
            .unwrap()
            .push(orphan.clone());
        shared.threads.lock().unwrap().insert(
            "root".into(),
            (message("root", ROOM_A, "alice", 0, 4), vec![orphan]),
        );
        let mut h = harness(glue);
        let driver = async {
            wait_subscribed(&shared, 1).await;
            shared.push("continuity.ready", json!({}));
            tick_until("snapshot", &mut h, |app| {
                app.state
                    .cache
                    .rooms
                    .room_with_id(&ROOM_A.to_owned())
                    .is_some()
            })
            .await;
            h.app
                .state
                .rooms_list
                .set_active_room_id(Some(ROOM_A.into()));
            h.app.state.update_room_selection_with_active_room();
            h.app.get_messages_if_room_empty(&ROOM_A.to_owned());
            tick_until("page", &mut h, |app| ids(&app.state, ROOM_A).len() == 3).await;
            select(&mut h, "root"); // the placeholder row carries the root ID
            tick_until("thread loaded", &mut h, |app| {
                matches!(
                    app.state
                        .cache
                        .rows_in_room(&ROOM_A.to_owned())
                        .find(|row| row.id().as_str() == "root"),
                    Some(crate::frontend::message_view::ViewRow::Message(_))
                )
            })
            .await;
            assert_eq!(shared.thread_reads.load(Ordering::SeqCst), 1);
            // The selection stayed on the (now real) root row.
            let selected = h.app.state.selected_message().unwrap();
            assert_eq!(selected.id.as_str(), "root");
            control.shutdown();
        };
        let (exit, ()) = tokio::join!(runtime.run(), driver);
        assert_eq!(exit, RuntimeExit::Shutdown);
    }

    /// S4 quit path (LOW-5): one metadata-only line per write in flight at
    /// quit, open review, queued leave or unsent composition.
    mod quit_summary {
        use super::*;

        async fn wait_writes(shared: &Shared, count: usize) {
            for _ in 0..2000 {
                if shared.writes.load(Ordering::SeqCst) == count {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            panic!("write never attempted");
        }

        async fn submitted(h: &mut Harness, text: &str) {
            h.app.state.message_editor.reset_with_text(text.into());
            loop {
                if h.app.send_message_buffer().is_ok() {
                    return;
                }
                h.app.update_on_tick().await;
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }

        #[tokio::test]
        async fn a_write_confirmed_during_shutdown_is_reported_confirmed() {
            let (runtime, glue, _control, shared) = fixture(None);
            let mut h = harness(glue);
            let mut mark = None;
            let driver = async {
                room_a_open(&mut h, &shared).await;
                submitted(&mut h, "secret body").await;
                // Attempted (not merely queued), its result not drained yet.
                wait_writes(&shared, 1).await;
                mark = h.app.begin_quit();
                let line = &h.app.state.bridge.as_ref().unwrap().line;
                assert!(line.starts_with("quitting: finishing write"), "{line}");
            };
            let (exit, ()) = tokio::join!(runtime.run(), driver);
            assert_eq!(exit, RuntimeExit::Shutdown);
            h.app.quit_tick();
            let line = &h.app.state.bridge.as_ref().unwrap().line;
            assert!(line.starts_with("quitting…"), "{line}");
            assert!(!line.contains("finishing write"), "{line}");
            let lines = h.app.quit_summary(&mark.unwrap());
            assert_eq!(lines, [format!("message in \"title {ROOM_A}\": confirmed")]);
        }

        #[tokio::test]
        async fn a_write_lost_with_the_runtime_is_unknown_and_its_text_not_repeated() {
            let (runtime, glue, _control, shared) = fixture(None);
            let mut h = harness(glue);
            let mut mark = None;
            let driver = async {
                room_a_open(&mut h, &shared).await;
                shared
                    .write_replies
                    .lock()
                    .unwrap()
                    .push_back(WriteReply::Pending);
                submitted(&mut h, "secret body").await;
                for _ in 0..2000 {
                    if shared.writes.load(Ordering::SeqCst) == 1 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                mark = h.app.begin_quit();
            };
            // The join bound expires: the runtime task is dropped mid-write.
            tokio::select! {
                exit = runtime.run() => panic!("runtime exited early: {exit:?}"),
                () = driver => {}
            }
            h.app.update_on_tick().await;
            let lines = h.app.quit_summary(&mark.unwrap());
            assert_eq!(
                lines,
                [format!(
                    "message in \"title {ROOM_A}\": unknown (check the room before resending)"
                )]
            );
            assert!(lines.iter().all(|line| !line.contains("secret body")));
        }

        #[tokio::test]
        async fn open_reviews_queued_leaves_and_unsent_drafts_are_listed() {
            let (runtime, glue, _control, shared) = fixture(None);
            let mut h = harness(glue);
            let mut mark = None;
            let driver = async {
                room_a_open(&mut h, &shared).await;
                shared
                    .write_replies
                    .lock()
                    .unwrap()
                    .push_back(WriteReply::Rejected);
                submitted(&mut h, "rejected body").await;
                tick_until("rejected", &mut h, |app| {
                    app.state.bridge.as_ref().is_some_and(|b| b.review_open)
                })
                .await;
                // A newer composition and a queued leave that never ran.
                h.app
                    .state
                    .message_editor
                    .reset_with_text("newer body".into());
                h.app.pending_leaves.push_back(ROOM_B.to_owned());
                mark = h.app.begin_quit();
            };
            let (exit, ()) = tokio::join!(runtime.run(), driver);
            assert_eq!(exit, RuntimeExit::Shutdown);
            h.app.update_on_tick().await;
            let lines = h.app.quit_summary(&mark.unwrap());
            assert_eq!(
                lines,
                [
                    format!("message in \"title {ROOM_A}\": not sent"),
                    format!("leave in \"title {ROOM_B}\": not sent"),
                    format!("draft in \"title {ROOM_A}\": draft discarded"),
                ]
            );
            assert!(lines.iter().all(|line| !line.contains("body")));
        }

        #[tokio::test]
        async fn a_retained_rejected_draft_is_not_also_reported_as_discarded() {
            let (runtime, glue, _control, shared) = fixture(None);
            let mut h = harness(glue);
            let mut mark = None;
            let driver = async {
                room_a_open(&mut h, &shared).await;
                shared
                    .write_replies
                    .lock()
                    .unwrap()
                    .push_back(WriteReply::Rejected);
                submitted(&mut h, "kept").await;
                tick_until("rejected", &mut h, |app| {
                    app.state.bridge.as_ref().is_some_and(|b| b.review_open)
                })
                .await;
                mark = h.app.begin_quit();
            };
            let (exit, ()) = tokio::join!(runtime.run(), driver);
            assert_eq!(exit, RuntimeExit::Shutdown);
            let lines = h.app.quit_summary(&mark.unwrap());
            assert_eq!(lines, [format!("message in \"title {ROOM_A}\": not sent")]);
        }

        #[tokio::test]
        async fn a_delete_confirmed_during_shutdown_is_reported() {
            let (runtime, glue, _control, shared) = fixture(None);
            let mut h = harness(glue);
            let mut mark = None;
            let driver = async {
                room_a_open(&mut h, &shared).await;
                select(&mut h, "a2");
                loop {
                    if h.app.delete_selected_message().is_ok() {
                        break;
                    }
                    h.app.update_on_tick().await;
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                wait_writes(&shared, 1).await;
                mark = h.app.begin_quit();
            };
            let (exit, ()) = tokio::join!(runtime.run(), driver);
            assert_eq!(exit, RuntimeExit::Shutdown);
            h.app.update_on_tick().await;
            assert_eq!(
                h.app.quit_summary(&mark.unwrap()),
                [format!("delete in \"title {ROOM_A}\": confirmed")]
            );
        }

        #[test]
        fn nothing_to_report_without_a_bridge() {
            let mut app = App::new(false, crate::theme::Theme::default());
            assert!(app.begin_quit().is_none());
        }
    }
}
