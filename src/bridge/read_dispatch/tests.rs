//! Fake-only read dispatch tests: no socket, daemon, Keychain or Webex I/O.
use super::*;
use crate::{
    bridge::session::{
        SessionBackend, SessionClock, SessionFailure, SessionLifecycle, SessionPhase,
        SessionReadError,
    },
    frontend::{
        continuity::{ContinuityBoundary, DaemonInstanceId, LifecycleControl, UiLifecycleFrame},
        data::{ComposedBody, MessageId, PersonId, RoomId, TeamId},
        envelopes::{FrontendResult, Operation, ReadOutcome, RequestSequence},
        requests::RoomViewRequests,
    },
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Result payloads deliberately have no Debug, so unwrap_err is unavailable.
trait Refused<E> {
    fn refused(self) -> E;
}
impl<T, E> Refused<E> for Result<T, E> {
    fn refused(self) -> E {
        match self {
            Ok(_) => panic!("expected a pre-I/O refusal"),
            Err(error) => error,
        }
    }
}

#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<String>>,
    fail: Mutex<Option<(u16, &'static str)>>,
}

impl Fake {
    fn record(&self, call: String) -> Option<ClientError> {
        self.calls.lock().unwrap().push(call);
        self.fail
            .lock()
            .unwrap()
            .map(|(status, code)| ClientError::Api {
                status: status.try_into().unwrap(),
                code: code.into(),
                message: "synthetic backend detail".into(),
            })
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

fn person() -> dto::Person {
    dto::Person {
        id: "person-a".into(),
        display_name: Some("Fixture".into()),
        emails: None,
    }
}
fn room(id: &str) -> dto::Room {
    dto::Room {
        id: id.into(),
        title: Some("Fixture room".into()),
        room_type: "group".into(),
        team_id: None,
        last_activity: None,
        is_member: true,
        is_locked: None,
    }
}
fn message(id: &str, parent: Option<&str>) -> dto::Message {
    dto::Message {
        id: id.into(),
        room_id: "room/a?opaque".into(),
        revision: 1,
        parent_id: parent.map(str::to_owned),
        sender_id: None,
        created_at: Some("2026-10-09T12:00:00Z".into()),
        updated_at: None,
        status: "active".into(),
        markdown: None,
        body: Some(dto::MessageBody {
            kind: "plainText".into(),
            content: "synthetic".into(),
        }),
        attachment_count: 0,
    }
}

impl UiReadBackend for Arc<Fake> {
    async fn me(&self) -> Reply<dto::Person> {
        self.record("me".into()).map_or(Ok(person()), Err)
    }
    async fn rooms(&self) -> Reply<Vec<dto::Room>> {
        self.record("rooms".into())
            .map_or(Ok(vec![room("room/a?opaque")]), Err)
    }
    async fn room(&self, id: &str) -> Reply<dto::Room> {
        self.record(format!("room:{id}")).map_or(Ok(room(id)), Err)
    }
    async fn team(&self, id: &str) -> Reply<dto::Team> {
        self.record(format!("team:{id}")).map_or(
            Ok(dto::Team {
                id: id.into(),
                name: None,
            }),
            Err,
        )
    }
    async fn person_read(&self, id: &str) -> Reply<dto::PersonRead> {
        self.record(format!("person:{id}")).map_or(
            Ok(dto::PersonRead {
                id: id.into(),
                state: "mirrored".into(),
                person: Some(dto::Person {
                    id: id.into(),
                    ..person()
                }),
            }),
            Err,
        )
    }
    async fn room_messages_page(
        &self,
        room: &str,
        before: Option<&str>,
        max: Option<u32>,
    ) -> Reply<dto::MessagePage> {
        self.record(format!("page:{room}:{before:?}:{max:?}"))
            .map_or(
                Ok(dto::MessagePage {
                    messages: vec![message("m1", None)],
                    next_before: None,
                    complete: true,
                    partial_reason: None,
                }),
                Err,
            )
    }
    async fn message_read(&self, id: &str) -> Reply<dto::MessageRead> {
        self.record(format!("message:{id}")).map_or(
            Ok(dto::MessageRead {
                id: id.into(),
                state: "mirrored".into(),
                message: Some(message(id, None)),
            }),
            Err,
        )
    }
    async fn thread_children(
        &self,
        parent: &str,
        before: Option<&str>,
        max: Option<u32>,
    ) -> Reply<dto::ThreadChildren> {
        self.record(format!("children:{parent}:{before:?}:{max:?}"))
            .map_or(
                Ok(dto::ThreadChildren {
                    parent: dto::ThreadParent {
                        id: parent.into(),
                        state: "missing".into(),
                        message: None,
                    },
                    children: vec![message("reply", Some(parent))],
                    next_before: None,
                    complete: false,
                    partial_reason: Some("parent_missing".into()),
                }),
                Err,
            )
    }
}

fn page(before: Option<&str>, max: u32) -> PageRequest {
    PageRequest {
        before: before.map(PageCursor::new),
        max,
    }
}
fn room_id() -> RoomId {
    RoomId::new("room/a?opaque")
}

fn reads() -> Vec<(FrontendIntent, &'static str)> {
    vec![
        (FrontendIntent::Initialize, "me"),
        (FrontendIntent::ListAllRooms, "rooms"),
        (FrontendIntent::UpdateRoom(room_id()), "room:room/a?opaque"),
        (
            FrontendIntent::UpdateTeam(TeamId::new("team/a")),
            "team:team/a",
        ),
        (
            FrontendIntent::UpdatePerson(PersonId::new("person/a")),
            "person:person/a",
        ),
        (
            FrontendIntent::ListMessagesInRoom {
                room_id: room_id(),
                page: page(Some("cursor/x"), 25),
            },
            "page:room/a?opaque:Some(\"cursor/x\"):Some(25)",
        ),
        (
            FrontendIntent::UpdateMessage {
                message_id: MessageId::new("m1"),
                room_id: room_id(),
            },
            "message:m1",
        ),
        (
            FrontendIntent::UpdateChildrenMessages {
                parent_id: MessageId::new("root"),
                room_id: room_id(),
                page: page(None, 100),
            },
            "children:root:None:Some(100)",
        ),
    ]
}

fn request(intent: FrontendIntent) -> RequestEnvelope {
    RequestSequence::default().begin(intent, None).unwrap()
}

#[tokio::test]
async fn all_eight_reads_call_exactly_one_matching_method_and_correlate() {
    let cases = reads();
    assert_eq!(cases.len(), 8);
    for (intent, call) in cases {
        let fake = Arc::new(Fake::default());
        let operation = intent.operation();
        let r = request(intent);
        let context = r.context.clone();
        let envelope = dispatch_read(&fake, r).await.unwrap();
        assert_eq!(fake.calls(), [call]);
        assert_eq!(envelope.context, context);
        assert_eq!(envelope.result.operation(), operation);
        assert!(!matches!(
            envelope.result,
            FrontendResult::UpdateRoom(ReadOutcome::Rejected(_))
                | FrontendResult::ListMessagesInRoom(ReadOutcome::Rejected(_))
                | FrontendResult::UpdateChildrenMessages(ReadOutcome::Rejected(_))
        ));
    }
}

#[tokio::test]
async fn backend_errors_become_typed_rejections_without_retry_or_detail() {
    for (intent, _) in reads() {
        let fake = Arc::new(Fake::default());
        *fake.fail.lock().unwrap() = Some((503, "synthetic"));
        let initialize = intent == FrontendIntent::Initialize;
        let envelope = dispatch_read(&fake, request(intent)).await.unwrap();
        assert_eq!(fake.calls().len(), 1, "no retry");
        let rejected = match &envelope.result {
            FrontendResult::Initialize(ReadOutcome::Rejected(e))
            | FrontendResult::UpdatePerson(ReadOutcome::Rejected(e)) => Some(e),
            FrontendResult::ListAllRooms(ReadOutcome::Rejected(e)) => Some(e),
            FrontendResult::UpdateRoom(ReadOutcome::Rejected(e)) => Some(e),
            FrontendResult::UpdateTeam(ReadOutcome::Rejected(e)) => Some(e),
            FrontendResult::ListMessagesInRoom(ReadOutcome::Rejected(e)) => Some(e),
            FrontendResult::UpdateMessage(ReadOutcome::Rejected(e)) => Some(e),
            FrontendResult::UpdateChildrenMessages(ReadOutcome::Rejected(e)) => Some(e),
            _ => None,
        };
        let error = rejected.unwrap_or_else(|| panic!("rejected (initialize={initialize})"));
        assert!(!error.message.contains("synthetic backend detail"));
    }
}

#[tokio::test]
async fn writes_mismatches_and_invalid_ids_or_pages_refuse_before_io() {
    let fake = Arc::new(Fake::default());
    let writes = [
        FrontendIntent::SendMessage {
            room_id: room_id(),
            parent_id: None,
            body: ComposedBody::PlainText("private".into()),
        },
        FrontendIntent::EditMessage {
            room_id: room_id(),
            message_id: MessageId::new("m"),
            body: ComposedBody::Markdown("private".into()),
        },
        FrontendIntent::DeleteMessage {
            room_id: room_id(),
            message_id: MessageId::new("m"),
        },
        FrontendIntent::LeaveRoom(room_id()),
    ];
    for intent in writes {
        assert_eq!(
            dispatch_read(&fake, request(intent)).await.refused(),
            ReadDispatchError::WrongIntent
        );
    }
    let mut mismatched = request(FrontendIntent::ListAllRooms);
    mismatched.context.operation = Operation::Initialize;
    assert_eq!(
        dispatch_read(&fake, mismatched).await.refused(),
        ReadDispatchError::WrongIntent
    );
    let invalid = [
        FrontendIntent::UpdateRoom(RoomId::new("")),
        FrontendIntent::UpdateTeam(TeamId::new("bad\nid")),
        FrontendIntent::UpdatePerson(PersonId::new("x".repeat(1025))),
        FrontendIntent::ListMessagesInRoom {
            room_id: room_id(),
            page: page(None, 0),
        },
        FrontendIntent::ListMessagesInRoom {
            room_id: room_id(),
            page: page(None, 101),
        },
        FrontendIntent::ListMessagesInRoom {
            room_id: room_id(),
            page: page(Some("bad\u{7}cursor"), 10),
        },
        FrontendIntent::UpdateMessage {
            message_id: MessageId::new(""),
            room_id: room_id(),
        },
        FrontendIntent::UpdateChildrenMessages {
            parent_id: MessageId::new("root"),
            room_id: RoomId::new(""),
            page: page(None, 10),
        },
    ];
    for intent in invalid {
        assert_eq!(
            dispatch_read(&fake, request(intent)).await.refused(),
            ReadDispatchError::InvalidRequest
        );
    }
    // A room-view context must name the same room as the read.
    let mut views = RoomViewRequests::default();
    views.select_room(Some("room/b".into())).unwrap();
    let stamp = views.start_read().unwrap().unwrap();
    let mut retargeted = RequestSequence::default()
        .begin(
            FrontendIntent::ListMessagesInRoom {
                room_id: RoomId::new("room/b"),
                page: page(None, 10),
            },
            Some(stamp),
        )
        .unwrap();
    if let FrontendIntent::ListMessagesInRoom { room_id: room, .. } = &mut retargeted.intent {
        *room = room_id(); // envelope fields are crate-visible; refuse drift anyway
    }
    assert_eq!(
        dispatch_read(&fake, retargeted).await.refused(),
        ReadDispatchError::InvalidRequest
    );
    assert!(fake.calls().is_empty());
}

// ---- Session-level Active-only gate --------------------------------------

#[derive(Clone)]
struct Clock(Arc<Mutex<Instant>>);
impl Clock {
    fn advance(&self, seconds: u64) {
        *self.0.lock().unwrap() += Duration::from_secs(seconds);
    }
}
impl SessionClock for Clock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

struct Backend(Arc<Fake>);
impl SessionBackend for Backend {
    type Client = Arc<Fake>;
    type Stream = ();
    type Writer = ();
    type SnapshotReader = Arc<Fake>;
    async fn issue(&mut self, _: &str) -> Result<Arc<Fake>, SessionFailure> {
        Ok(Arc::clone(&self.0))
    }
    async fn subscribe(&mut self, _: &Arc<Fake>) -> Result<(), SessionFailure> {
        Ok(())
    }
    async fn continuity(&mut self, _: &Arc<Fake>) -> Result<ContinuityBoundary, SessionFailure> {
        Ok(marker())
    }
    async fn renew(&mut self, _: &Arc<Fake>) -> Result<u64, crate::bridge::session::RenewFailure> {
        // Not exercised here: behaves like a daemon at the 12 h cap.
        Err(crate::bridge::session::RenewFailure::CapReached)
    }
    fn lease_ttl(_: &Arc<Fake>) -> u64 {
        100
    }
    fn writer(_: &Arc<Fake>) {}
    fn snapshot_reader(client: &Arc<Fake>) -> Arc<Fake> {
        Arc::clone(client)
    }
}

fn marker() -> ContinuityBoundary {
    ContinuityBoundary {
        instance_id: DaemonInstanceId::new("synthetic-instance"),
        watermark: 0,
    }
}

#[tokio::test]
async fn session_reads_only_while_active_and_proves_zero_io_otherwise() {
    let fake = Arc::new(Fake::default());
    let clock = Clock(Arc::new(Mutex::new(Instant::now())));
    let mut session = SessionLifecycle::new(Backend(Arc::clone(&fake)), clock.clone());
    let read = || request(FrontendIntent::ListAllRooms);

    assert_eq!(
        session.dispatch_read(read()).await.refused(),
        SessionReadError::NotReady
    );
    let sub = session.connect("synthetic-secret").await.unwrap();
    assert_eq!(session.phase(), SessionPhase::AwaitingReady);
    assert_eq!(
        session.dispatch_read(read()).await.refused(),
        SessionReadError::NotReady
    );
    session
        .observe(
            sub,
            UiLifecycleFrame::Control(LifecycleControl::Ready(marker())),
        )
        .unwrap();
    assert_eq!(session.phase(), SessionPhase::SnapshotRequired);
    // Snapshot preparation uses only the ticket-fenced reader, never this path.
    assert_eq!(
        session.dispatch_read(read()).await.refused(),
        SessionReadError::NotReady
    );
    assert!(fake.calls().is_empty());

    let ticket = session.begin_snapshot().await.unwrap();
    session.finish_snapshot(ticket).unwrap();
    assert_eq!(session.phase(), SessionPhase::Active);
    let envelope = session.dispatch_read(read()).await.unwrap();
    assert!(matches!(
        envelope.result,
        FrontendResult::ListAllRooms(ReadOutcome::Success(_))
    ));
    assert_eq!(fake.calls(), ["rooms"]);
    assert_eq!(
        session
            .dispatch_read(request(FrontendIntent::LeaveRoom(room_id())))
            .await
            .refused(),
        SessionReadError::Dispatch(ReadDispatchError::WrongIntent)
    );

    clock.advance(100);
    assert_eq!(
        session.dispatch_read(read()).await.refused(),
        SessionReadError::Expired
    );
    assert_eq!(session.phase(), SessionPhase::Expired);

    let sub = session.connect("synthetic-secret").await.unwrap();
    session.stream_failed(sub, SessionFailure::Transport);
    assert_eq!(
        session.dispatch_read(read()).await.refused(),
        SessionReadError::Failed
    );
    session.disconnect();
    assert_eq!(
        session.dispatch_read(read()).await.refused(),
        SessionReadError::NotReady
    );
    assert_eq!(
        fake.calls(),
        ["rooms"],
        "only the Active read performed I/O"
    );
}

#[tokio::test]
async fn session_level_auth_rejections_fail_the_session_like_a_stream_failure() {
    for (status, code, failure) in [
        (401, "auth_required", Some(SessionFailure::AuthRequired)),
        (403, "forbidden_session", Some(SessionFailure::Forbidden)),
        (403, "forbidden_scope", Some(SessionFailure::Forbidden)),
        (403, "forbidden_peer", Some(SessionFailure::Forbidden)),
        (403, "write_not_allowed", None), // resource-level, not the session
        (503, "bridge_unavailable", None),
    ] {
        let fake = Arc::new(Fake::default());
        let clock = Clock(Arc::new(Mutex::new(Instant::now())));
        let mut session = SessionLifecycle::new(Backend(Arc::clone(&fake)), clock);
        let sub = session.connect("synthetic-secret").await.unwrap();
        session
            .observe(
                sub,
                UiLifecycleFrame::Control(LifecycleControl::Ready(marker())),
            )
            .unwrap();
        let ticket = session.begin_snapshot().await.unwrap();
        session.finish_snapshot(ticket).unwrap();
        *fake.fail.lock().unwrap() = Some((status, code));
        let envelope = session
            .dispatch_read(request(FrontendIntent::ListAllRooms))
            .await
            .unwrap();
        assert!(matches!(
            envelope.result,
            FrontendResult::ListAllRooms(ReadOutcome::Rejected(_))
        ));
        match failure {
            Some(failure) => {
                assert_eq!(session.phase(), SessionPhase::Failed(failure), "{code}");
                assert_eq!(session.subscription(), None);
            }
            None => assert_eq!(session.phase(), SessionPhase::Active, "{code}"),
        }
    }
}
